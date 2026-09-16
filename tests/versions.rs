//! Regressions for the two places where an upgrade could change more than the
//! format: the legacy world-generator marker in the `.fwl`, and the ZDO
//! property-count encoding in the `.db`.

use binrw::BinWrite;
use binrw::io::Cursor;

use fejdit::format::chunked::ChunkedDb;
use fejdit::format::db::{RandEventData, WorldDb, ZdoSection, ZoneSystemData};
use fejdit::format::fwl::WorldMeta;
use fejdit::format::primitives::{CsString, Vec2s};
use fejdit::format::versions::{WORLD_GEN_LEGACY, world};
use fejdit::format::zdo::{PropTable, Zdo};

mod common;
use common::{corrupt_example, fejdit};

/// A `.fwl` payload older than `world::WORLD_GEN_VERSION`, so it carries no
/// `m_worldGenVersion` field at all.
fn legacy_fwl(version: i32) -> Vec<u8> {
    let mut payload = Cursor::new(Vec::new());
    version.write_le(&mut payload).unwrap();
    CsString::from("legacy").write_le(&mut payload).unwrap();
    CsString::from("seed").write_le(&mut payload).unwrap();
    1234i32.write_le(&mut payload).unwrap();
    5678i64.write_le(&mut payload).unwrap();
    let payload = payload.into_inner();
    let mut out = Cursor::new(Vec::new());
    (payload.len() as i32).write_le(&mut out).unwrap();
    std::io::Write::write_all(&mut out, &payload).unwrap();
    out.into_inner()
}

/// Upgrading a pre-26 world must leave it on the legacy generator. The game
/// reads no `m_worldGenVersion` there, keeps the zero default, and writes zero
/// back; stamping the current version would move unexplored zones onto
/// different mountain and Mistlands/swamp thresholds than the terrain already
/// generated in the same world.
#[test]
fn upgrade_keeps_legacy_world_gen_version() {
    let meta = WorldMeta::parse(&legacy_fwl(world::WORLD_GEN_VERSION - 1)).unwrap();
    assert_eq!(meta.world_gen_version, None);

    let mut upgraded = meta.clone();
    upgraded.upgrade();
    assert_eq!(upgraded.version, world::CURRENT);
    assert_eq!(upgraded.world_gen_version, Some(WORLD_GEN_LEGACY));
}

/// A `.db` holding one ZDO with `count` float properties, with every optional
/// section filled in, then pinned to `version`. The sector is given
/// explicitly because the single-file layout stores one and reading it back
/// fills the field in.
fn db_with_floats(version: i32, count: usize) -> WorldDb {
    let zdo = Zdo {
        sector: Some(Vec2s::default()),
        floats: PropTable {
            entries: (0..count as i32).map(|i| (i, i as f32)).collect(),
        },
        ..Zdo::default()
    };
    let mut db = WorldDb {
        version: world::CURRENT,
        net_time: None,
        zdos: ZdoSection {
            session_id: 1,
            next_uid: 2,
            zdos: vec![zdo],
        },
        zone_system: Some(ZoneSystemData::default()),
        rand_events: Some(RandEventData::default()),
        persistent_events: None,
    };
    db.upgrade();
    db.version = version;
    db
}

/// The count encoding follows the file's own version, both ways. Writing the
/// wide form into a pre-33 file would desynchronise the whole ZDO stream,
/// because that reader takes the first byte at face value.
#[test]
fn property_counts_follow_the_file_version() {
    let narrow = db_with_floats(world::ZDO_WIDE_COUNTS - 1, 200);
    let wide = db_with_floats(world::ZDO_WIDE_COUNTS, 200);

    let narrow_bytes = narrow.to_bytes().unwrap();
    let wide_bytes = wide.to_bytes().unwrap();
    assert_eq!(
        narrow_bytes.len() + 1,
        wide_bytes.len(),
        "the wide form should spend one more byte on the count"
    );

    assert_eq!(WorldDb::parse(&narrow_bytes).unwrap(), narrow);
    assert_eq!(WorldDb::parse(&wide_bytes).unwrap(), wide);
}

/// `upgrade` is what re-encodes the counts, by moving the version first. A
/// `.db` can only go as far as the last single-file version; the current
/// one is a different layout (`WorldData::upgrade`).
#[test]
fn upgrade_widens_property_counts() {
    let mut db = db_with_floats(world::MIN_SUPPORTED, 200);
    let before = db.to_bytes().unwrap();
    db.upgrade();
    let after = db.to_bytes().unwrap();
    assert_eq!(before.len() + 1, after.len());
    assert_eq!(
        WorldDb::parse(&after).unwrap().version,
        world::LEGACY_CURRENT
    );
}

/// A pre-33 count is a single byte, so it cannot describe 256 entries. Better
/// to fail the write than to emit something the game silently misreads.
#[test]
fn narrow_counts_reject_oversized_tables() {
    let err = format!(
        "{:#}",
        db_with_floats(world::ZDO_WIDE_COUNTS - 1, 256)
            .to_bytes()
            .unwrap_err()
    );
    assert!(err.contains("255"), "unexpected error: {err}");
    assert!(
        db_with_floats(world::ZDO_WIDE_COUNTS, 256)
            .to_bytes()
            .is_ok()
    );
}

/// The CLI keeps a repaired world on its own data version unless asked, and
/// asking means the chunked directory layout, since that is the only place
/// the current version exists. Skips when the (git-ignored) sample corrupt
/// world is absent.
#[test]
fn clean_preserves_the_data_version_unless_upgraded() {
    let Some(source) = corrupt_example() else {
        return;
    };
    let dir = std::env::temp_dir().join(format!("fejdit-versions-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let before = std::fs::read(&source).unwrap();
    let source_db = WorldDb::parse(&before).unwrap();
    assert!(
        source_db.version < world::CURRENT,
        "the sample is meant to be an old world"
    );

    let out = dir.join("cleaned.db");
    let (ok, _, stderr) = fejdit(&[
        "world".as_ref(),
        "clean-netobj-corruption".as_ref(),
        source.as_os_str(),
        "-o".as_ref(),
        out.as_os_str(),
    ]);
    assert!(ok, "{stderr}");
    let db = WorldDb::parse(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!(db.version, source_db.version);

    let out_dir = dir.join("upgraded");
    let (ok, _, stderr) = fejdit(&[
        "world".as_ref(),
        "clean-netobj-corruption".as_ref(),
        source.as_os_str(),
        "-o".as_ref(),
        out_dir.as_os_str(),
        "--upgrade".as_ref(),
    ]);
    assert!(ok, "{stderr}");
    for name in [
        "_main.1.fwl2",
        "_main.1.db2",
        "_main.1.chunks",
        "_main.1.ok",
    ] {
        assert!(out_dir.join(name).is_file(), "missing {name}");
    }
    let chunked = ChunkedDb::read_dir(&out_dir, 1).unwrap();
    assert_eq!(chunked.db2.version, world::CURRENT);
    assert_eq!(
        chunked.zdo_count(),
        source_db.zdos.zdos.len(),
        "every object is carried over"
    );
    let meta = WorldMeta::parse(&std::fs::read(out_dir.join("_main.1.fwl2")).unwrap()).unwrap();
    assert_eq!(meta.version, world::CURRENT);
    assert!(meta.is_chunked());

    assert_eq!(
        std::fs::read(&source).unwrap(),
        before,
        "the source was modified"
    );
    std::fs::remove_dir_all(&dir).ok();
}
