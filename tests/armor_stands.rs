//! `world fix-armor-stands`: putting back the armor game 1.0 loses when it
//! converts an old world. No game-converted world is available, so one is
//! staged the way 1.0 leaves it: the slot keys of the old stand minus
//! `<n>_crafterName`, no `<n>_itemData`, and `<n>_item` as a hash. Skips
//! without the (git-ignored) `examples/IPv6Test` pair.

mod common;
use common::fejdit;

use std::path::{Path, PathBuf};

use fejdit::format::chunked::ChunkedDb;
use fejdit::format::db::WorldDb;
use fejdit::format::fwl::WorldMeta;
use fejdit::format::inventory::SingleItem;
use fejdit::format::migrate::GAME_HASHED_SLOTS;
use fejdit::format::primitives::{CsString, Vec2s, Vec3};
use fejdit::format::zdo::Zdo;
use fejdit::hash::stable_hash;

fn example_path(ext: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("examples/IPv6Test.{ext}"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fejdit-armor-{}-{tag}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A 0.221 armor stand at `position` with a helmet in slot 1, a chest piece
/// in slot 3 and greaves in slot 7, all crafted, plus an empty slot 0. Slot 7
/// is past the sixth, which the game's `ConvertPrefabStrings` never reaches.
fn old_stand(position: Vec3) -> Zdo {
    let mut z = Zdo {
        persistent: true,
        sector: Some(position.zone()),
        position,
        prefab: stable_hash("ArmorStand_Male"),
        ..Zdo::default()
    };
    z.ints.entries.push((stable_hash("pose"), 2));
    z.strings
        .entries
        .push((stable_hash("0_item"), CsString::from("")));
    for (slot, item, durability, quality, crafter) in [
        (1, "HelmetBronze", 300.5f32, 2, "Sigrid"),
        (3, "ArmorBronzeChest", 812.25, 3, "Bjorn"),
        (7, "ArmorBronzeLegs", 640.75, 2, "Hrafn"),
    ] {
        let k = |n: &str| stable_hash(&format!("{slot}_{n}"));
        z.strings.entries.push((k("item"), CsString::from(item)));
        z.floats.entries.push((k("durability"), durability));
        z.ints.entries.push((k("stack"), 1));
        z.ints.entries.push((k("quality"), quality));
        z.ints.entries.push((k("variant"), 0));
        z.longs.entries.push((k("crafterID"), 77));
        z.strings
            .entries
            .push((k("crafterName"), CsString::from(crafter)));
        z.ints.entries.push((k("dataCount"), 0));
        z.ints.entries.push((k("worldLevel"), 0));
        z.ints.entries.push((k("pickedUp"), 1));
    }
    z
}

/// What `ZDOMan.Load` in 1.0 makes of that stand: prefab strings hashed for
/// the first six slots only, `<n>_crafterName` stripped, everything else left
/// as it was.
fn game_converted(old: &Zdo) -> Zdo {
    let mut z = old.clone();
    z.compact();
    for slot in 0..GAME_HASHED_SLOTS {
        let item = stable_hash(&format!("{slot}_item"));
        if let Some(name) = z.string(item).map(str::to_owned) {
            z.strings.entries.retain(|(k, _)| *k != item);
            if !name.is_empty() {
                z.ints.entries.push((item, stable_hash(&name)));
            }
        }
        let crafter = stable_hash(&format!("{slot}_crafterName"));
        z.strings.entries.retain(|(k, _)| *k != crafter);
    }
    z
}

/// Stage an old world with `stands` and the new world 1.0 would have made
/// of it, after `tweak` has had its way with the converted stands.
fn stage(dir: &Path, stands: &[Zdo], tweak: impl Fn(&mut Zdo)) -> (PathBuf, PathBuf) {
    let mut db = WorldDb::parse(&std::fs::read(example_path("db")).unwrap()).unwrap();
    let mut meta = WorldMeta::parse(&std::fs::read(example_path("fwl")).unwrap()).unwrap();
    db.zdos.zdos.extend(stands.iter().cloned());
    std::fs::write(dir.join("Old.db"), db.to_bytes().unwrap()).unwrap();
    std::fs::write(dir.join("Old.fwl"), meta.to_file_bytes().unwrap()).unwrap();

    let mut chunked = ChunkedDb::from_legacy(&db);
    let stand_prefab = stable_hash("ArmorStand_Male");
    let mut i = 0;
    for z in chunked.zdos_mut().filter(|z| z.prefab == stand_prefab) {
        *z = game_converted(&stands[i]);
        tweak(z);
        i += 1;
    }
    assert_eq!(i, stands.len());
    meta.upgrade();
    let new_dir = dir.join("New");
    std::fs::create_dir_all(&new_dir).unwrap();
    std::fs::write(new_dir.join("_main.1.fwl2"), meta.to_file_bytes().unwrap()).unwrap();
    for (name, bytes) in chunked.files(1).unwrap() {
        std::fs::write(new_dir.join(name), bytes).unwrap();
    }
    (dir.join("Old.fwl"), new_dir)
}

fn slot_blob(z: &Zdo, slot: i32) -> Option<SingleItem> {
    z.bytes(stable_hash(&format!("{slot}_itemData")))
        .map(|b| SingleItem::parse(b).unwrap())
}

/// The converted stand shows the armor but has no data for it; the repair
/// puts the blob back from the old world, crafter and all, and clears the
/// leftover keys the way the game's own conversion does for loose items.
#[test]
fn restores_slots_from_the_old_world() {
    if !example_path("db").is_file() {
        return;
    }
    let dir = scratch("restore");
    let stand = old_stand(Vec3 {
        x: 12.5,
        y: 31.0,
        z: -40.25,
    });
    let (old, new_dir) = stage(&dir, &[stand], |_| {});

    // Sanity: staged the way the game leaves it.
    let before = ChunkedDb::read_dir(&new_dir, 1).unwrap();
    let staged = before
        .zdos()
        .find(|z| z.prefab == stable_hash("ArmorStand_Male"))
        .unwrap();
    assert_eq!(slot_blob(staged, 1), None);
    assert_eq!(staged.string(stable_hash("1_crafterName")), None);
    assert_eq!(
        staged.int(stable_hash("1_item")),
        Some(stable_hash("HelmetBronze"))
    );
    assert_eq!(staged.int(stable_hash("7_item")), None);
    assert_eq!(
        staged.string(stable_hash("7_item")),
        Some("ArmorBronzeLegs"),
        "the game hashes no further than 5_item"
    );

    let (ok, stdout, stderr) = fejdit(&[
        "world".as_ref(),
        "fix-armor-stands".as_ref(),
        old.as_os_str(),
        new_dir.as_os_str(),
        "--dry-run".as_ref(),
        "--detailed".as_ref(),
    ]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("3  slots restored"), "{stdout}");
    assert!(
        stdout.contains("1  named their item with a string"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Sigrid") && stdout.contains("Bjorn") && stdout.contains("Hrafn"),
        "{stdout}"
    );
    assert_eq!(
        ChunkedDb::read_dir(&new_dir, 1).unwrap(),
        before,
        "a dry run writes nothing"
    );

    let (ok, stdout, stderr) = fejdit(&[
        "world".as_ref(),
        "fix-armor-stands".as_ref(),
        old.as_os_str(),
        new_dir.as_os_str(),
        "--in-place".as_ref(),
    ]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("1 files changed"), "{stdout}");

    let after = ChunkedDb::read_dir(&new_dir, 1).unwrap();
    let fixed = after
        .zdos()
        .find(|z| z.prefab == stable_hash("ArmorStand_Male"))
        .unwrap();
    let helmet = slot_blob(fixed, 1).unwrap().item;
    assert_eq!(helmet.prefab_hash, Some(stable_hash("HelmetBronze")));
    assert_eq!(helmet.durability_hundredths, 30050);
    assert_eq!(helmet.quality, Some(2));
    assert_eq!(
        helmet.crafter.as_ref().map(|c| c.name.as_str()),
        Some("Sigrid")
    );
    assert!(helmet.picked_up);
    let chest = slot_blob(fixed, 3).unwrap().item;
    assert_eq!(chest.durability_hundredths, 81225);
    assert_eq!(chest.quality, Some(3));
    assert_eq!(
        chest.crafter.as_ref().map(|c| c.name.as_str()),
        Some("Bjorn")
    );
    assert_eq!(slot_blob(fixed, 0), None, "an empty slot gets nothing");
    assert_eq!(
        fixed.float(stable_hash("1_durability")),
        None,
        "leftover keys go"
    );
    assert_eq!(
        fixed.int(stable_hash("1_quality")),
        Some(2),
        "a non-default quality stays"
    );
    assert_eq!(
        fixed.int(stable_hash("pose")),
        Some(2),
        "the stand's own keys stay"
    );
    assert_eq!(
        fixed.int(stable_hash("1_item")),
        Some(stable_hash("HelmetBronze"))
    );
    let legs = slot_blob(fixed, 7).unwrap().item;
    assert_eq!(legs.durability_hundredths, 64075);
    assert_eq!(
        legs.crafter.as_ref().map(|c| c.name.as_str()),
        Some("Hrafn")
    );
    assert_eq!(
        fixed.int(stable_hash("7_item")),
        Some(stable_hash("ArmorBronzeLegs")),
        "a slot past the sixth gets the hash the stand reads"
    );
    assert_eq!(
        fixed.string(stable_hash("7_item")),
        None,
        "and loses the string it had"
    );

    // Running again finds nothing left to do.
    let (ok, stdout, _) = fejdit(&[
        "world".as_ref(),
        "fix-armor-stands".as_ref(),
        old.as_os_str(),
        new_dir.as_os_str(),
        "--in-place".as_ref(),
    ]);
    assert!(ok);
    assert!(
        stdout.contains("0  slots restored") && stdout.contains("nothing to restore"),
        "{stdout}"
    );
    assert!(stdout.contains("3  already had item data"), "{stdout}");
    std::fs::remove_dir_all(&dir).ok();
}

/// A slot that holds something else now, a stand that has moved, and a
/// stand whose slot was refilled after the conversion are all left alone.
#[test]
fn leaves_changed_slots_and_moved_stands_alone() {
    if !example_path("db").is_file() {
        return;
    }
    let dir = scratch("changed");
    let kept = old_stand(Vec3 {
        x: 100.0,
        y: 10.0,
        z: 100.0,
    });
    let moved = old_stand(Vec3 {
        x: 200.0,
        y: 10.0,
        z: 200.0,
    });
    let (old, new_dir) = stage(&dir, &[kept, moved], |z| {
        if z.position.x == 100.0 {
            // Slot 1 was swapped for a different helmet since; slot 3 was
            // taken down and put back, so 1.0 wrote it a blob already.
            for (k, v) in &mut z.ints.entries {
                if *k == stable_hash("1_item") {
                    *v = stable_hash("HelmetIron");
                }
            }
            for (k, v) in &mut z.strings.entries {
                if *k == stable_hash("7_item") {
                    *v = CsString::from("ArmorIronLegs");
                }
            }
            z.byte_arrays.entries.push((
                stable_hash("3_itemData"),
                fejdit::format::primitives::ByteArray {
                    data: vec![109, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                },
            ));
        } else {
            z.position.x = 250.0;
        }
    });

    let (ok, stdout, stderr) = fejdit(&[
        "world".as_ref(),
        "fix-armor-stands".as_ref(),
        old.as_os_str(),
        new_dir.as_os_str(),
        "--json".as_ref(),
        "--dry-run".as_ref(),
    ]);
    assert!(ok, "{stderr}");
    let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["old_stands"], 2);
    assert_eq!(report["stands_matched"], 1);
    assert_eq!(report["stands_unmatched"], 1);
    assert_eq!(report["slots_restored"], 0);
    assert_eq!(report["slots_changed"], 2);
    assert_eq!(
        report["slot_items_hashed"], 0,
        "a slot naming a different item keeps its string"
    );
    assert_eq!(report["slots_already_restored"], 1);
    std::fs::remove_dir_all(&dir).ok();
}

/// The two arguments are checked for layout and for being the same world.
#[test]
fn refuses_the_wrong_worlds() {
    if !example_path("db").is_file() {
        return;
    }
    let dir = scratch("refuse");
    let (old, new_dir) = stage(&dir, &[old_stand(Vec3::default())], |_| {});

    let (ok, _, stderr) = fejdit(&[
        "world".as_ref(),
        "fix-armor-stands".as_ref(),
        new_dir.as_os_str(),
        old.as_os_str(),
        "--dry-run".as_ref(),
    ]);
    assert!(!ok);
    assert!(stderr.contains("before the game converted it"), "{stderr}");

    let other = dir.join("Other");
    std::fs::create_dir_all(&other).unwrap();
    let mut meta = WorldMeta::parse(&std::fs::read(new_dir.join("_main.1.fwl2")).unwrap()).unwrap();
    meta.uid += 1;
    std::fs::write(other.join("_main.1.fwl2"), meta.to_file_bytes().unwrap()).unwrap();
    for name in std::fs::read_dir(&new_dir).unwrap() {
        let name = name.unwrap().file_name();
        if name != "_main.1.fwl2" {
            std::fs::copy(new_dir.join(&name), other.join(&name)).unwrap();
        }
    }
    let (ok, _, stderr) = fejdit(&[
        "world".as_ref(),
        "fix-armor-stands".as_ref(),
        old.as_os_str(),
        other.as_os_str(),
        "--dry-run".as_ref(),
    ]);
    assert!(!ok);
    assert!(stderr.contains("different worlds"), "{stderr}");
    let _ = Vec2s::default();
    std::fs::remove_dir_all(&dir).ok();
}
