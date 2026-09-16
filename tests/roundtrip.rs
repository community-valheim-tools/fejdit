//! Parse every save file under `examples/` (not versioned) and check that
//! re-serializing reproduces the original bytes exactly. Skips silently when
//! the directory is absent so CI without sample data still passes.

use std::path::{Path, PathBuf};

use fejdit::format::db::WorldDb;
use fejdit::format::fch::CharacterFile;
use fejdit::format::fwl::WorldMeta;
use fejdit::format::versions;

fn examples(ext: &str) -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some(ext))
        .collect();
    files.sort();
    files
}

#[test]
fn fwl_roundtrip() {
    for path in examples("fwl") {
        let bytes = std::fs::read(&path).unwrap();
        let meta = WorldMeta::parse(&bytes).unwrap_or_else(|e| panic!("{}: {e:#}", path.display()));
        let out = meta.to_file_bytes().unwrap();
        assert_eq!(out, bytes, "{} did not round-trip", path.display());
        if meta.version == versions::world::CURRENT {
            let mut upgraded = meta.clone();
            upgraded.upgrade();
            assert_eq!(
                upgraded,
                meta,
                "{}: upgrade changed a current-version file",
                path.display()
            );
        }
    }
}

#[test]
fn db_roundtrip() {
    for path in examples("db") {
        let bytes = std::fs::read(&path).unwrap();
        let db = WorldDb::parse(&bytes).unwrap_or_else(|e| panic!("{}: {e:#}", path.display()));
        let out = db.to_bytes().unwrap();
        assert_eq!(out.len(), bytes.len(), "{}: size differs", path.display());
        assert!(out == bytes, "{} did not round-trip", path.display());
        if db.version == versions::world::CURRENT {
            let mut upgraded = db.clone();
            upgraded.upgrade();
            assert!(
                upgraded == db,
                "{}: upgrade changed a current-version file",
                path.display()
            );
        }
    }
}

#[test]
fn fch_roundtrip() {
    for path in examples("fch") {
        let bytes = std::fs::read(&path).unwrap();
        let file =
            CharacterFile::parse(&bytes).unwrap_or_else(|e| panic!("{}: {e:#}", path.display()));
        assert!(file.hash_ok, "{}: stored hash mismatch", path.display());
        let out = file.to_file_bytes().unwrap();
        assert!(out == bytes, "{} did not round-trip", path.display());
        if let Some(head) = file.player_data_head() {
            head.unwrap_or_else(|e| panic!("{}: {e:#}", path.display()));
        }
    }
}
