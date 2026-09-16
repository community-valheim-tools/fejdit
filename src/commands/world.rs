use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use globset::{GlobBuilder, GlobMatcher};
use serde::Serialize;

use crate::clean;
use crate::commands::{human_bytes, with_thousands};
use crate::format::chunked::{ChunkMap, ChunkedDb, main_file_name};
use crate::format::db::WorldDb;
use crate::format::fwl::WorldMeta;
use crate::format::inventory::{Inventory, SingleItem};
use crate::format::versions::world;
use crate::format::world::WorldData;
use crate::format::zdo::Zdo;
use crate::hash::stable_hash;
use crate::names::NameTable;
use crate::paths::{
    Layout, OutputArgs, WorldFiles, retire_file, write_file, write_file_if_changed,
};
use crate::schema::Schema;

/// Seconds per in-game day (`EnvMan.m_dayLengthSec`).
const DAY_LENGTH_SECS: f64 = 1800.0;

/// The save number a world gets when it is first written in the chunked
/// layout (`SaveSystem.BeginSave` from a zero counter).
const FIRST_SAVE_NUMBER: u32 = 1;

#[derive(Subcommand)]
pub enum WorldCmd {
    /// Summarize a world's metadata and data files
    Info(InfoArgs),
    /// Change a world's name, and its file names to match
    Rename(RenameArgs),
    /// Find items and objects by prefab name, and items by the character who
    /// crafted them
    Find(FindArgs),
    /// Remove properties and connections that do not belong on the objects
    /// carrying them
    CleanNetobjCorruption(CleanArgs),
    /// Show which property keys and connections each prefab carries (analysis aid)
    Keys(KeysArgs),
    /// Restore the armor on armor stands that game 1.0 lost when it converted
    /// an old world, copying it from the world as it was before
    FixArmorStands(FixArmorStandsArgs),
}

#[derive(Args)]
pub struct NameArgs {
    /// Extra file(s) of prefab or key names, one per line, for resolving hashes
    #[arg(long = "names", value_name = "FILE")]
    pub name_files: Vec<PathBuf>,
}

impl NameArgs {
    fn table(&self) -> Result<NameTable> {
        let mut table = NameTable::builtin();
        for f in &self.name_files {
            table.add_file(f)?;
        }
        Ok(table)
    }
}

#[derive(Args)]
pub struct InfoArgs {
    /// Path to the world: its .fwl or .db, or (1.0) its directory or any
    /// file inside it
    pub world: PathBuf,
    /// Print as JSON
    #[arg(long)]
    pub json: bool,
    /// Skip the world data (fast: metadata only)
    #[arg(long)]
    pub meta_only: bool,
    /// How many of the most common prefabs to list
    #[arg(long, default_value_t = 10)]
    pub top: usize,
    #[command(flatten)]
    pub names: NameArgs,
}

#[derive(Args)]
pub struct RenameArgs {
    /// Path to the world: its .fwl or .db, or (1.0) its directory or any
    /// file inside it
    pub world: PathBuf,
    /// The new world name
    pub new_name: String,
    /// Keep the current file (or directory) name; only change the name
    /// stored in the metadata
    #[arg(long)]
    pub keep_file_name: bool,
    /// Also bring the world to the current save format. For a .fwl + .db
    /// world that means the chunked directory layout of game 1.0, which is
    /// what the game itself writes the next time it saves. Off by default:
    /// renaming a world should not change its format
    #[arg(long)]
    pub upgrade: bool,
    #[command(flatten)]
    pub out: OutputArgs,
}

#[derive(Args)]
pub struct FindArgs {
    /// Path to the world: its .fwl or .db, or (1.0) its directory or any
    /// file inside it
    pub world: PathBuf,
    /// Prefab name glob. Tried against every item and, separately, against
    /// every object in the world; whatever it matches is reported. So
    /// 'Sword*' finds swords wherever they sit and 'Player_tombstone' finds
    /// the tombstones themselves (case-insensitive; repeatable, and repeats
    /// widen the search)
    #[arg(short, long = "item", value_name = "GLOB")]
    pub items: Vec<String>,
    /// Crafter character name glob. Unlike --item this narrows: only items
    /// with a matching crafter are reported, so nothing uncrafted comes back
    /// and no object does (case-insensitive)
    #[arg(short, long, value_name = "GLOB")]
    pub crafter: Option<String>,
    /// Print as JSON
    #[arg(long)]
    pub json: bool,
    #[command(flatten)]
    pub names: NameArgs,
}

#[derive(Args)]
pub struct CleanArgs {
    /// Path to the world: its .fwl or .db, or (1.0) its directory or any
    /// file inside it
    pub world: PathBuf,
    /// Report what would be removed without writing anything
    #[arg(long)]
    pub dry_run: bool,
    /// Do not remove keys the game strips on load anyway
    #[arg(long)]
    pub keep_stale: bool,
    /// Do not remove keys or connections belonging to components the object's
    /// prefab lacks
    #[arg(long)]
    pub keep_foreign: bool,
    /// Never remove this key, or connection type such as Portal, whatever
    /// object it is on (repeatable)
    #[arg(long = "allow-key", value_name = "NAME")]
    pub allow_keys: Vec<String>,
    /// Break the report down by prefab and key
    #[arg(long)]
    pub detailed: bool,
    /// Print the report as JSON
    #[arg(long)]
    pub json: bool,
    /// Also bring the world to the current save format. For a .fwl + .db
    /// world that means the chunked directory layout of game 1.0, so the
    /// output becomes a directory. Off by default: repairing a world should
    /// not change which game builds can open it
    #[arg(long)]
    pub upgrade: bool,
    #[command(flatten)]
    pub names: NameArgs,
    #[command(flatten)]
    pub out: OutputArgs,
}

#[derive(Args)]
pub struct FixArmorStandsArgs {
    /// The world as it was before game 1.0 converted it: its .fwl or .db, or
    /// the bare stem (a backup the game made, or your own copy)
    pub old_world: PathBuf,
    /// The converted world, a 1.0 world directory or any file inside it
    pub new_world: PathBuf,
    /// Report what would be restored without writing anything
    #[arg(long)]
    pub dry_run: bool,
    /// List every restored slot
    #[arg(long)]
    pub detailed: bool,
    /// Print the report as JSON
    #[arg(long)]
    pub json: bool,
    #[command(flatten)]
    pub names: NameArgs,
    #[command(flatten)]
    pub out: OutputArgs,
}

#[derive(Args)]
pub struct KeysArgs {
    /// Path to the world: its .fwl or .db, or (1.0) its directory or any
    /// file inside it
    pub world: PathBuf,
    /// Only prefabs matching this glob (case-insensitive)
    #[arg(short, long, value_name = "GLOB")]
    pub prefab: Option<String>,
    /// Only prefabs carrying this property key
    #[arg(short, long, value_name = "NAME")]
    pub key: Option<String>,
    /// Skip prefabs with fewer instances than this
    #[arg(long, default_value_t = 1)]
    pub min: usize,
    /// Print as JSON
    #[arg(long)]
    pub json: bool,
    #[command(flatten)]
    pub names: NameArgs,
}

pub fn run(cmd: WorldCmd) -> Result<()> {
    match cmd {
        WorldCmd::Info(args) => info(args),
        WorldCmd::Rename(args) => rename(args),
        WorldCmd::Find(args) => find(args),
        WorldCmd::CleanNetobjCorruption(args) => clean_corruption(args),
        WorldCmd::Keys(args) => keys(args),
        WorldCmd::FixArmorStands(args) => fix_armor_stands(args),
    }
}

fn load_meta(files: &WorldFiles) -> Result<WorldMeta> {
    let bytes =
        std::fs::read(&files.fwl).with_context(|| format!("reading {}", files.fwl.display()))?;
    WorldMeta::parse(&bytes).with_context(|| format!("parsing {}", files.fwl.display()))
}

/// The world's data in whichever layout it has, with the bytes it occupies
/// on disk.
fn load_world(files: &WorldFiles) -> Result<(WorldData, u64)> {
    match files.layout {
        Layout::Legacy => {
            if !files.db.is_file() {
                bail!("world data file not found: {}", files.db.display());
            }
            let bytes = std::fs::read(&files.db)
                .with_context(|| format!("reading {}", files.db.display()))?;
            let db = WorldDb::parse(&bytes)
                .with_context(|| format!("parsing {}", files.db.display()))?;
            Ok((WorldData::Db(db), bytes.len() as u64))
        }
        Layout::Chunked => {
            let number = files.save_number.unwrap_or(FIRST_SAVE_NUMBER);
            let db = ChunkedDb::read_dir(&files.dir, number)
                .with_context(|| format!("reading world directory {}", files.dir.display()))?;
            let mut size = 0;
            for name in std::iter::once(main_file_name(number, "db2"))
                .chain(std::iter::once(main_file_name(number, "chunks")))
                .chain(db.chunks.iter().map(|c| c.info.file_name()))
            {
                size += std::fs::metadata(files.dir.join(name))
                    .map(|m| m.len())
                    .unwrap_or(0);
            }
            Ok((WorldData::Chunked(db), size))
        }
    }
}

fn glob(pattern: &str) -> Result<GlobMatcher> {
    Ok(GlobBuilder::new(pattern)
        .case_insensitive(true)
        .literal_separator(false)
        .build()
        .with_context(|| format!("invalid glob {pattern:?}"))?
        .compile_matcher())
}

/// The directory the game would list the world from: the parent of a
/// `.fwl` + `.db` pair, or the parent of a world directory.
fn worlds_folder(files: &WorldFiles) -> PathBuf {
    match files.layout {
        Layout::Legacy => files.dir.clone(),
        Layout::Chunked => files
            .dir
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf),
    }
}

// ---------------------------------------------------------------- info

#[derive(Serialize)]
struct WorldInfo {
    file_name: String,
    layout: &'static str,
    save_number: Option<u32>,
    meta: WorldMeta,
    data: Option<DataInfo>,
}

#[derive(Serialize)]
struct DataInfo {
    file_size: u64,
    version: i32,
    net_time: Option<f64>,
    day: Option<f64>,
    /// The saving session and next object id; only a `.db` stores them.
    session_id: Option<i64>,
    next_uid: Option<u32>,
    zdo_count: usize,
    persistent_zdos: usize,
    property_count: usize,
    unknown_prefab_hashes: usize,
    generated_zones: usize,
    global_keys: Vec<String>,
    locations: usize,
    locations_placed: usize,
    random_event: Option<String>,
    /// Chunk files of a chunked world, by size class.
    chunks: Option<ChunkSummary>,
    top_prefabs: Vec<(String, usize)>,
}

#[derive(Serialize)]
struct ChunkSummary {
    files: usize,
    /// How many chunk files there are of each size (0 = 8 x 8 zones, each
    /// size up doubles the side).
    by_size: BTreeMap<u8, usize>,
    /// Objects in the biggest chunk file.
    largest: usize,
    /// Objects in the portal chunk.
    portals: usize,
}

fn info(args: InfoArgs) -> Result<()> {
    let files = WorldFiles::locate(&args.world)?;
    let meta = load_meta(&files)?;
    let data_present = match files.layout {
        Layout::Legacy => files.db.is_file(),
        Layout::Chunked => files.db.is_file() && files.chunks().is_some_and(|c| c.is_file()),
    };
    let data = if args.meta_only || !data_present {
        None
    } else {
        let names = args.names.table()?;
        let (world, size) = load_world(&files)?;
        let mut counts: HashMap<i32, usize> = HashMap::new();
        let mut persistent = 0;
        let mut props = 0;
        for z in world.zdos() {
            *counts.entry(z.prefab).or_default() += 1;
            persistent += usize::from(z.persistent);
            props += z.property_count();
        }
        let unknown = counts
            .keys()
            .filter(|h| names.resolve(**h).is_none())
            .count();
        let mut top: Vec<(String, usize)> = counts
            .iter()
            .map(|(h, c)| (names.display(*h), *c))
            .collect();
        top.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        top.truncate(args.top);
        let zone = world.zone_summary();
        let session = world.session();
        let chunks = match &world {
            WorldData::Db(_) => None,
            WorldData::Chunked(c) => {
                let mut by_size = BTreeMap::new();
                for chunk in &c.chunks {
                    *by_size.entry(chunk.info.index.size).or_default() += 1;
                }
                Some(ChunkSummary {
                    files: c.chunks.len(),
                    by_size,
                    largest: c
                        .chunks
                        .iter()
                        .map(|c| c.file.zdos.len())
                        .max()
                        .unwrap_or(0),
                    portals: c
                        .chunks
                        .iter()
                        .filter(|c| c.info.index == crate::format::chunked::ChunkIndex::PORTALS)
                        .map(|c| c.file.zdos.len())
                        .sum(),
                })
            }
        };
        Some(DataInfo {
            file_size: size,
            version: world.version(),
            net_time: world.net_time(),
            day: world.net_time().map(|t| t / DAY_LENGTH_SECS),
            session_id: session.map(|s| s.0),
            next_uid: session.map(|s| s.1),
            zdo_count: world.zdo_count(),
            persistent_zdos: persistent,
            property_count: props,
            unknown_prefab_hashes: unknown,
            generated_zones: zone.generated_zones,
            global_keys: zone.global_keys,
            locations: zone.locations,
            locations_placed: zone.locations_placed,
            random_event: world
                .random_event()
                .map(|e| format!("{} at ({:.0}, {:.0})", e.name, e.position.x, e.position.z)),
            chunks,
            top_prefabs: top,
        })
    };
    let info = WorldInfo {
        file_name: files.file_name.clone(),
        layout: match files.layout {
            Layout::Legacy => "single-file",
            Layout::Chunked => "chunked",
        },
        save_number: files.save_number,
        meta,
        data,
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&info)?);
        return Ok(());
    }

    let m = &info.meta;
    println!("World: {}", m.name);
    println!("  files:         {}", files.describe());
    println!("  layout:        {}", files.layout.label());
    println!(
        "  version:       {}{}",
        m.version,
        m.world_gen_version
            .map_or(String::new(), |g| format!("  (world gen {g})"))
    );
    println!("  seed:          {}  ({})", m.seed_name, m.seed);
    println!("  uid:           {}", m.uid);
    if let Some(n) = m.needs_db {
        println!("  needs db:      {}", if n.0 { "yes" } else { "no" });
    }
    if let Some(keys) = &m.starting_global_keys
        && !keys.items.is_empty()
    {
        let list: Vec<&str> = keys.items.iter().map(|s| s.as_str()).collect();
        println!("  start keys:    {}", list.join(", "));
    }
    if let Some(history) = &m.player_history
        && !history.players.is_empty()
    {
        let list: Vec<String> = history
            .players
            .iter()
            .map(|p| {
                if p.display_name.as_str().is_empty() {
                    p.id.0.clone()
                } else {
                    format!("{} ({})", p.display_name, p.id)
                }
            })
            .collect();
        println!("  players seen:  {}", list.join(", "));
    }
    let Some(d) = &info.data else {
        if !args.meta_only {
            println!("  data file:     missing ({})", files.db.display());
        }
        return Ok(());
    };
    println!(
        "Data: {} ({})",
        files.db.display(),
        human_bytes(d.file_size)
    );
    println!("  version:       {}", d.version);
    if let (Some(t), Some(day)) = (d.net_time, d.day) {
        println!("  net time:      {t:.1} s  (day {})", day.floor() as i64);
    }
    println!(
        "  objects:       {} ZDOs, {} persistent, {} properties, {} unknown prefab hashes",
        with_thousands(d.zdo_count),
        with_thousands(d.persistent_zdos),
        with_thousands(d.property_count),
        d.unknown_prefab_hashes
    );
    if let (Some(session), Some(uid)) = (d.session_id, d.next_uid) {
        println!("  session/uid:   {session} / {uid}");
    }
    if let Some(c) = &d.chunks {
        let sizes: Vec<String> = c
            .by_size
            .iter()
            .map(|(size, n)| format!("{n} of size {size}"))
            .collect();
        println!(
            "  chunks:        {} files ({}); largest holds {} objects, {} portals",
            c.files,
            sizes.join(", "),
            with_thousands(c.largest),
            c.portals
        );
    }
    println!(
        "  zones:         {} generated",
        with_thousands(d.generated_zones)
    );
    println!(
        "  locations:     {} ({} placed)",
        with_thousands(d.locations),
        with_thousands(d.locations_placed)
    );
    println!(
        "  global keys:   {}",
        if d.global_keys.is_empty() {
            "none".to_owned()
        } else {
            d.global_keys.join(", ")
        }
    );
    println!(
        "  random event:  {}",
        d.random_event.as_deref().unwrap_or("none")
    );
    if !d.top_prefabs.is_empty() {
        println!("Most common prefabs:");
        for (name, count) in &d.top_prefabs {
            println!("  {:>10}  {name}", with_thousands(*count));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- writing a chunked world

/// What writing a world directory did.
#[derive(Default)]
struct Written {
    files: Vec<PathBuf>,
    backups: Vec<PathBuf>,
    unchanged: usize,
}

/// Write every file of a chunked world into `target.dir`, touching only the
/// ones whose bytes differ from what is there. `meta_bytes` is the `.fwl2`
/// content; the rest comes from `data`.
fn write_chunked(
    target: &WorldFiles,
    meta_bytes: &[u8],
    data: &ChunkedDb,
    backup: bool,
) -> Result<Written> {
    let number = target.save_number.unwrap_or(FIRST_SAVE_NUMBER);
    let mut written = Written::default();
    let mut files = vec![(main_file_name(number, "fwl2"), meta_bytes.to_vec())];
    files.extend(data.files(number)?);
    // The .ok marker means "this save completed", so it goes last.
    let ok = files.pop();
    for (name, bytes) in files.into_iter().chain(ok) {
        let path = target.dir.join(&name);
        let (changed, bak) = write_file_if_changed(&path, &bytes, backup)?;
        if changed {
            written.files.push(path);
        } else {
            written.unchanged += 1;
        }
        written.backups.extend(bak);
    }
    Ok(written)
}

/// Copy a chunked world's files verbatim into `target.dir`, replacing only
/// the metadata. Only the files of the current save are taken: the map and
/// the chunks it names, the `.db2` and the `.ok` marker.
fn copy_chunked(source: &WorldFiles, target: &WorldFiles, meta_bytes: &[u8]) -> Result<Written> {
    let number = source.save_number.unwrap_or(FIRST_SAVE_NUMBER);
    let map_path = source.dir.join(main_file_name(number, "chunks"));
    let map = ChunkMap::parse(
        &std::fs::read(&map_path).with_context(|| format!("reading {}", map_path.display()))?,
    )
    .with_context(|| format!("parsing {}", map_path.display()))?;
    let mut written = Written::default();
    let mut names = vec![
        main_file_name(number, "db2"),
        main_file_name(number, "chunks"),
    ];
    names.extend(map.chunks.iter().map(|c| c.file_name()));
    names.push(main_file_name(number, "ok"));
    let fwl2 = target.dir.join(main_file_name(number, "fwl2"));
    written.backups.extend(write_file(&fwl2, meta_bytes, true)?);
    written.files.push(fwl2);
    for name in names {
        let bytes = std::fs::read(source.dir.join(&name))
            .with_context(|| format!("reading {}", source.dir.join(&name).display()))?;
        let path = target.dir.join(&name);
        written.backups.extend(write_file(&path, &bytes, true)?);
        written.files.push(path);
    }
    Ok(written)
}

/// The world directory `--output PATH` names for a chunked result: `PATH`
/// itself, whose name becomes the world's directory name.
fn output_world_dir(out: &Path, save_number: u32) -> Result<WorldFiles> {
    if out.is_file() {
        bail!(
            "{} is a file, but a world in the current format is a directory; give --output a directory path",
            out.display()
        );
    }
    let name = out
        .file_name()
        .and_then(|n| n.to_str())
        .with_context(|| format!("cannot derive a world name from {}", out.display()))?;
    let parent = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    Ok(WorldFiles::chunked_under(&parent, name, save_number))
}

// ---------------------------------------------------------------- rename

fn rename(args: RenameArgs) -> Result<()> {
    args.out.validate()?;
    let new_name = args.new_name.trim();
    if new_name.is_empty() {
        bail!("the new name must not be empty");
    }
    if new_name.contains(['/', '\\']) {
        bail!("the new name is used as a file name and must not contain path separators");
    }
    let files = WorldFiles::locate(&args.world)?;
    let mut meta = load_meta(&files)?;
    let old_name = meta.name.0.clone();
    let old_version = meta.version;
    meta.name = new_name.into();
    let new_file_name = if args.keep_file_name {
        files.file_name.clone()
    } else {
        new_name.to_owned()
    };
    let backup = !args.out.no_backup;
    let in_place = args.out.in_place;
    if let Some(dir) = &args.out.output
        && dir.is_file()
    {
        bail!("--output must be a directory for worlds (more than one file is written)");
    }
    let home = args
        .out
        .output
        .clone()
        .unwrap_or_else(|| worlds_folder(&files));

    let converting = args.upgrade && files.layout == Layout::Legacy;
    if args.upgrade {
        meta.upgrade();
    }
    let fwl_bytes = meta.to_file_bytes()?;

    println!("Renamed world \"{old_name}\" to \"{new_name}\"");
    match (files.layout, converting) {
        (Layout::Legacy, false) => {
            rename_legacy(&files, &home, &new_file_name, &fwl_bytes, in_place, backup)?
        }
        (Layout::Legacy, true) => {
            let (world, _) = load_world(&files)?;
            let mut world = world;
            world.upgrade();
            let WorldData::Chunked(data) = &world else {
                unreachable!("upgrade always yields the chunked layout")
            };
            let target = WorldFiles::chunked_under(&home, &new_file_name, FIRST_SAVE_NUMBER);
            if target.dir.exists() && in_place {
                bail!(
                    "{} already exists; refusing to write the converted world over it",
                    target.dir.display()
                );
            }
            let written = write_chunked(&target, &fwl_bytes, data, backup)?;
            println!(
                "  wrote {} in the chunked layout ({} files)",
                target.dir.display(),
                written.files.len()
            );
            if in_place {
                for old in [&files.fwl, &files.db] {
                    if let Some(bak) = retire_file(old, backup)? {
                        println!("  backup: {}", bak.display());
                    }
                }
            }
        }
        (Layout::Chunked, _) => {
            let number = files.save_number.unwrap_or(FIRST_SAVE_NUMBER);
            let target = WorldFiles::chunked_under(&home, &new_file_name, number);
            let same_dir = in_place && target.dir == files.dir;
            if same_dir {
                let bak = write_file(&target.fwl, &fwl_bytes, backup)?;
                println!("  metadata: {}", target.fwl.display());
                if let Some(b) = bak {
                    println!("  backup: {}", b.display());
                }
            } else if in_place {
                if target.dir.exists() {
                    bail!(
                        "{} already exists; refusing to move the world over it",
                        target.dir.display()
                    );
                }
                std::fs::rename(&files.dir, &target.dir).with_context(|| {
                    format!("moving {} to {}", files.dir.display(), target.dir.display())
                })?;
                let bak = write_file(&target.fwl, &fwl_bytes, backup)?;
                println!(
                    "  moved {} to {}",
                    files.dir.display(),
                    target.dir.display()
                );
                if let Some(b) = bak {
                    println!("  backup: {}", b.display());
                }
            } else {
                let written = copy_chunked(&files, &target, &fwl_bytes)?;
                println!(
                    "  copied the world to {} ({} files)",
                    target.dir.display(),
                    written.files.len()
                );
                for b in written.backups {
                    println!("  backup: {}", b.display());
                }
            }
        }
    }
    if old_version != meta.version {
        println!(
            "  upgraded metadata format from version {old_version} to {}",
            meta.version
        );
    } else if meta.version < world::CURRENT {
        println!(
            "  metadata format left at version {old_version} (--upgrade to stamp {})",
            world::CURRENT
        );
    }
    Ok(())
}

/// Rename in the single-file layout: write the new `.fwl`, copy or move the
/// `.db` beside it, and retire the old metadata when renaming in place.
fn rename_legacy(
    files: &WorldFiles,
    home: &Path,
    new_file_name: &str,
    fwl_bytes: &[u8],
    in_place: bool,
    backup: bool,
) -> Result<()> {
    let target = files.sibling(home, new_file_name);
    let same_files = target.fwl == files.fwl;

    // Metadata file.
    let fwl_backup = write_file(&target.fwl, fwl_bytes, backup)?;
    // Data file: copied when writing elsewhere, moved when renaming in place.
    let mut db_note = None;
    if files.db.is_file() && target.db != files.db {
        if in_place {
            if backup {
                std::fs::copy(&files.db, &target.db).with_context(|| {
                    format!("copying {} to {}", files.db.display(), target.db.display())
                })?;
                let bak = retire_file(&files.db, true)?;
                db_note = Some(format!(
                    "moved {} to {} (backup: {})",
                    files.db.display(),
                    target.db.display(),
                    bak.map_or(String::new(), |b| b.display().to_string())
                ));
            } else {
                std::fs::rename(&files.db, &target.db).with_context(|| {
                    format!("moving {} to {}", files.db.display(), target.db.display())
                })?;
                db_note = Some(format!(
                    "moved {} to {}",
                    files.db.display(),
                    target.db.display()
                ));
            }
        } else {
            std::fs::copy(&files.db, &target.db).with_context(|| {
                format!("copying {} to {}", files.db.display(), target.db.display())
            })?;
            db_note = Some(format!(
                "copied {} to {}",
                files.db.display(),
                target.db.display()
            ));
        }
    } else if !files.db.is_file() {
        db_note = Some(format!(
            "no data file found at {}; only the metadata was written",
            files.db.display()
        ));
    }
    if in_place && !same_files {
        // The old metadata file is superseded by the new one.
        if let Some(bak) = retire_file(&files.fwl, backup)? {
            println!("  backup: {}", bak.display());
        }
    }
    println!("  metadata: {}", target.fwl.display());
    if let Some(b) = fwl_backup {
        println!("  backup: {}", b.display());
    }
    if let Some(n) = db_note {
        println!("  data: {n}");
    }
    Ok(())
}

// ---------------------------------------------------------------- find

#[derive(Serialize, Clone)]
struct Found {
    item: String,
    stack: i32,
    quality: i32,
    crafter: String,
    /// Where the row sits: `container`, `tombstone`, `stand`, `dropped`
    /// (lying loose in the world, whether a player dropped it or it spawned
    /// there), or `object` for anything else the name filter matched. The
    /// first three describe the holder when this row is a held item, and the
    /// object itself when it is not.
    location: &'static str,
    /// Prefab of the holding object. Equal to `item` when the row is the
    /// object itself rather than something it holds.
    holder: String,
    /// Tombstone owner, if any
    owner: Option<String>,
    x: f32,
    y: f32,
    z: f32,
}

struct Keys {
    items: i32,
    owner_name: i32,
    crafter_name: i32,
    item: i32,
    item_data: i32,
    stack: i32,
    quality: i32,
    durability: i32,
}

impl Keys {
    fn new() -> Self {
        Keys {
            items: stable_hash("items"),
            owner_name: stable_hash("ownerName"),
            crafter_name: stable_hash("crafterName"),
            item: stable_hash("item"),
            item_data: stable_hash("itemData"),
            stack: stable_hash("stack"),
            quality: stable_hash("quality"),
            durability: stable_hash("durability"),
        }
    }
}

/// A container's inventory, wherever its version keeps it: a base64 string
/// up to world version 39, a raw byte array from 40.
fn inventory_of(z: &Zdo, k: &Keys) -> Option<Result<Inventory>> {
    if let Some(text) = z.string(k.items) {
        return Some(Inventory::from_base64(text));
    }
    z.bytes(k.items).map(Inventory::parse)
}

/// Whether the object carries an inventory at all, decodable or not.
fn has_inventory(z: &Zdo, k: &Keys) -> bool {
    z.strings.contains(k.items) || z.byte_arrays.contains(k.items)
}

/// A prefab named by an object property: a string up to world version 39,
/// a hash from 40 (`ZDOMan.ConvertPrefabStrings`).
fn prefab_property(z: &Zdo, key: i32, names: &NameTable) -> Option<String> {
    if let Some(s) = z.string(key) {
        return (!s.is_empty()).then(|| s.to_owned());
    }
    z.int(key).filter(|h| *h != 0).map(|h| names.display(h))
}

/// Stack, quality and crafter of an item lying in the world: from its
/// `itemData` blob when it has one (world version 40 up), else from the
/// separate keys.
fn loose_item_details(z: &Zdo, k: &Keys) -> (i32, i32, String) {
    if let Some(item) = z.bytes(k.item_data).and_then(|b| SingleItem::parse(b).ok()) {
        let item = item.item;
        return (
            item.stack.map_or(1, i32::from),
            item.quality.map_or(1, i32::from),
            item.crafter.map_or(String::new(), |c| c.name.0),
        );
    }
    (
        z.int(k.stack).unwrap_or(1),
        z.int(k.quality).unwrap_or(1),
        z.string(k.crafter_name).unwrap_or("").to_owned(),
    )
}

/// What an object is, for the row `find` prints when the name filter matches
/// the object itself rather than something it holds. A tombstone is also a
/// container, so the more specific class wins.
fn object_kind(z: &Zdo, schema: &Schema, k: &Keys) -> &'static str {
    if schema.knows_prefab(z.prefab) {
        for (class, kind) in [
            ("TombStone", "tombstone"),
            ("Container", "container"),
            ("ItemStand", "stand"),
            ("ArmorStand", "stand"),
        ] {
            if schema.prefab_has_class(z.prefab, class) {
                return kind;
            }
        }
        return "object";
    }
    // A modded prefab is absent from the table, so go by the keys it carries.
    if has_inventory(z, k) {
        "container"
    } else if z.strings.contains(k.item) || z.ints.contains(k.item) {
        "stand"
    } else {
        "object"
    }
}

/// Record an item lying loose in the world, where the ZDO's own prefab names
/// the item. Shared by the two ways of recognising one.
fn push_loose_item(
    found: &mut Vec<Found>,
    z: &Zdo,
    names: &NameTable,
    k: &Keys,
    matches: &impl Fn(&str, &str) -> bool,
    (x, y, z_): (f32, f32, f32),
) {
    let item = names.display(z.prefab);
    let (stack, quality, crafter) = loose_item_details(z, k);
    if !matches(&item, &crafter) {
        return;
    }
    found.push(Found {
        item: item.clone(),
        stack,
        quality,
        crafter,
        location: "dropped",
        holder: item,
        owner: None,
        x,
        y,
        z: z_,
    });
}

fn find(args: FindArgs) -> Result<()> {
    if args.items.is_empty() && args.crafter.is_none() {
        bail!("give at least one filter: --item GLOB and/or --crafter GLOB");
    }
    let item_globs = args
        .items
        .iter()
        .map(|p| glob(p))
        .collect::<Result<Vec<_>>>()?;
    let crafter_glob = args.crafter.as_deref().map(glob).transpose()?;
    // A crafter filter only ever matches items that have a crafter, so
    // `--crafter '*'` means "anything crafted by anyone".
    let matches = |item: &str, crafter: &str| {
        (item_globs.is_empty() || item_globs.iter().any(|g| g.is_match(item)))
            && crafter_glob
                .as_ref()
                .is_none_or(|g| !crafter.is_empty() && g.is_match(crafter))
    };

    let files = WorldFiles::locate(&args.world)?;
    let mut names = args.names.table()?;
    let (world, _) = load_world(&files)?;
    let zdos: Vec<&Zdo> = world.zdos().collect();
    let k = Keys::new();
    let schema = Schema::builtin();

    // Pass 1: decode every inventory, learning item names as we go so that
    // dropped-item prefabs resolve even when absent from the built-in table.
    let mut inventories: Vec<(usize, Inventory)> = Vec::new();
    let mut undecodable = 0;
    for (idx, z) in zdos.iter().enumerate() {
        match inventory_of(z, &k) {
            Some(Ok(inv)) => {
                for it in &inv.items {
                    if let crate::format::inventory::Item::Legacy(l) = it {
                        names.learn(l.name.as_str());
                    }
                }
                inventories.push((idx, inv));
            }
            Some(Err(_)) => undecodable += 1,
            None => {}
        }
    }

    // Pass 2: match.
    let mut found: Vec<Found> = Vec::new();
    let position = |z: &Zdo| (z.position.x, z.position.y, z.position.z);
    for (idx, inv) in &inventories {
        let z = zdos[*idx];
        let holder = names.display(z.prefab);
        let owner = z
            .string(k.owner_name)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let location = if owner.is_some() || holder.contains("tombstone") {
            "tombstone"
        } else {
            "container"
        };
        for it in &inv.items {
            let name = it.name(&names);
            if name.is_empty() || !matches(&name, it.crafter_name()) {
                continue;
            }
            let (x, y, z_) = position(z);
            found.push(Found {
                item: name,
                stack: it.stack(),
                quality: it.quality(),
                crafter: it.crafter_name().to_owned(),
                location,
                holder: holder.clone(),
                owner: owner.clone(),
                x,
                y,
                z: z_,
            });
        }
    }
    for z in &zdos {
        let z = *z;
        let (x, y, z_) = position(z);
        // A free-standing item is an ItemDrop ZDO whose prefab is the item
        // itself, whether a player dropped it or the world spawned it there.
        // The prefab says so outright, so ask the schema before looking at
        // any keys: an untouched item carries none of the keys a dropped one
        // picks up, and a corrupt world puts those keys on walls and floors.
        let known_prefab = schema.knows_prefab(z.prefab);
        if known_prefab && schema.prefab_has_class(z.prefab, "ItemDrop") {
            push_loose_item(&mut found, z, &names, &k, &matches, (x, y, z_));
            continue;
        }
        // The object itself. The filter is a name filter, and a tombstone or
        // a chest has a name like anything else, so every object is a
        // candidate whether or not it holds something. Every ZDO reaches this
        // test, so borrow the name rather than building one per object; only
        // a prefab with no name has to fall back to `display`.
        let unnamed;
        let name = match names.resolve(z.prefab) {
            Some(n) => n,
            None => {
                unnamed = names.display(z.prefab);
                &unnamed
            }
        };
        if matches(name, "") {
            found.push(Found {
                item: name.to_owned(),
                stack: 1,
                quality: 1,
                crafter: String::new(),
                location: object_kind(z, &schema, &k),
                holder: name.to_owned(),
                owner: z
                    .string(k.owner_name)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned),
                x,
                y,
                z: z_,
            });
        }
        // Whatever the object holds. An inventory was already read in pass 1.
        if has_inventory(z, &k) {
            continue;
        }
        // Item stands hold one item whose prefab is stored under `item`.
        if let Some(item) = prefab_property(z, k.item, &names) {
            let (_, quality, crafter) = loose_item_details(z, &k);
            if matches(&item, &crafter) {
                found.push(Found {
                    item,
                    stack: 1,
                    quality,
                    crafter,
                    location: "stand",
                    holder: names.display(z.prefab),
                    owner: None,
                    x,
                    y,
                    z: z_,
                });
            }
            continue;
        }
        // Armor stands: one slot per `<n>_item`, a string or (1.0) a hash.
        let mut on_stand = false;
        let slot_keys: Vec<(i32, String)> = z
            .strings
            .entries
            .iter()
            .map(|(key, _)| *key)
            .chain(z.ints.entries.iter().map(|(key, _)| *key))
            .filter_map(|key| {
                let name = names.resolve(key)?;
                let slot = name.strip_suffix("_item")?;
                slot.parse::<u32>().ok()?;
                Some((key, slot.to_owned()))
            })
            .collect();
        for (key, slot) in slot_keys {
            let Some(item) = prefab_property(z, key, &names) else {
                continue;
            };
            on_stand = true;
            let slot_item = z
                .bytes(stable_hash(&format!("{slot}_itemData")))
                .and_then(|b| SingleItem::parse(b).ok())
                .map(|s| s.item);
            let crafter = match &slot_item {
                Some(item) => item
                    .crafter
                    .as_ref()
                    .map_or(String::new(), |c| c.name.0.clone()),
                None => z
                    .string(stable_hash(&format!("{slot}_crafterName")))
                    .unwrap_or("")
                    .to_owned(),
            };
            let quality = match &slot_item {
                Some(item) => item.quality.map_or(1, i32::from),
                None => z.int(stable_hash(&format!("{slot}_quality"))).unwrap_or(1),
            };
            if matches(&item, &crafter) {
                found.push(Found {
                    item,
                    stack: 1,
                    quality,
                    crafter,
                    location: "stand",
                    holder: names.display(z.prefab),
                    owner: None,
                    x,
                    y,
                    z: z_,
                });
            }
        }
        if on_stand {
            continue;
        }
        // Modded prefabs are absent from the schema, so fall back to the
        // keys an item picks up once a player has handled it. This is the
        // best guess available and stays off prefabs the schema does know.
        if !known_prefab
            && (z.ints.contains(k.stack)
                || z.floats.contains(k.durability)
                || z.strings.contains(k.crafter_name)
                || z.byte_arrays.contains(k.item_data))
        {
            push_loose_item(&mut found, z, &names, &k, &matches, (x, y, z_));
        }
    }
    found.sort_by(|a, b| {
        a.item
            .to_lowercase()
            .cmp(&b.item.to_lowercase())
            .then_with(|| a.crafter.cmp(&b.crafter))
            .then_with(|| a.x.total_cmp(&b.x))
    });

    if args.json {
        println!("{}", serde_json::to_string_pretty(&found)?);
        return Ok(());
    }
    if found.is_empty() {
        println!("no matches");
    }
    for f in &found {
        let q = if f.quality > 1 {
            format!(" q{}", f.quality)
        } else {
            String::new()
        };
        let by = if f.crafter.is_empty() {
            String::new()
        } else {
            format!("  by {}", f.crafter)
        };
        // A row that holds itself is the object; anything else sits in one.
        let is_object = f.holder == f.item;
        let where_ = match (f.location, is_object) {
            ("tombstone", _) => format!(
                "tombstone{}",
                f.owner
                    .as_ref()
                    .map_or(String::new(), |o| format!(" of {o}"))
            ),
            ("container", false) => format!("in {}", f.holder),
            ("stand", false) => format!("on {}", f.holder),
            ("container", true) => "container".to_owned(),
            ("stand", true) => "stand".to_owned(),
            ("object", _) => "in the world".to_owned(),
            _ => "dropped".to_owned(),
        };
        println!(
            "{:<28} x{:<4}{q:<4}{by:<24}  {where_:<32}  x={:.1} z={:.1} (y={:.1})",
            f.item, f.stack, f.x, f.z, f.y
        );
    }
    let mut holders = std::collections::HashSet::new();
    for f in &found {
        holders.insert((f.x.to_bits(), f.y.to_bits(), f.z.to_bits()));
    }
    println!(
        "{} match(es) in {} object(s); {} containers scanned",
        found.len(),
        holders.len(),
        inventories.len()
    );
    if undecodable > 0 {
        println!("warning: {undecodable} container(s) had inventories this tool could not decode");
    }
    Ok(())
}

// ---------------------------------------------------------------- clean

fn clean_corruption(args: CleanArgs) -> Result<()> {
    if !args.dry_run {
        args.out.validate()?;
    }
    let files = WorldFiles::locate(&args.world)?;
    let names = args.names.table()?;
    let (mut world, _) = load_world(&files)?;
    let old_version = world.version();

    let schema = Schema::builtin();
    let opts = clean::Options {
        stale: !args.keep_stale,
        foreign: !args.keep_foreign,
        allow_keys: args.allow_keys.clone(),
        detailed: args.detailed || args.json,
    };
    let report = clean::clean(world.zdos_mut(), &names, &schema, &opts);

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "Scanned {} objects: {} propert{} removed from {} object{}",
            with_thousands(report.zdos_scanned),
            with_thousands(report.properties_removed),
            if report.properties_removed == 1 {
                "y"
            } else {
                "ies"
            },
            with_thousands(report.zdos_touched),
            if report.zdos_touched == 1 { "" } else { "s" }
        );
        for (rule, n) in &report.by_rule {
            println!("  {:>10}  {}", with_thousands(*n), rule.label());
        }
        if report.unknown_prefab_zdos > 0 {
            println!(
                "  {:>10}  objects skipped: prefab not in the component table",
                with_thousands(report.unknown_prefab_zdos)
            );
        }
        if !report.unattributed_keys.is_empty() {
            let total: usize = report.unattributed_keys.values().sum();
            println!(
                "  {:>10}  properties kept: named key not attributed to any class ({})",
                with_thousands(total),
                report
                    .unattributed_keys
                    .keys()
                    .take(8)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        if args.detailed {
            for (prefab, keys) in &report.by_prefab_key {
                let total: usize = keys.values().sum();
                println!("  {prefab} ({total}):");
                for (key, n) in keys {
                    println!("      {:>8}  {key}", with_thousands(*n));
                }
            }
        }
    }

    if args.dry_run {
        status(args.json, "dry run: nothing written");
        return Ok(());
    }
    if report.properties_removed == 0 && !(args.upgrade && !world.is_current()) {
        status(args.json, "nothing to remove; no file written");
        return Ok(());
    }
    if args.upgrade {
        world.upgrade();
    }
    let backup = !args.out.no_backup;
    match &world {
        WorldData::Db(db) => {
            let bytes = db.to_bytes().with_context(|| {
                format!(
                    "serializing the cleaned world at data version {}",
                    db.version
                )
            })?;
            let target = clean_target(&args, &files)?;
            let bak = write_file(&target, &bytes, backup)?;
            status(args.json, format!("wrote {}", target.display()));
            if let Some(b) = bak {
                status(args.json, format!("  backup: {}", b.display()));
            }
            if args.out.output.is_some() {
                let out_fwl = Path::new(&target).with_extension("fwl");
                if !out_fwl.exists() {
                    status(
                        args.json,
                        format!(
                            "  note: the game needs a matching .fwl next to it; copy {} to {}",
                            files.fwl.display(),
                            out_fwl.display()
                        ),
                    );
                }
            }
        }
        WorldData::Chunked(data) => {
            let converted = files.layout == Layout::Legacy;
            let number = files.save_number.unwrap_or(FIRST_SAVE_NUMBER);
            let target = match &args.out.output {
                Some(out) => output_world_dir(out, number)?,
                None if converted => {
                    WorldFiles::chunked_under(&worlds_folder(&files), &files.file_name, number)
                }
                None => files.clone(),
            };
            if converted && target.dir.exists() && args.out.in_place {
                bail!(
                    "{} already exists; refusing to write the converted world over it",
                    target.dir.display()
                );
            }
            let meta_bytes = if converted {
                let mut meta = load_meta(&files)?;
                meta.upgrade();
                meta.to_file_bytes()?
            } else {
                std::fs::read(&files.fwl)
                    .with_context(|| format!("reading {}", files.fwl.display()))?
            };
            let written = write_chunked(&target, &meta_bytes, data, backup)?;
            status(
                args.json,
                format!(
                    "wrote {} ({} files changed, {} unchanged)",
                    target.dir.display(),
                    written.files.len(),
                    written.unchanged
                ),
            );
            for b in written.backups {
                status(args.json, format!("  backup: {}", b.display()));
            }
            if converted && args.out.in_place {
                for old in [&files.fwl, &files.db] {
                    if let Some(bak) = retire_file(old, backup)? {
                        status(args.json, format!("  backup: {}", bak.display()));
                    }
                }
            }
        }
    }
    if old_version != world.version() {
        status(
            args.json,
            format!(
                "  upgraded data format from version {old_version} to {}",
                world.version()
            ),
        );
    } else if !world.is_current() {
        status(
            args.json,
            format!(
                "  data format left at version {old_version} (--upgrade to convert to version {})",
                world::CURRENT
            ),
        );
    }
    Ok(())
}

/// Where a cleaned `.db` goes. Only world data is written here -- the
/// `.fwl` is left alone -- so a `.fwl` target is a mistake worth catching, and
/// a directory is a convenience worth honouring.
fn clean_target(args: &CleanArgs, files: &WorldFiles) -> Result<PathBuf> {
    let Some(out) = args.out.output.clone() else {
        return Ok(files.db.clone());
    };
    if out.is_dir() {
        return Ok(out.join(format!("{}.db", files.file_name)));
    }
    if out
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        == Some("fwl".to_owned())
    {
        bail!(
            "--output names the world data file, so it cannot be {}: \
             this command writes the .db and never the .fwl. Give it a .db \
             path, or a directory to write {}.db into",
            out.display(),
            files.file_name
        );
    }
    Ok(out)
}

/// A progress or result line. With `--json` stdout carries only the report,
/// so these go to stderr instead of corrupting it for anything parsing it.
fn status(json: bool, line: impl AsRef<str>) {
    if json {
        eprintln!("{}", line.as_ref());
    } else {
        println!("{}", line.as_ref());
    }
}

// ---------------------------------------------------------------- fix-armor-stands

fn fix_armor_stands(args: FixArmorStandsArgs) -> Result<()> {
    if !args.dry_run {
        args.out.validate()?;
    }
    let old_files = WorldFiles::locate(&args.old_world)?;
    let new_files = WorldFiles::locate(&args.new_world)?;
    if old_files.layout != Layout::Legacy {
        bail!(
            "{} is a 1.0 world directory; the first argument is the world as it was before the game converted it (a .fwl + .db pair)",
            old_files.dir.display()
        );
    }
    if new_files.layout != Layout::Chunked {
        bail!(
            "{} is a .fwl + .db pair; the second argument is the world after game 1.0 converted it (a world directory). \
             The armor is only lost by that conversion, so there is nothing to repair here",
            new_files.fwl.display()
        );
    }
    let old_meta = load_meta(&old_files)?;
    let new_meta = load_meta(&new_files)?;
    if old_meta.uid != new_meta.uid {
        bail!(
            "these are different worlds: {} has uid {} and {} has uid {}",
            old_files.fwl.display(),
            old_meta.uid,
            new_files.fwl.display(),
            new_meta.uid
        );
    }
    let names = args.names.table()?;
    let (old, _) = load_world(&old_files)?;
    let (mut new, _) = load_world(&new_files)?;
    let report =
        crate::repair::restore_armor_stands(&old, &mut new, &names, args.detailed || args.json);

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "Armor stands: {} in the old world, {} found in the new one, {} not found",
            report.old_stands, report.stands_matched, report.stands_unmatched
        );
        println!(
            "  {:>6}  slot{} restored",
            report.slots_restored,
            if report.slots_restored == 1 { "" } else { "s" }
        );
        if report.slot_items_hashed > 0 {
            println!(
                "  {:>6}  named their item with a string the stand cannot read, now hashed",
                report.slot_items_hashed
            );
        }
        if report.slots_already_restored > 0 {
            println!(
                "  {:>6}  already had item data (placed again since, or restored before)",
                report.slots_already_restored
            );
        }
        if report.slots_changed > 0 {
            println!(
                "  {:>6}  hold a different item now, or nothing",
                report.slots_changed
            );
        }
        if report.slots_without_data > 0 {
            println!(
                "  {:>6}  had no item data in the old world",
                report.slots_without_data
            );
        }
        if args.detailed {
            for r in &report.restored {
                let by = if r.crafter.is_empty() {
                    String::new()
                } else {
                    format!("  by {}", r.crafter)
                };
                let q = if r.quality > 1 {
                    format!(" q{}", r.quality)
                } else {
                    String::new()
                };
                println!(
                    "  {:<20} slot {}  {:<24}{q:<4} durability {:.2}{by}  x={:.1} z={:.1} (y={:.1})",
                    r.stand, r.slot, r.item, r.durability, r.x, r.z, r.y
                );
            }
        }
    }

    if args.dry_run {
        status(args.json, "dry run: nothing written");
        return Ok(());
    }
    if report.slots_restored == 0 && report.slot_items_hashed == 0 {
        status(args.json, "nothing to restore; no file written");
        return Ok(());
    }
    let WorldData::Chunked(data) = &new else {
        unreachable!("the new world was checked to be chunked")
    };
    let number = new_files.save_number.unwrap_or(FIRST_SAVE_NUMBER);
    let target = match &args.out.output {
        Some(out) => output_world_dir(out, number)?,
        None => new_files.clone(),
    };
    let meta_bytes = std::fs::read(&new_files.fwl)
        .with_context(|| format!("reading {}", new_files.fwl.display()))?;
    let written = write_chunked(&target, &meta_bytes, data, !args.out.no_backup)?;
    status(
        args.json,
        format!(
            "wrote {} ({} files changed, {} unchanged)",
            target.dir.display(),
            written.files.len(),
            written.unchanged
        ),
    );
    for b in written.backups {
        status(args.json, format!("  backup: {}", b.display()));
    }
    Ok(())
}

// ---------------------------------------------------------------- keys

#[derive(Serialize)]
struct PrefabKeys {
    prefab: String,
    instances: usize,
    keys: BTreeMap<String, usize>,
}

fn keys(args: KeysArgs) -> Result<()> {
    let files = WorldFiles::locate(&args.world)?;
    let names = args.names.table()?;
    let (world, _) = load_world(&files)?;
    let prefab_glob = args.prefab.as_deref().map(glob).transpose()?;
    let key_hash = args.key.as_deref().map(stable_hash);

    let mut per_prefab: HashMap<i32, (usize, BTreeMap<String, usize>)> = HashMap::new();
    for z in world.zdos() {
        if let Some(h) = key_hash
            && !z.has_key(h)
        {
            continue;
        }
        let entry = per_prefab.entry(z.prefab).or_default();
        entry.0 += 1;
        for (kind, key, _) in z.properties() {
            *entry
                .1
                .entry(format!("{}:{}", kind.label(), names.display(key)))
                .or_default() += 1;
        }
        // Connections live outside the property tables; show them so a
        // stray one is as visible as a stray key.
        if let Some(conn) = z.connection {
            *entry.1.entry(format!("conn:{}", conn.label())).or_default() += 1;
        }
    }
    let mut rows: Vec<PrefabKeys> = per_prefab
        .into_iter()
        .map(|(h, (n, keys))| PrefabKeys {
            prefab: names.display(h),
            instances: n,
            keys,
        })
        .filter(|r| r.instances >= args.min)
        .filter(|r| prefab_glob.as_ref().is_none_or(|g| g.is_match(&r.prefab)))
        .collect();
    rows.sort_by(|a, b| {
        b.instances
            .cmp(&a.instances)
            .then_with(|| a.prefab.cmp(&b.prefab))
    });

    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    for r in &rows {
        println!("{} ({} instances)", r.prefab, with_thousands(r.instances));
        let mut keys: Vec<(&String, &usize)> = r.keys.iter().collect();
        keys.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
        for (key, n) in keys {
            println!("    {:>8}  {key}", with_thousands(*n));
        }
    }
    println!("{} prefab(s)", rows.len());
    Ok(())
}
