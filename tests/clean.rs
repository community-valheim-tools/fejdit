//! Inject corruption into a real world and check that only the injected
//! properties are removed. Skips when `examples/IPv6Test.db` is absent.

use std::path::Path;

use fejdit::clean::{self, Options, RuleId};
use fejdit::format::db::WorldDb;
use fejdit::format::primitives::{ByteArray, CsString};
use fejdit::format::zdo::Connection;
use fejdit::hash::{animator_hash, animator_key, stable_hash};
use fejdit::names::NameTable;
use fejdit::schema::{Schema, Verdict};

fn load_example() -> Option<WorldDb> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/IPv6Test.db");
    let bytes = std::fs::read(path).ok()?;
    Some(WorldDb::parse(&bytes).unwrap())
}

#[test]
fn injected_foreign_keys_are_removed_and_legit_keys_kept() {
    let Some(mut db) = load_example() else { return };
    let names = NameTable::builtin();
    let schema = Schema::builtin();

    // A tree with health is normal; give it a tame timer, a chest inventory,
    // an item stack, and an unknown (mod-like) key.
    let tree = stable_hash("Beech1");
    let idx = db
        .zdos
        .zdos
        .iter()
        .position(|z| z.prefab == tree)
        .expect("a beech in the example");
    let before = db.zdos.zdos[idx].property_count();
    {
        let z = &mut db.zdos.zdos[idx];
        z.floats.entries.push((stable_hash("TameTimeLeft"), 12.0));
        z.ints.entries.push((stable_hash("tamed"), 1));
        z.ints.entries.push((stable_hash("stack"), 3));
        z.strings
            .entries
            .push((stable_hash("items"), CsString::from("AAAA")));
        z.strings
            .entries
            .push((stable_hash("SomeMod.m_custom"), CsString::from("x")));
        z.ints.entries.push((-123_456_789, 7));
        z.byte_arrays
            .entries
            .push((stable_hash("health"), ByteArray { data: vec![1] }));
    }
    let opts = Options {
        stale: true,
        foreign: true,
        allow_keys: vec![],
        detailed: true,
    };
    let report = clean::clean(&mut db.zdos.zdos, &names, &schema, &opts);

    assert_eq!(report.zdos_touched, 1);
    assert_eq!(report.by_rule.get(&RuleId::Foreign), Some(&4), "{report:?}");
    assert_eq!(
        report.by_rule.get(&RuleId::Stale),
        Some(&1),
        "byte-array health is stale"
    );
    let z = &db.zdos.zdos[idx];
    assert_eq!(
        z.property_count(),
        before + 2,
        "mod key and unknown hash survive"
    );
    assert!(z.strings.contains(stable_hash("SomeMod.m_custom")));
    assert!(z.ints.contains(-123_456_789));
    assert!(!z.ints.contains(stable_hash("tamed")));
    assert!(!z.strings.contains(stable_hash("items")));
    assert_eq!(report.by_prefab_key["Beech1"]["int:tamed"], 1);
}

#[test]
fn allow_key_overrides_removal() {
    let Some(mut db) = load_example() else { return };
    let names = NameTable::builtin();
    let schema = Schema::builtin();
    let rock = stable_hash("Rock_4");
    let idx = db.zdos.zdos.iter().position(|z| z.prefab == rock).unwrap();
    db.zdos.zdos[idx]
        .ints
        .entries
        .push((stable_hash("tamed"), 1));
    let opts = Options {
        stale: true,
        foreign: true,
        allow_keys: vec!["tamed".into()],
        detailed: false,
    };
    let report = clean::clean(&mut db.zdos.zdos, &names, &schema, &opts);
    assert_eq!(report.properties_removed, 0);
}

#[test]
fn schema_verdicts_on_common_prefabs() {
    let s = Schema::builtin();
    let wall = stable_hash("stone_wall_2x1");
    assert_eq!(
        s.judge(wall, stable_hash("health"), Some("health")),
        Verdict::Owned("WearNTear".into())
    );
    assert_eq!(
        s.judge(wall, stable_hash("creator"), Some("creator")),
        Verdict::Owned("Piece".into())
    );
    assert!(matches!(
        s.judge(wall, stable_hash("items"), Some("items")),
        Verdict::Foreign(_)
    ));
    let deer = stable_hash("Deer");
    assert_eq!(
        s.judge(deer, stable_hash("alert"), Some("alert")),
        Verdict::Owned("BaseAI".into())
    );
    let zone = stable_hash("_ZoneCtrl");
    assert_eq!(
        s.judge(zone, stable_hash("b_Greydwarf21"), Some("b_Greydwarf21")),
        Verdict::Owned("SpawnSystem".into())
    );
    let stone = stable_hash("BossStone_Bonemass");
    assert_eq!(
        s.judge(stone, stable_hash("durability"), Some("durability")),
        Verdict::Owned("ItemStand".into())
    );
}

#[test]
fn foreign_connections_are_removed_and_real_ones_kept() {
    let Some(mut db) = load_example() else { return };
    let names = NameTable::builtin();
    let schema = Schema::builtin();

    // A beech has no TeleportWorld, so a portal connection cannot be its own;
    // a spawner target is unconstrained by the game, so it has to survive.
    let tree = stable_hash("Beech1");
    let trees: Vec<usize> = db
        .zdos
        .zdos
        .iter()
        .enumerate()
        .filter(|(_, z)| z.prefab == tree)
        .map(|(i, _)| i)
        .take(3)
        .collect();
    assert_eq!(trees.len(), 3, "three beeches in the example");
    db.zdos.zdos[trees[0]].connection = Some(Connection { kind: 1, hash: 11 });
    db.zdos.zdos[trees[1]].connection = Some(Connection {
        kind: 3 | Connection::TARGET,
        hash: 12,
    });
    // A type this tool does not know must be left alone, like an unnamed key.
    db.zdos.zdos[trees[2]].connection = Some(Connection { kind: 9, hash: 13 });

    let opts = Options {
        stale: true,
        foreign: true,
        allow_keys: vec![],
        detailed: true,
    };
    let report = clean::clean(&mut db.zdos.zdos, &names, &schema, &opts);

    assert_eq!(report.properties_removed, 1, "{report:?}");
    assert_eq!(report.by_rule.get(&RuleId::Foreign), Some(&1));
    assert_eq!(report.by_prefab_key["Beech1"]["conn:Portal"], 1);
    assert_eq!(db.zdos.zdos[trees[0]].connection, None);
    assert!(
        db.zdos.zdos[trees[1]].connection.is_some(),
        "Spawned|Target is unconstrained"
    );
    assert!(
        db.zdos.zdos[trees[2]].connection.is_some(),
        "unknown connection type is left alone"
    );
}

#[test]
fn allow_key_protects_a_connection_type() {
    let Some(mut db) = load_example() else { return };
    let names = NameTable::builtin();
    let schema = Schema::builtin();
    let tree = stable_hash("Beech1");
    let idx = db.zdos.zdos.iter().position(|z| z.prefab == tree).unwrap();
    db.zdos.zdos[idx].connection = Some(Connection { kind: 1, hash: 1 });
    let opts = Options {
        stale: true,
        foreign: true,
        allow_keys: vec!["Portal".into()],
        detailed: false,
    };
    let report = clean::clean(&mut db.zdos.zdos, &names, &schema, &opts);
    assert_eq!(report.properties_removed, 0);
    assert!(db.zdos.zdos[idx].connection.is_some());
}

#[test]
fn connection_verdicts_follow_the_table() {
    let s = Schema::builtin();
    let portal = stable_hash("portal_wood");
    let tree = stable_hash("Beech1");
    let spawner = stable_hash("Spawner_Boar");

    assert_eq!(
        s.judge_connection(portal, Some("Portal"), false),
        Verdict::Owned("TeleportWorld".into())
    );
    // Both ends of a portal pair come from ZDOMan.GetPortals().
    assert_eq!(
        s.judge_connection(portal, Some("Portal"), true),
        Verdict::Owned("TeleportWorld".into())
    );
    assert!(matches!(
        s.judge_connection(tree, Some("Portal"), false),
        Verdict::Foreign(_)
    ));
    assert_eq!(
        s.judge_connection(spawner, Some("Spawned"), false),
        Verdict::Owned("CreatureSpawner".into())
    );
    assert!(matches!(
        s.judge_connection(tree, Some("Spawned"), false),
        Verdict::Foreign(_)
    ));
    // CreatureSpawner.Spawn only requires a ZNetView on what it instantiates,
    // and ZSyncTransform attaches to any networked parent, so neither target
    // side can be judged.
    assert_eq!(
        s.judge_connection(tree, Some("Spawned"), true),
        Verdict::Universal
    );
    assert_eq!(
        s.judge_connection(tree, Some("SyncTransform"), true),
        Verdict::Universal
    );
    assert_eq!(
        s.judge_connection(tree, None, false),
        Verdict::UnknownKey,
        "unknown connection type says nothing"
    );
    assert_eq!(
        s.judge_connection(stable_hash("NoSuchPrefab"), Some("Portal"), false),
        Verdict::UnknownPrefab
    );
}

#[test]
fn removing_a_connection_survives_a_write_and_reread() {
    let Some(mut db) = load_example() else { return };
    let names = NameTable::builtin();
    let schema = Schema::builtin();

    // Dropping a connection clears the 0x0001 flag bit, so the ZDO is written
    // one table shorter than it was read; make sure the stream stays in step.
    let tree = stable_hash("Beech1");
    let idx = db.zdos.zdos.iter().position(|z| z.prefab == tree).unwrap();
    db.zdos.zdos[idx].connection = Some(Connection { kind: 1, hash: 77 });
    let with_conn = db.to_bytes().unwrap();
    let reread = WorldDb::parse(&with_conn).unwrap();
    assert_eq!(
        reread.zdos.zdos[idx].connection,
        Some(Connection { kind: 1, hash: 77 }),
        "the injected connection is really in the bytes"
    );

    let opts = Options {
        stale: true,
        foreign: true,
        allow_keys: vec![],
        detailed: false,
    };
    let report = clean::clean(&mut db.zdos.zdos, &names, &schema, &opts);
    assert_eq!(report.properties_removed, 1);

    let cleaned = WorldDb::parse(&db.to_bytes().unwrap()).unwrap();
    assert_eq!(cleaned.zdos.zdos.len(), db.zdos.zdos.len());
    assert_eq!(cleaned.zdos.zdos[idx].connection, None);
    assert_eq!(cleaned, db, "clean output re-reads to the same world");
    assert!(
        cleaned.to_bytes().unwrap().len() < with_conn.len(),
        "the connection's five bytes are gone"
    );
}

#[test]
fn animator_keys_resolve_and_belong_to_zsyncanimation() {
    let names = NameTable::builtin();
    let s = Schema::builtin();

    // ZSyncAnimation keys are 438569 + CRC32(param), so they never appear in
    // the stable-hash tables and used to show up as bare hashes on every
    // creature in every world.
    assert_eq!(
        names.resolve(animator_key("forward_speed")),
        Some("anim:forward_speed")
    );
    assert_eq!(
        names.resolve(animator_hash("anim_speed")),
        Some("anim:anim_speed"),
        "anim_speed is written without the offset"
    );

    let boar = stable_hash("Boar");
    let arch = stable_hash("stone_arch");
    for param in ["forward_speed", "onGround", "statef", "flapping"] {
        assert_eq!(
            s.judge(
                boar,
                animator_key(param),
                names.resolve(animator_key(param))
            ),
            Verdict::Owned("ZSyncAnimation".into()),
            "{param} on a creature"
        );
        assert!(
            matches!(
                s.judge(
                    arch,
                    animator_key(param),
                    names.resolve(animator_key(param))
                ),
                Verdict::Foreign(_)
            ),
            "{param} on a building piece"
        );
    }
}

#[test]
fn concatenated_index_keys_resolve() {
    let names = NameTable::builtin();
    let s = Schema::builtin();
    // "slot" + i and "drop_hash" + i are built without a separator, a form the
    // derived-name generator did not cover.
    assert_eq!(names.resolve(stable_hash("slot0")), Some("slot0"));
    assert_eq!(names.resolve(stable_hash("drop_hash0")), Some("drop_hash0"));
    assert_eq!(
        s.judge(
            stable_hash("piece_cookingstation"),
            stable_hash("slot0"),
            Some("slot0")
        ),
        Verdict::Owned("CookingStation".into())
    );
    assert_eq!(
        s.judge(
            stable_hash("Greydwarf_ragdoll"),
            stable_hash("drop_hash0"),
            Some("drop_hash0")
        ),
        Verdict::Owned("Ragdoll".into())
    );
}

#[test]
fn spawn_list_prefabs_get_spawner_names() {
    let names = NameTable::builtin();
    let s = Schema::builtin();
    // A SpawnSystemList is not limited to prefabs with creature components,
    // so b_Fish16 and friends have to be nameable too.
    for key in [
        "b_Fish16",
        "b_Seagal5",
        "b_odin30",
        "e_Spawner_CharredStone_event1",
    ] {
        assert_eq!(names.resolve(stable_hash(key)), Some(key), "{key}");
        assert_eq!(
            s.judge(stable_hash("_ZoneCtrl"), stable_hash(key), Some(key)),
            Verdict::Owned("SpawnSystem".into()),
            "{key}"
        );
    }
}

#[test]
fn legacy_matvar_stays_on_the_pieces_that_carry_it() {
    let s = Schema::builtin();
    // The grausten pieces swapped MaterialVariation for RandomMaterialValues
    // but still carry MatVar0 from older builds. It is a version leftover, not
    // corruption, so clean must leave it alone.
    assert_eq!(
        s.judge(
            stable_hash("Piece_grausten_wall_4x2"),
            stable_hash("MatVar0"),
            Some("MatVar0")
        ),
        Verdict::Owned("RandomMaterialValues".into())
    );
}
