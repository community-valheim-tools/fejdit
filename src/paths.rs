//! Locating save files and writing them back without clobbering originals.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use clap::Args;

use crate::format::chunked::main_file_name;

/// How a world is stored on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// `<name>.fwl` beside `<name>.db` (world versions up to 39).
    Legacy,
    /// `<name>/_main.<N>.fwl2`, `.db2`, `.chunks`, `.ok` and `*.chunk`
    /// (world version 40 up, game 1.0).
    Chunked,
}

impl Layout {
    pub fn label(self) -> &'static str {
        match self {
            Layout::Legacy => "single-file (.fwl + .db)",
            Layout::Chunked => "chunked directory",
        }
    }
}

/// A world's files, derived from whichever of them the user named.
#[derive(Clone, Debug)]
pub struct WorldFiles {
    pub layout: Layout,
    /// The metadata file: `<name>.fwl` or `<dir>/_main.<N>.fwl2`.
    pub fwl: PathBuf,
    /// The data file: `<name>.db` or `<dir>/_main.<N>.db2`.
    pub db: PathBuf,
    /// The directory the world lives in: the parent of the pair, or the
    /// world's own directory when chunked.
    pub dir: PathBuf,
    /// The save number `N` of a chunked world.
    pub save_number: Option<u32>,
    /// File stem or directory name the game knows the world by.
    pub file_name: String,
}

const CHUNKED_EXTENSIONS: &[&str] = &["fwl2", "db2", "chunks", "ok", "chunk"];

impl WorldFiles {
    /// Find a world from a `.fwl`/`.db`, a bare stem, a world directory, or
    /// any `_main.<N>.*` or `.chunk` file inside one. A stem that names both
    /// a `.fwl` and a directory resolves to the directory, since that is
    /// what a 1.0 game would have written most recently.
    pub fn locate(input: &Path) -> Result<Self> {
        let ext = input
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        if input.is_dir() {
            return Self::chunked(input, None);
        }
        match ext.as_deref() {
            Some("fwl") | Some("db") => Self::legacy(&input.with_extension("")),
            Some(e) if CHUNKED_EXTENSIONS.contains(&e) => {
                let dir = input
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
                let number = input
                    .file_name()
                    .and_then(|n| n.to_str())
                    .and_then(save_number_of);
                Self::chunked(&dir, number)
            }
            _ => {
                if input.with_extension("fwl").is_file() {
                    Self::legacy(input)
                } else if input.is_dir() {
                    Self::chunked(input, None)
                } else {
                    bail!(
                        "no world at {}: expected {}.fwl or a world directory",
                        input.display(),
                        input.display()
                    )
                }
            }
        }
    }

    fn legacy(stem: &Path) -> Result<Self> {
        let fwl = stem.with_extension("fwl");
        let db = stem.with_extension("db");
        let file_name = stem
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_owned)
            .with_context(|| format!("cannot derive a world name from {}", stem.display()))?;
        if !fwl.is_file() {
            bail!("world metadata file not found: {}", fwl.display());
        }
        Ok(WorldFiles {
            layout: Layout::Legacy,
            dir: stem
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .map_or_else(|| PathBuf::from("."), Path::to_path_buf),
            fwl,
            db,
            save_number: None,
            file_name,
        })
    }

    fn chunked(dir: &Path, number: Option<u32>) -> Result<Self> {
        let file_name = dir
            .canonicalize()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()))
            .with_context(|| format!("cannot derive a world name from {}", dir.display()))?;
        let number = match number {
            Some(n) => n,
            None => latest_save_number(dir)?,
        };
        let fwl = dir.join(main_file_name(number, "fwl2"));
        if !fwl.is_file() {
            bail!("world metadata file not found: {}", fwl.display());
        }
        Ok(WorldFiles {
            layout: Layout::Chunked,
            fwl,
            db: dir.join(main_file_name(number, "db2")),
            dir: dir.to_path_buf(),
            save_number: Some(number),
            file_name,
        })
    }

    /// The chunk map of a chunked world.
    pub fn chunks(&self) -> Option<PathBuf> {
        self.save_number
            .map(|n| self.dir.join(main_file_name(n, "chunks")))
    }

    /// Where the same world would live under `dir` as `file_name`, in this
    /// layout.
    pub fn sibling(&self, dir: &Path, file_name: &str) -> WorldFiles {
        match self.layout {
            Layout::Legacy => WorldFiles {
                layout: Layout::Legacy,
                fwl: dir.join(format!("{file_name}.fwl")),
                db: dir.join(format!("{file_name}.db")),
                dir: dir.to_path_buf(),
                save_number: None,
                file_name: file_name.to_owned(),
            },
            Layout::Chunked => {
                let n = self.save_number.unwrap_or(1);
                let world_dir = dir.join(file_name);
                WorldFiles {
                    layout: Layout::Chunked,
                    fwl: world_dir.join(main_file_name(n, "fwl2")),
                    db: world_dir.join(main_file_name(n, "db2")),
                    dir: world_dir,
                    save_number: Some(n),
                    file_name: file_name.to_owned(),
                }
            }
        }
    }

    /// The chunked-layout home of this world's name under `parent`:
    /// `<parent>/<file_name>/_main.<N>.*`.
    pub fn chunked_under(parent: &Path, file_name: &str, save_number: u32) -> WorldFiles {
        let world_dir = parent.join(file_name);
        WorldFiles {
            layout: Layout::Chunked,
            fwl: world_dir.join(main_file_name(save_number, "fwl2")),
            db: world_dir.join(main_file_name(save_number, "db2")),
            dir: world_dir,
            save_number: Some(save_number),
            file_name: file_name.to_owned(),
        }
    }

    /// A short description of where the world is, for reports.
    pub fn describe(&self) -> String {
        match self.layout {
            Layout::Legacy => format!("{}  (+ .db)", self.fwl.display()),
            Layout::Chunked => format!(
                "{}  (save number {})",
                self.dir.display(),
                self.save_number.unwrap_or(0)
            ),
        }
    }
}

/// `N` from `_main.N.<ext>`.
pub fn save_number_of(file_name: &str) -> Option<u32> {
    let rest = file_name.strip_prefix("_main.")?;
    let (number, _ext) = rest.split_once('.')?;
    number.parse().ok()
}

/// The save number to read from a world directory: the highest `N` whose
/// `_main.N.ok` marker exists, since the game writes that last. Without any
/// marker, the highest `N` with metadata, which is what an interrupted save
/// leaves behind.
pub fn latest_save_number(dir: &Path) -> Result<u32> {
    let mut with_meta = Vec::new();
    let mut with_ok = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let name = entry?.file_name();
        let name = name.to_string_lossy();
        let Some(n) = save_number_of(&name) else {
            continue;
        };
        if name.ends_with(".fwl2") {
            with_meta.push(n);
        } else if name.ends_with(".ok") {
            with_ok.push(n);
        }
    }
    if with_meta.is_empty() {
        bail!(
            "{} is not a world directory: it holds no _main.<N>.fwl2",
            dir.display()
        );
    }
    let complete = with_ok
        .iter()
        .copied()
        .filter(|n| with_meta.contains(n))
        .max();
    let newest = with_meta.iter().copied().max().unwrap_or(0);
    if let Some(n) = complete {
        if n != newest {
            eprintln!(
                "warning: {} has save number {newest} without its .ok marker; reading save number {n}",
                dir.display()
            );
        }
        return Ok(n);
    }
    eprintln!(
        "warning: {} has no .ok marker for any save; reading save number {newest}",
        dir.display()
    );
    Ok(newest)
}

/// Where a mutating command writes its result. Exactly one of the two must
/// be chosen so nothing is overwritten by accident.
#[derive(Args, Clone, Debug)]
pub struct OutputArgs {
    /// Write the result to this path instead of touching the input
    #[arg(short, long, value_name = "PATH", conflicts_with = "in_place")]
    pub output: Option<PathBuf>,
    /// Overwrite the input file(s), keeping a timestamped backup beside them
    #[arg(long, conflicts_with = "output")]
    pub in_place: bool,
    /// With --in-place: skip the backup
    #[arg(long, requires = "in_place")]
    pub no_backup: bool,
}

impl OutputArgs {
    pub fn validate(&self) -> Result<()> {
        if self.output.is_none() && !self.in_place {
            bail!("choose where to write the result: --output <PATH> or --in-place");
        }
        Ok(())
    }
}

/// Write `bytes` to `target`. When `target` already exists and `backup` is
/// set, the existing file is first moved to `<name>.<timestamp>.fejdit-bak`.
pub fn write_file(target: &Path, bytes: &[u8], backup: bool) -> Result<Option<PathBuf>> {
    if target.exists() && !target.is_file() {
        // Backing up means renaming the target aside, which would happily
        // carry off a whole directory and leave a file in its place.
        bail!(
            "{} exists and is not a regular file; refusing to replace it",
            target.display()
        );
    }
    if let Some(parent) = target.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let tmp = target.with_extension(format!(
        "{}.fejdit-tmp",
        target.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    let mut backup_path = None;
    if target.exists() {
        if backup {
            let bak = backup_name(target);
            fs::rename(target, &bak)
                .with_context(|| format!("backing up {} to {}", target.display(), bak.display()))?;
            backup_path = Some(bak);
        } else {
            fs::remove_file(target).with_context(|| format!("removing {}", target.display()))?;
        }
    }
    fs::rename(&tmp, target).with_context(|| format!("moving {} into place", tmp.display()))?;
    Ok(backup_path)
}

/// Write `bytes` to `target` only if it does not already hold exactly them.
/// Returns whether anything was written, and the backup made if so.
pub fn write_file_if_changed(
    target: &Path,
    bytes: &[u8],
    backup: bool,
) -> Result<(bool, Option<PathBuf>)> {
    if target.is_file() && fs::read(target).map(|old| old == bytes).unwrap_or(false) {
        return Ok((false, None));
    }
    write_file(target, bytes, backup).map(|bak| (true, bak))
}

/// Move `target` aside to `<name>.<timestamp>.fejdit-bak`, or remove it.
pub fn retire_file(target: &Path, backup: bool) -> Result<Option<PathBuf>> {
    if !target.is_file() {
        return Ok(None);
    }
    if backup {
        let bak = backup_name(target);
        fs::rename(target, &bak)
            .with_context(|| format!("backing up {} to {}", target.display(), bak.display()))?;
        Ok(Some(bak))
    } else {
        fs::remove_file(target).with_context(|| format!("removing {}", target.display()))?;
        Ok(None)
    }
}

pub fn backup_name(target: &Path) -> PathBuf {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");
    target.with_file_name(format!("{name}.{secs}.fejdit-bak"))
}
