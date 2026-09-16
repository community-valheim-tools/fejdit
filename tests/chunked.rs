//! The chunked world layout of game 1.0: the compact ZDO record, the
//! directory of files, and the conversion the game performs the first time
//! it saves an old world. No 1.0 sample save is checked in, so the directory
//! tests start from the (git-ignored) `examples/IPv6Test` pair and skip
//! without it.

mod common;
use common::fejdit;

use std::path::{Path, PathBuf};

use binrw::io::Cursor;
use binrw::{BinRead, BinWrite};

use fejdit::format::chunked::{
    ChunkIndex, ChunkedDb, PORTAL_PREFABS, ZONES_PER_CHUNK, ZONES_PER_SIDE, sector_index,
};
use fejdit::format::db::WorldDb;
use fejdit::format::fwl::WorldMeta;
use fejdit::format::inventory::{Inventory, Item, LegacyItem, SingleItem};
use fejdit::format::migrate::migrate_zdo;
use fejdit::format::primitives::{
    Bool, CsString, StringStringEntry, StringStringMap, Vec2i, Vec2s, Vec3, read_small_rotation,
    write_small_rotation,
};
use fejdit::format::versions::{inventory, world};
use fejdit::format::zdo::Zdo;
use fejdit::hash::stable_hash;
use fejdit::paths::{Layout, WorldFiles};

fn example_path(ext: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("examples/IPv6Test.{ext}"))
}

fn load_example() -> Option<(WorldMeta, WorldDb)> {
    let db = std::fs::read(example_path("db")).ok()?;
    let fwl = std::fs::read(example_path("fwl")).ok()?;
    Some((
        WorldMeta::parse(&fwl).unwrap(),
        WorldDb::parse(&db).unwrap(),
    ))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fejdit-chunked-{}-{tag}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Convert the example and write it as world directory `name` under `dir`.
fn converted_world(dir: &Path, name: &str) -> Option<(PathBuf, ChunkedDb)> {
    let (mut meta, db) = load_example()?;
    let chunked = ChunkedDb::from_legacy(&db);
    meta.upgrade();
    let world_dir = dir.join(name);
    std::fs::create_dir_all(&world_dir).unwrap();
    std::fs::write(
        world_dir.join("_main.1.fwl2"),
        meta.to_file_bytes().unwrap(),
    )
    .unwrap();
    for (file, bytes) in chunked.files(1).unwrap() {
        std::fs::write(world_dir.join(file), bytes).unwrap();
    }
    Some((world_dir, chunked))
}

fn write_zdo(zdo: &Zdo, version: i32) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    zdo.write_le_args(&mut out, (version,)).unwrap();
    out.into_inner()
}

fn read_zdo(bytes: &[u8], version: i32) -> Zdo {
    let mut cursor = Cursor::new(bytes);
    let zdo = Zdo::read_le_args(&mut cursor, (version,)).unwrap();
    assert_eq!(cursor.position() as usize, bytes.len(), "unparsed bytes");
    zdo
}

fn v3(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3 { x, y, z }
}

// ---------------------------------------------------------------- the ZDO record

/// The same object written for both layouts: the compact record drops the
/// sector, packs the position into shorts and the rotation into two bytes,
/// and reading the old layout back fills the sector in.
#[test]
fn compact_zdo_record_is_smaller_and_round_trips() {
    let zdo = Zdo {
        persistent: true,
        position: v3(100.0, 0.0, -200.0),
        prefab: stable_hash("Beech1"),
        rotation: Some(v3(0.0, 90.0, 0.0)),
        ..Zdo::default()
    };
    let compact = write_zdo(&zdo, world::CURRENT);
    assert_eq!(
        compact.len(),
        2 + 4 + 4 + 2,
        "flags, short pair, prefab, yaw"
    );
    assert_eq!(read_zdo(&compact, world::CURRENT), zdo);

    let legacy = write_zdo(&zdo, world::LEGACY_CURRENT);
    assert_eq!(
        legacy.len(),
        2 + 4 + 12 + 4 + 12,
        "flags, sector, vec3, prefab, vec3"
    );
    let back = read_zdo(&legacy, world::LEGACY_CURRENT);
    assert_eq!(
        back.sector,
        Some(Vec2s { x: 2, y: -3 }),
        "the sector is derived"
    );
    assert_eq!(
        Zdo {
            sector: None,
            ..back
        },
        zdo
    );
}

/// Positions the short pair cannot hold stay as three floats, in both
/// layouts, and the reader tells the two apart by the flag alone.
#[test]
fn positions_outside_the_small_form_stay_full() {
    for position in [
        v3(100.5, 0.0, -200.0),
        v3(100.0, 3.0, -200.0),
        v3(40000.0, 0.0, 0.0),
        v3(1.0, -0.0, 1.0),
    ] {
        assert_eq!(position.small(), None, "{position:?}");
        let zdo = Zdo {
            position,
            ..Zdo::default()
        };
        let bytes = write_zdo(&zdo, world::CURRENT);
        assert_eq!(bytes.len(), 2 + 12 + 4, "{position:?}");
        assert_eq!(read_zdo(&bytes, world::CURRENT), zdo);
    }
    assert_eq!(
        v3(-20000.0, 0.0, 20000.0).small(),
        Some(Vec2s {
            x: -20000,
            y: 20000
        })
    );
}

/// `ZPackage.WriteSmallRotation` quantises to half degrees and drops a pitch
/// or roll within half a degree of zero or of a full turn.
#[test]
fn small_rotation_follows_the_game_rules() {
    let encode = |r: Vec3| {
        let mut out = Cursor::new(Vec::new());
        write_small_rotation(&mut out, &r).unwrap();
        out.into_inner()
    };
    let decode = |bytes: &[u8]| read_small_rotation(&mut Cursor::new(bytes)).unwrap();

    let yaw = encode(v3(0.0, 90.0, 0.0));
    assert_eq!(yaw, (0x8000u16 | 180).to_le_bytes());
    assert_eq!(decode(&yaw), v3(0.0, 90.0, 0.0));

    for negligible in [v3(0.5, 90.0, 0.0), v3(359.75, 90.0, 359.9)] {
        let bytes = encode(negligible);
        assert_eq!(bytes.len(), 2, "{negligible:?} is yaw only");
        assert_eq!(decode(&bytes), v3(0.0, 90.0, 0.0));
    }

    let full = encode(v3(1.0, 90.0, 0.0));
    assert_eq!(full.len(), 4);
    assert_eq!(decode(&full), v3(1.0, 90.0, 0.0));

    let quantised = encode(v3(12.25, 45.0, 300.0));
    assert_eq!(decode(&quantised), v3(12.0, 45.0, 300.0));
}

/// `Zdo::compact` mirrors what `ZDO.Save` would leave out.
#[test]
fn compacting_drops_the_sector_and_a_negligible_rotation() {
    let mut zdo = Zdo {
        sector: Some(Vec2s { x: 4, y: 4 }),
        rotation: Some(v3(0.25, 0.0, 0.4)),
        ..Zdo::default()
    };
    zdo.compact();
    assert_eq!(zdo.sector, None);
    assert_eq!(zdo.rotation, None);

    let mut kept = Zdo {
        rotation: Some(v3(0.0, 0.5, 0.0)),
        ..Zdo::default()
    };
    kept.compact();
    assert_eq!(kept.rotation, Some(v3(0.0, 0.5, 0.0)));

    // Half a degree of pitch is enough to keep the rotation, and not enough
    // for the encoding to carry, so the game reads back a flagged zero.
    let mut rounded = Zdo {
        rotation: Some(v3(0.5, 0.0, 0.0)),
        ..Zdo::default()
    };
    rounded.compact();
    assert_eq!(rounded.rotation, Some(v3(0.0, 0.0, 0.0)));
    let mut steps = Zdo {
        rotation: Some(v3(12.25, 45.9, 300.0)),
        ..Zdo::default()
    };
    steps.compact();
    assert_eq!(steps.rotation, Some(v3(12.0, 45.5, 300.0)));
}

#[test]
fn zones_and_sectors_match_the_game() {
    assert_eq!(v3(31.9, 0.0, -32.1).zone(), Vec2s { x: 0, y: -1 });
    assert_eq!(v3(32.0, 0.0, 32.0).zone(), Vec2s { x: 1, y: 1 });
    assert_eq!(v3(-33.0, 0.0, 0.0).zone(), Vec2s { x: -1, y: 0 });
    assert_eq!(sector_index(&v3(0.0, 0.0, 0.0)), 256 * ZONES_PER_SIDE + 256);
    assert_eq!(
        sector_index(&v3(-16400.0, 0.0, 0.0)),
        256 * ZONES_PER_SIDE,
        "the grid's edge"
    );
    assert_eq!(sector_index(&v3(-17000.0, 0.0, 0.0)), 0, "off the grid");
}

// ---------------------------------------------------------------- inventories and migration

fn legacy_inventory() -> Inventory {
    Inventory {
        version: inventory::PICKED_UP,
        items: vec![
            Item::Legacy(LegacyItem {
                name: CsString::from("SwordIron"),
                stack: 1,
                durability: 33.333,
                grid_pos: Vec2i { x: 3, y: 1 },
                equipped: Bool(true),
                quality: Some(2),
                variant: Some(0),
                crafter: Some(fejdit::format::inventory::Crafter {
                    id: 5,
                    name: CsString::from("Bob"),
                }),
                custom_data: Some(StringStringMap {
                    entries: vec![StringStringEntry {
                        key: CsString::from("k"),
                        value: CsString::from("v"),
                    }],
                }),
                world_level: Some(0),
                picked_up: Some(Bool(true)),
                cheated: None,
            }),
            Item::Legacy(LegacyItem {
                name: CsString::from("Wood"),
                stack: 40,
                durability: 100.0,
                grid_pos: Vec2i { x: 0, y: 0 },
                equipped: Bool(false),
                quality: Some(1),
                variant: Some(0),
                crafter: Some(fejdit::format::inventory::Crafter {
                    id: 0,
                    name: CsString::from("ignored"),
                }),
                custom_data: Some(StringStringMap::default()),
                world_level: Some(0),
                picked_up: Some(Bool(false)),
                cheated: None,
            }),
            // No prefab: `Inventory.LoadOld` never adds it.
            Item::Legacy(LegacyItem {
                name: CsString::from(""),
                stack: 1,
                durability: 1.0,
                grid_pos: Vec2i { x: 1, y: 1 },
                equipped: Bool(false),
                quality: Some(1),
                variant: Some(0),
                crafter: Some(fejdit::format::inventory::Crafter::default()),
                custom_data: Some(StringStringMap::default()),
                world_level: Some(0),
                picked_up: Some(Bool(false)),
                cheated: None,
            }),
        ],
    }
}

/// The compact record keeps only what differs from the defaults, names the
/// prefab by hash, and reads back as it was written.
#[test]
fn inventories_convert_to_the_compact_record_and_round_trip() {
    let names = fejdit::names::NameTable::builtin();
    let legacy = legacy_inventory();
    assert_eq!(
        Inventory::parse(&legacy.to_bytes().unwrap()).unwrap(),
        legacy
    );

    let compact = legacy.to_compact();
    assert_eq!(compact.version, inventory::CURRENT);
    assert_eq!(compact.items.len(), 2, "the nameless item is dropped");
    let Item::Compact(sword) = &compact.items[0] else {
        panic!("compact record expected")
    };
    assert_eq!(sword.prefab_hash, Some(stable_hash("SwordIron")));
    assert_eq!(sword.durability_hundredths, 3333);
    assert_eq!(sword.quality, Some(2));
    assert_eq!(sword.stack, None);
    assert_eq!(sword.variant, None);
    assert_eq!(sword.crafter.as_ref().map(|c| c.name.as_str()), Some("Bob"));
    assert_eq!(sword.custom_data.as_ref().map(|d| d.entries.len()), Some(1));
    assert!(sword.equipped && sword.picked_up);
    assert_eq!(sword.cheated_flags, Some(0));
    let Item::Compact(wood) = &compact.items[1] else {
        panic!("compact record expected")
    };
    assert_eq!(wood.stack, Some(40));
    assert_eq!(wood.crafter, None, "a zero crafter id drops the name too");
    assert_eq!(wood.custom_data, None);

    let bytes = compact.to_bytes().unwrap();
    assert_eq!(&bytes[..4], &inventory::CURRENT.to_le_bytes());
    assert_eq!(&bytes[4..6], &2u16.to_le_bytes());
    assert_eq!(Inventory::parse(&bytes).unwrap(), compact);

    for (item, expected) in compact.items.iter().zip(["SwordIron", "Wood"]) {
        assert_eq!(item.name(&names), expected);
    }
    assert_eq!(compact.items[0].quality(), 2);
    assert_eq!(compact.items[1].stack(), 40);
    assert_eq!(compact.items[0].crafter_name(), "Bob");
}

/// `ZDOMan.ConvertInventories` and `ConvertPrefabStrings` on the three
/// kinds of object they touch.
#[test]
fn migration_rewrites_containers_items_and_prefab_strings() {
    use base64::Engine as _;
    let items = stable_hash("items");
    let mut chest = Zdo::default();
    chest.strings.entries.push((
        items,
        CsString(
            base64::engine::general_purpose::STANDARD
                .encode(legacy_inventory().to_bytes().unwrap()),
        ),
    ));
    migrate_zdo(&mut chest, world::LEGACY_CURRENT);
    assert!(!chest.strings.contains(items));
    let inv = Inventory::parse(chest.bytes(items).unwrap()).unwrap();
    assert_eq!(inv, legacy_inventory().to_compact());

    let mut loose = Zdo::default();
    loose.ints.entries.push((stable_hash("stack"), 5));
    loose.floats.entries.push((stable_hash("durability"), 50.0));
    loose.longs.entries.push((stable_hash("crafterID"), 7));
    loose
        .strings
        .entries
        .push((stable_hash("crafterName"), CsString::from("Ann")));
    loose.ints.entries.push((stable_hash("quality"), 3));
    loose.floats.entries.push((stable_hash("health"), 12.0));
    migrate_zdo(&mut loose, world::LEGACY_CURRENT);
    assert_eq!(loose.ints.get(stable_hash("stack")), None);
    assert_eq!(loose.floats.get(stable_hash("durability")), None);
    assert_eq!(loose.longs.get(stable_hash("crafterID")), None);
    assert_eq!(loose.string(stable_hash("crafterName")), None);
    assert_eq!(
        loose.int(stable_hash("quality")),
        Some(3),
        "a non-default quality stays"
    );
    assert_eq!(
        loose.float(stable_hash("health")),
        Some(12.0),
        "unrelated keys stay"
    );
    let data = SingleItem::parse(loose.bytes(stable_hash("itemData")).unwrap()).unwrap();
    assert_eq!(data.version, inventory::CURRENT as u8);
    assert_eq!(data.item.stack, Some(5));
    assert_eq!(data.item.durability_hundredths, 5000);
    assert_eq!(data.item.quality, Some(3));
    assert_eq!(
        data.item.crafter.as_ref().map(|c| c.name.as_str()),
        Some("Ann")
    );
    assert_eq!(data.item.prefab_hash, None);

    // An armor stand: slot 1 holds a helmet with the full 0.221 key set, and
    // slot 0 is empty. The game leaves these keys behind; the upgrade folds
    // them into the blob 1.0 reads.
    let mut armor = Zdo::default();
    armor
        .strings
        .entries
        .push((stable_hash("1_item"), CsString::from("HelmetBronze")));
    armor
        .floats
        .entries
        .push((stable_hash("1_durability"), 300.5));
    armor.ints.entries.push((stable_hash("1_stack"), 1));
    armor.ints.entries.push((stable_hash("1_quality"), 2));
    armor.ints.entries.push((stable_hash("1_variant"), 0));
    armor.longs.entries.push((stable_hash("1_crafterID"), 9));
    armor
        .strings
        .entries
        .push((stable_hash("1_crafterName"), CsString::from("Sigrid")));
    armor.ints.entries.push((stable_hash("1_dataCount"), 0));
    armor.ints.entries.push((stable_hash("1_worldLevel"), 0));
    armor.ints.entries.push((stable_hash("1_pickedUp"), 1));
    // Slot 8, past the `5_item` the game's ConvertPrefabStrings stops at.
    armor
        .strings
        .entries
        .push((stable_hash("8_item"), CsString::from("ArmorBronzeLegs")));
    armor
        .floats
        .entries
        .push((stable_hash("8_durability"), 100.0));
    armor.ints.entries.push((stable_hash("8_stack"), 1));
    armor.ints.entries.push((stable_hash("8_quality"), 1));
    armor.ints.entries.push((stable_hash("pose"), 3));
    migrate_zdo(&mut armor, world::LEGACY_CURRENT);
    let slot = SingleItem::parse(armor.bytes(stable_hash("1_itemData")).unwrap()).unwrap();
    assert_eq!(slot.item.prefab_hash, Some(stable_hash("HelmetBronze")));
    assert_eq!(slot.item.durability_hundredths, 30050);
    assert_eq!(slot.item.quality, Some(2));
    assert_eq!(
        slot.item.crafter.as_ref().map(|c| c.name.as_str()),
        Some("Sigrid")
    );
    assert!(slot.item.picked_up);
    assert_eq!(
        armor.int(stable_hash("1_item")),
        Some(stable_hash("HelmetBronze"))
    );
    assert_eq!(
        armor.int(stable_hash("1_quality")),
        Some(2),
        "a non-default quality stays"
    );
    assert_eq!(armor.int(stable_hash("1_variant")), None);
    assert_eq!(armor.float(stable_hash("1_durability")), None);
    assert_eq!(armor.string(stable_hash("1_crafterName")), None);
    assert_eq!(
        armor.int(stable_hash("pose")),
        Some(3),
        "the stand's own keys stay"
    );
    assert!(
        armor.bytes(stable_hash("0_itemData")).is_none(),
        "an empty slot gets no blob"
    );
    let legs = SingleItem::parse(armor.bytes(stable_hash("8_itemData")).unwrap()).unwrap();
    assert_eq!(legs.item.prefab_hash, Some(stable_hash("ArmorBronzeLegs")));
    assert_eq!(legs.item.durability_hundredths, 10000);
    assert_eq!(
        armor.int(stable_hash("8_item")),
        Some(stable_hash("ArmorBronzeLegs")),
        "a slot the game leaves as a string is hashed here"
    );
    assert_eq!(armor.string(stable_hash("8_item")), None);

    let mut stand = Zdo::default();
    stand
        .strings
        .entries
        .push((stable_hash("item"), CsString::from("Torch")));
    stand
        .strings
        .entries
        .push((stable_hash("0_item"), CsString::from("")));
    stand
        .strings
        .entries
        .push((stable_hash("1_item"), CsString::from("HelmetBronze")));
    migrate_zdo(&mut stand, world::LEGACY_CURRENT);
    assert!(stand.strings.is_empty());
    assert_eq!(stand.int(stable_hash("item")), Some(stable_hash("Torch")));
    assert_eq!(
        stand.int(stable_hash("0_item")),
        None,
        "an empty name is just removed"
    );
    assert_eq!(
        stand.int(stable_hash("1_item")),
        Some(stable_hash("HelmetBronze"))
    );
}

// ---------------------------------------------------------------- the directory

/// Converting the example deals every object into a chunk that covers its
/// zone, keeps portals apart, and reads back from disk as written.
#[test]
fn converted_world_round_trips_through_a_directory() {
    let Some((_, db)) = load_example() else {
        return;
    };
    let chunked = ChunkedDb::from_legacy(&db);
    assert_eq!(chunked.db2.version, world::CURRENT);
    assert_eq!(chunked.zdo_count(), db.zdos.zdos.len());
    assert!(
        chunked
            .chunks
            .iter()
            .all(|c| c.file.version == world::CURRENT as i16)
    );
    assert!(chunked.zdos().all(|z| z.sector.is_none()));

    let portal_hashes: Vec<i32> = PORTAL_PREFABS.iter().map(|p| stable_hash(p)).collect();
    let portals_in_source = db
        .zdos
        .zdos
        .iter()
        .filter(|z| portal_hashes.contains(&z.prefab))
        .count();
    for chunk in &chunked.chunks {
        let index = chunk.info.index;
        assert_eq!(chunk.info.num_zdos as usize, chunk.file.zdos.len());
        assert_eq!(chunk.info.version, 1);
        if index == ChunkIndex::PORTALS {
            assert_eq!(chunk.file.zdos.len(), portals_in_source);
            assert!(
                chunk
                    .file
                    .zdos
                    .iter()
                    .all(|z| portal_hashes.contains(&z.prefab))
            );
            continue;
        }
        let (zx, zy) = index.zone_origin();
        let span = (ZONES_PER_CHUNK * index.span()) as i32;
        for z in &chunk.file.zdos {
            assert!(
                !portal_hashes.contains(&z.prefab),
                "a portal outside its chunk"
            );
            let sector = sector_index(&z.position);
            if sector == 0 && index.chunk == 0 {
                continue; // off-grid objects land in chunk 0
            }
            let (sx, sy) = (
                (sector % ZONES_PER_SIDE) as i32 - 256,
                (sector / ZONES_PER_SIDE) as i32 - 256,
            );
            assert!(
                (zx..zx + span).contains(&sx) && (zy..zy + span).contains(&sy),
                "object at zone ({sx}, {sy}) filed under chunk {index:?} spanning ({zx}, {zy}) + {span}"
            );
        }
    }
    if portals_in_source > 0 {
        assert_eq!(
            chunked.chunks.last().map(|c| c.info.index),
            Some(ChunkIndex::PORTALS),
            "portals come last"
        );
    }

    // Everything the source had is still there, migrations aside.
    let mut expected: Vec<(i32, [u32; 3])> = db
        .zdos
        .zdos
        .iter()
        .map(|z| {
            (
                z.prefab,
                [
                    z.position.x.to_bits(),
                    z.position.y.to_bits(),
                    z.position.z.to_bits(),
                ],
            )
        })
        .collect();
    let mut actual: Vec<(i32, [u32; 3])> = chunked
        .zdos()
        .map(|z| {
            (
                z.prefab,
                [
                    z.position.x.to_bits(),
                    z.position.y.to_bits(),
                    z.position.z.to_bits(),
                ],
            )
        })
        .collect();
    expected.sort();
    actual.sort();
    assert_eq!(expected, actual);

    let dir = scratch("roundtrip");
    let (world_dir, _) = converted_world(&dir, "IPv6Test").unwrap();
    let back = ChunkedDb::read_dir(&world_dir, 1).unwrap();
    assert_same_world(&back, &chunked);
    assert!(world_dir.join("_main.1.ok").is_file());
    std::fs::remove_dir_all(&dir).ok();
}

/// `assert_eq!` on two worlds would print megabytes on a mismatch; narrow
/// it down first.
fn assert_same_world(actual: &ChunkedDb, expected: &ChunkedDb) {
    assert_eq!(actual.db2.version, expected.db2.version);
    assert_eq!(actual.db2.net_time, expected.db2.net_time);
    assert_eq!(actual.db2.rand_events, expected.db2.rand_events);
    assert_eq!(actual.db2.persistent_events, expected.db2.persistent_events);
    assert!(
        actual.db2.zone_system == expected.db2.zone_system,
        "zone system differs"
    );
    assert_eq!(actual.map_version, expected.map_version);
    assert_eq!(actual.chunks.len(), expected.chunks.len());
    for (a, e) in actual.chunks.iter().zip(&expected.chunks) {
        assert_eq!(a.info, e.info);
        assert_eq!(a.file.version, e.file.version);
        assert_eq!(a.file.zdos.len(), e.file.zdos.len(), "chunk {:?}", a.info);
        for (i, (za, ze)) in a.file.zdos.iter().zip(&e.file.zdos).enumerate() {
            assert!(
                za == ze,
                "chunk {:?} object {i} differs:\n{za:?}\n{ze:?}",
                a.info
            );
        }
    }
}

/// A world directory is found from the directory, from any of its files,
/// and from a bare stem when the directory exists.
#[test]
fn world_directories_are_located_from_any_entry() {
    let Some(_) = load_example() else { return };
    let dir = scratch("locate");
    let (world_dir, chunked) = converted_world(&dir, "Located").unwrap();
    let chunk_file = world_dir.join(chunked.chunks[0].info.file_name());
    for input in [
        world_dir.clone(),
        world_dir.join("_main.1.fwl2"),
        world_dir.join("_main.1.chunks"),
        chunk_file,
        dir.join("Located"),
    ] {
        let files =
            WorldFiles::locate(&input).unwrap_or_else(|e| panic!("{}: {e:#}", input.display()));
        assert_eq!(files.layout, Layout::Chunked, "{}", input.display());
        assert_eq!(files.file_name, "Located");
        assert_eq!(files.save_number, Some(1));
        assert_eq!(files.fwl, world_dir.join("_main.1.fwl2"));
    }
    // A later, unfinished save is skipped in favour of the one with its
    // .ok marker.
    std::fs::write(world_dir.join("_main.2.fwl2"), b"").unwrap();
    assert_eq!(WorldFiles::locate(&world_dir).unwrap().save_number, Some(1));
    std::fs::remove_dir_all(&dir).ok();
}

/// The commands read a world directory like a file pair, and `find` sees
/// the same items whether the world's containers hold base64 strings or
/// byte arrays.
#[test]
fn commands_read_a_world_directory() {
    let Some(_) = load_example() else { return };
    let dir = scratch("commands");
    let (world_dir, _) = converted_world(&dir, "IPv6Test").unwrap();

    let (ok, stdout, stderr) = fejdit(&["world".as_ref(), "info".as_ref(), world_dir.as_os_str()]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("chunked directory"), "{stdout}");
    assert!(stdout.contains("chunks:"), "{stdout}");

    let find = |world: &Path| {
        let (ok, stdout, stderr) = fejdit(&[
            "world".as_ref(),
            "find".as_ref(),
            "--item".as_ref(),
            "*".as_ref(),
            "--json".as_ref(),
            world.as_os_str(),
        ]);
        assert!(ok, "{stderr}");
        serde_json::from_str::<Vec<serde_json::Value>>(&stdout).unwrap()
    };
    let legacy = find(&example_path("db"));
    let converted = find(&world_dir);
    assert!(!legacy.is_empty());
    assert_eq!(legacy, converted);

    let (ok, stdout, stderr) = fejdit(&[
        "world".as_ref(),
        "keys".as_ref(),
        "--key".as_ref(),
        "items".as_ref(),
        world_dir.as_os_str(),
    ]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("bytes:items"), "{stdout}");
    assert!(!stdout.contains("string:items"), "{stdout}");
    std::fs::remove_dir_all(&dir).ok();
}

/// Cleaning in place touches only the chunk files that changed, backs
/// those up, and leaves the map and markers alone.
#[test]
fn clean_in_place_rewrites_only_the_changed_chunks() {
    let Some((mut meta, db)) = load_example() else {
        return;
    };
    let mut chunked = ChunkedDb::from_legacy(&db);
    // Corrupt one object in the second chunk.
    let victim = chunked
        .chunks
        .iter_mut()
        .filter(|c| c.info.index != ChunkIndex::PORTALS)
        .nth(1)
        .expect("a second chunk");
    let victim_name = victim.info.file_name();
    let tree = stable_hash("Beech1");
    let Some(zdo) = victim.file.zdos.iter_mut().find(|z| z.prefab == tree) else {
        // Not every chunk has a tree.
        return;
    };
    zdo.ints.entries.push((stable_hash("tamed"), 1));

    let dir = scratch("clean");
    let world_dir = dir.join("Dirty");
    std::fs::create_dir_all(&world_dir).unwrap();
    meta.upgrade();
    std::fs::write(
        world_dir.join("_main.1.fwl2"),
        meta.to_file_bytes().unwrap(),
    )
    .unwrap();
    for (file, bytes) in chunked.files(1).unwrap() {
        std::fs::write(world_dir.join(file), bytes).unwrap();
    }
    let before: Vec<(String, Vec<u8>)> = std::fs::read_dir(&world_dir)
        .unwrap()
        .map(|e| {
            let p = e.unwrap().path();
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read(&p).unwrap(),
            )
        })
        .collect();

    let (ok, stdout, stderr) = fejdit(&[
        "world".as_ref(),
        "clean-netobj-corruption".as_ref(),
        world_dir.as_os_str(),
        "--in-place".as_ref(),
    ]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("1 property removed"), "{stdout}");
    assert!(stdout.contains("1 files changed"), "{stdout}");

    let backups: Vec<PathBuf> = std::fs::read_dir(&world_dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().ends_with(".fejdit-bak"))
        .collect();
    assert_eq!(backups.len(), 1, "{backups:?}");
    assert!(backups[0].to_string_lossy().contains(&victim_name));
    for (name, bytes) in &before {
        if name == &victim_name {
            assert_ne!(&std::fs::read(world_dir.join(name)).unwrap(), bytes);
        } else {
            assert_eq!(
                &std::fs::read(world_dir.join(name)).unwrap(),
                bytes,
                "{name} changed"
            );
        }
    }
    let cleaned = ChunkedDb::read_dir(&world_dir, 1).unwrap();
    assert!(
        cleaned
            .zdos()
            .all(|z| !z.ints.contains(stable_hash("tamed")))
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// Renaming a world directory moves the directory in place, or copies the
/// current save elsewhere, and rewrites only the metadata.
#[test]
fn rename_moves_or_copies_a_world_directory() {
    let Some(_) = load_example() else { return };
    let dir = scratch("rename");
    let (world_dir, chunked) = converted_world(&dir, "Alpha").unwrap();
    let chunk_files = chunked.chunks.len();

    let (ok, stdout, stderr) = fejdit(&[
        "world".as_ref(),
        "rename".as_ref(),
        world_dir.as_os_str(),
        "Beta".as_ref(),
        "--in-place".as_ref(),
        "--no-backup".as_ref(),
    ]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("moved"), "{stdout}");
    assert!(!world_dir.exists());
    let beta = dir.join("Beta");
    let meta = WorldMeta::parse(&std::fs::read(beta.join("_main.1.fwl2")).unwrap()).unwrap();
    assert_eq!(meta.name.as_str(), "Beta");
    assert_eq!(meta.version, world::CURRENT);
    assert_same_world(&ChunkedDb::read_dir(&beta, 1).unwrap(), &chunked);

    let elsewhere = dir.join("elsewhere");
    let (ok, _, stderr) = fejdit(&[
        "world".as_ref(),
        "rename".as_ref(),
        beta.as_os_str(),
        "Gamma".as_ref(),
        "--output".as_ref(),
        elsewhere.as_os_str(),
    ]);
    assert!(ok, "{stderr}");
    assert!(beta.is_dir(), "the source stays");
    let gamma = elsewhere.join("Gamma");
    let meta = WorldMeta::parse(&std::fs::read(gamma.join("_main.1.fwl2")).unwrap()).unwrap();
    assert_eq!(meta.name.as_str(), "Gamma");
    assert_same_world(&ChunkedDb::read_dir(&gamma, 1).unwrap(), &chunked);
    let copied = std::fs::read_dir(&gamma).unwrap().count();
    assert_eq!(
        copied,
        chunk_files + 4,
        "fwl2, db2, chunks, ok and the chunk files"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// `--upgrade` on a `.fwl` + `.db` world writes the directory the game
/// would, and in place retires the old pair.
#[test]
fn rename_with_upgrade_converts_a_legacy_world() {
    let Some((_, db)) = load_example() else {
        return;
    };
    let dir = scratch("upgrade");
    std::fs::copy(example_path("fwl"), dir.join("Old.fwl")).unwrap();
    std::fs::copy(example_path("db"), dir.join("Old.db")).unwrap();

    let (ok, stdout, stderr) = fejdit(&[
        "world".as_ref(),
        "rename".as_ref(),
        dir.join("Old.fwl").as_os_str(),
        "New".as_ref(),
        "--upgrade".as_ref(),
        "--in-place".as_ref(),
    ]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("chunked layout"), "{stdout}");
    assert!(!dir.join("Old.fwl").exists() && !dir.join("Old.db").exists());
    assert!(
        std::fs::read_dir(&dir)
            .unwrap()
            .filter(|e| e
                .as_ref()
                .unwrap()
                .path()
                .to_string_lossy()
                .ends_with(".fejdit-bak"))
            .count()
            == 2,
        "both old files are backed up"
    );
    let files = WorldFiles::locate(&dir.join("New")).unwrap();
    assert_eq!(files.layout, Layout::Chunked);
    let meta = WorldMeta::parse(&std::fs::read(&files.fwl).unwrap()).unwrap();
    assert_eq!(meta.name.as_str(), "New");
    assert!(meta.is_chunked());
    let converted = ChunkedDb::read_dir(&files.dir, 1).unwrap();
    assert_eq!(converted.zdo_count(), db.zdos.zdos.len());
    std::fs::remove_dir_all(&dir).ok();
}
