//! How `world find` decides a ZDO is an item lying loose in the world.
//!
//! The prefab itself says whether it is an item, so that is what the command
//! asks. Reading the question off the keys instead gets it wrong both ways: an
//! item nobody has touched carries none of them, and a corrupt world sprays
//! them onto walls and floors.

mod common;
use common::fejdit;

use std::path::{Path, PathBuf};

use fejdit::format::db::WorldDb;
use fejdit::format::primitives::CsString;
use fejdit::hash::stable_hash;
use fejdit::schema::Schema;

fn example_path(ext: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("examples/IPv6Test.{ext}"))
}

fn load_example() -> Option<WorldDb> {
    let bytes = std::fs::read(example_path("db")).ok()?;
    Some(WorldDb::parse(&bytes).unwrap())
}

/// Write `db` to a scratch world and return the path to its `.db`. A world is
/// two files, so the example's `.fwl` comes along or the command cannot locate
/// it.
fn scratch_db(db: &WorldDb, tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fejdit-find-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("world.db");
    std::fs::write(&path, db.to_bytes().unwrap()).unwrap();
    std::fs::copy(example_path("fwl"), dir.join("world.fwl")).unwrap();
    path
}

fn find_json(world: &Path) -> Vec<serde_json::Value> {
    let (ok, stdout, stderr) = fejdit(&[
        "world".as_ref(),
        "find".as_ref(),
        "--item".as_ref(),
        "*".as_ref(),
        "--json".as_ref(),
        world.as_os_str(),
    ]);
    assert!(ok, "find failed: {stderr}");
    serde_json::from_str(&stdout).unwrap()
}

/// The component list is the authority, and it disagrees with the keys often
/// enough to matter: a fish is an item whatever it carries, a wall is not.
#[test]
fn item_prefabs_are_recognised_by_component() {
    let s = Schema::builtin();
    for item in [
        "Coins",
        "Fish1",
        "BoneFragments",
        "BlackMetalScrap",
        "SwordIron",
        // A piece you can also place, so it is both ItemDrop and Piece.
        "HoneyGlazedChicken",
    ] {
        assert!(
            s.prefab_has_class(stable_hash(item), "ItemDrop"),
            "{item} should be an item"
        );
    }
    for other in [
        "stone_wall_2x1",
        "wood_floor",
        "Beech1",
        "Lox",
        "Seagal",
        "piece_chest_wood",
        "itemstand",
        "Player_tombstone",
    ] {
        assert!(
            !s.prefab_has_class(stable_hash(other), "ItemDrop"),
            "{other} should not be an item"
        );
    }
    assert!(
        !s.prefab_has_class(stable_hash("NoSuchPrefab"), "ItemDrop"),
        "an unknown prefab is not claimed either way"
    );
}

/// An item the world spawned and nobody has picked up has no stack, no
/// durability and no crafter, which is exactly what the old key test looked
/// for. It still has to be found.
#[test]
fn untouched_items_are_found() {
    let Some(mut db) = load_example() else { return };
    let fish = stable_hash("Fish1");
    let (stack, durability, crafter) = (
        stable_hash("stack"),
        stable_hash("durability"),
        stable_hash("crafterName"),
    );

    let mut stripped = 0;
    for z in db.zdos.zdos.iter_mut().filter(|z| z.prefab == fish) {
        z.ints.entries.retain(|(k, _)| *k != stack);
        z.floats.entries.retain(|(k, _)| *k != durability);
        z.strings.entries.retain(|(k, _)| *k != crafter);
        stripped += 1;
    }
    assert!(stripped > 0, "the example has fish to strip");

    let found = find_json(&scratch_db(&db, "untouched"));
    let fish_rows = found
        .iter()
        .filter(|r| r["item"] == "Fish1" && r["location"] == "dropped")
        .count();
    assert_eq!(
        fish_rows, stripped,
        "every stripped fish is still found: {found:#?}"
    );
    // Nothing to read a stack from, so it counts as one.
    assert!(
        found
            .iter()
            .filter(|r| r["item"] == "Fish1")
            .all(|r| r["stack"] == 1)
    );
}

/// The corruption this tool exists to clean puts `crafterName` and friends on
/// building pieces. Those are not items and must not be reported as any.
#[test]
fn corrupt_building_pieces_are_not_reported_as_items() {
    let Some(mut db) = load_example() else { return };
    let wall = stable_hash("Beech1");
    let idx = db
        .zdos
        .zdos
        .iter()
        .position(|z| z.prefab == wall)
        .expect("a beech in the example");
    {
        let z = &mut db.zdos.zdos[idx];
        z.ints.entries.push((stable_hash("stack"), 42));
        z.floats.entries.push((stable_hash("durability"), 100.0));
        z.strings
            .entries
            .push((stable_hash("crafterName"), CsString::from("Ghost")));
    }

    let found = find_json(&scratch_db(&db, "corrupt"));
    assert!(
        !found
            .iter()
            .any(|r| r["item"] == "Beech1" && r["location"] == "dropped"),
        "a tree wearing item keys is still a tree: {found:#?}"
    );
    assert!(
        found
            .iter()
            .any(|r| r["item"] == "Beech1" && r["location"] == "object"),
        "it is still an object the name filter can find"
    );
    assert!(
        !found.iter().any(|r| r["crafter"] == "Ghost"),
        "and its crafter is not a crafter"
    );
}

/// `--item` is a name filter, so it has to match the object carrying items as
/// readily as the items themselves. A tombstone is not an item and never turns
/// up as one, but asking for it by name has to find it.
#[test]
fn objects_match_by_name_and_report_what_they_are() {
    let Some(db) = load_example() else { return };
    let world = scratch_db(&db, "objects");

    let (ok, stdout, stderr) = fejdit(&[
        "world".as_ref(),
        "find".as_ref(),
        "--item".as_ref(),
        "TreasureChest_meadows".as_ref(),
        "--json".as_ref(),
        world.as_os_str(),
    ]);
    assert!(ok, "find failed: {stderr}");
    let chests: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert!(
        !chests.is_empty(),
        "the example has meadows treasure chests"
    );
    for row in &chests {
        assert_eq!(row["item"], "TreasureChest_meadows");
        assert_eq!(row["location"], "container", "a chest is a container");
        assert_eq!(
            row["holder"], row["item"],
            "an object holds itself, which is how a held item is told apart"
        );
    }

    // The chest's contents are a separate question, and answering it must not
    // drag the chest itself in.
    let all = find_json(&world);
    let held = all
        .iter()
        .filter(|r| r["location"] == "container" && r["holder"] != r["item"])
        .count();
    assert!(held > 0, "the example has items in containers: {all:#?}");
}

/// Every loose item the command reports has to be a prefab the schema calls an
/// item, or one it has never heard of (a mod), never a known non-item.
#[test]
fn no_known_non_item_prefab_is_reported_loose() {
    if load_example().is_none() {
        return;
    }
    let s = Schema::builtin();
    for row in find_json(&example_path("db")) {
        if row["location"] != "dropped" {
            continue;
        }
        let name = row["item"].as_str().unwrap();
        let prefab = stable_hash(name);
        assert!(
            !s.knows_prefab(prefab) || s.prefab_has_class(prefab, "ItemDrop"),
            "{name} is a known prefab that is not an item"
        );
    }
}
