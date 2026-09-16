//! The 1.0 character profile: stats per difficulty tier, and the migration
//! of an older profile into it.

use binrw::io::Cursor;
use binrw::{BinRead, BinWrite};

use fejdit::format::fch::{CharacterFile, PlayerProfile, TierStats, TieredStats};
use fejdit::format::primitives::{CsString, StringFloatEntry, StringFloatMap};
use fejdit::format::versions::player;

fn example() -> Option<CharacterFile> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/jtatest.fch");
    let bytes = std::fs::read(path).ok()?;
    Some(CharacterFile::parse(&bytes).unwrap())
}

/// A pre-1.0 profile moves its stats and maps into tier 0 of the tiered
/// layout, with the other tiers empty, and the result reads back as
/// written. Skips without the (git-ignored) sample.
#[test]
fn old_profile_upgrades_into_tier_zero() {
    let Some(file) = example() else { return };
    let before = file.profile.clone();
    assert!(
        before.version < player::CURRENT,
        "the sample is meant to be old"
    );
    assert!(before.tiered_stats.is_none());
    let old_values = before.stat_values();
    let old_worlds = before.known_worlds().to_vec();
    assert!(!old_values.is_empty());

    let mut upgraded = file.clone();
    upgraded.profile.upgrade();
    let p = &upgraded.profile;
    assert_eq!(p.version, player::CURRENT);
    assert!(p.stats.is_none() && p.legacy_stats.is_none());
    assert_eq!(p.extended.as_ref().and_then(|e| e.known.as_ref()), None);
    let tiers = &p.tiered_stats.as_ref().unwrap().tiers;
    assert_eq!(tiers.len(), player::STAT_TIERS);
    for tier in tiers {
        assert_eq!(tier.values.len(), player::STAT_COUNT);
        assert_eq!(tier.enemy_stats.maps.len(), player::ENEMY_STAT_GROUPS);
    }
    assert_eq!(&tiers[0].values[..old_values.len()], old_values.as_slice());
    assert!(
        tiers[0].values[old_values.len()..]
            .iter()
            .all(|v| *v == 0.0)
    );
    assert_eq!(tiers[0].known_worlds.entries, old_worlds);
    assert!(
        tiers[1..]
            .iter()
            .all(|t| t.values.iter().all(|v| *v == 0.0))
    );
    assert_eq!(p.stat_values(), tiers[0].values);
    assert_eq!(p.known_worlds(), old_worlds.as_slice());
    assert_eq!(
        p.player_data, before.player_data,
        "the in-world blob is untouched"
    );

    let bytes = upgraded.to_file_bytes().unwrap();
    let back = CharacterFile::parse(&bytes).unwrap();
    assert!(back.hash_ok);
    assert_eq!(back, upgraded);
}

/// The tiered block as `PlayerProfile.SavePlayerToDisk` lays it out: the
/// stat count, the tier count, then each tier's values and maps.
#[test]
fn tiered_stats_round_trip() {
    let map = |k: &str, v: f32| StringFloatMap {
        entries: vec![StringFloatEntry {
            key: CsString::from(k),
            value: v,
        }],
    };
    let tier = TierStats {
        values: vec![1.0, 2.0, 3.0],
        known_worlds: map("Spacegutter", 360.0),
        known_world_keys: map("defeated_eikthyr default", 360.0),
        known_commands: StringFloatMap::default(),
        enemy_stats: fejdit::format::fch::StringFloatMapList {
            maps: vec![map("Greyling", 4.0), StringFloatMap::default()],
        },
        item_pickup_stats: map("Wood", 12.0),
        item_craft_stats: StringFloatMap::default(),
        pickable_stats: map("Raspberry", 2.0),
        food_eaten_stats: StringFloatMap::default(),
        pieces_placed_stats: map("wood_wall", 8.0),
    };
    let stats = TieredStats {
        tiers: vec![
            tier.clone(),
            TierStats {
                values: vec![0.0, 0.0, 0.0],
                enemy_stats: fejdit::format::fch::StringFloatMapList {
                    maps: vec![StringFloatMap::default(), StringFloatMap::default()],
                },
                ..TierStats::default()
            },
        ],
    };
    let mut out = Cursor::new(Vec::new());
    stats.write_le(&mut out).unwrap();
    let bytes = out.into_inner();
    assert_eq!(
        &bytes[..8],
        &[3, 0, 0, 0, 2, 0, 0, 0],
        "stat count, tier count"
    );
    let back = TieredStats::read_le(&mut Cursor::new(&bytes)).unwrap();
    assert_eq!(back, stats);

    let profile = PlayerProfile {
        version: player::CURRENT,
        tiered_stats: Some(stats),
        stats: None,
        legacy_stats: None,
        first_spawn: Some(fejdit::format::primitives::Bool(false)),
        worlds: Default::default(),
        name: CsString::from("Tester"),
        player_id: 42,
        start_seed: CsString::from("seed"),
        extended: Some(fejdit::format::fch::ExtendedProfile {
            used_cheats: fejdit::format::primitives::Bool(false),
            date_created: 1_700_000_000,
            known: None,
        }),
        player_data: None,
    };
    let mut out = Cursor::new(Vec::new());
    profile.write_le(&mut out).unwrap();
    let back = PlayerProfile::read_le(&mut Cursor::new(out.into_inner())).unwrap();
    assert_eq!(back, profile);
    assert_eq!(back.stat_values(), vec![1.0, 2.0, 3.0]);
    assert_eq!(back.known_worlds()[0].key.as_str(), "Spacegutter");
}
