//! `.fch` character profile (`PlayerProfile.SavePlayerToDisk` /
//! `PlayerProfile.LoadPlayerFromDisk`).
//!
//! On disk: `i32` payload length, payload, `i32` hash length, SHA-512 of the
//! payload. The payload ends with an opaque `Player.Save` blob holding the
//! in-world state (inventory, skills, appearance) that this tool leaves as is.

use anyhow::{Context, Result, bail};
use binrw::io::Cursor;
use binrw::{BinRead, BinWrite, binrw};
use serde::Serialize;
use sha2::{Digest, Sha512};

use super::inventory::Inventory;
use super::primitives::{
    Bool, ByteArray, CsString, IntList, StringFloatEntry, StringFloatMap, StringIntMap, StringList,
    StringStringMap, Vec3,
};
use super::versions::{player, player_data};

#[binrw]
#[brw(little)]
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerProfile {
    pub version: i32,
    /// From [`player::TIERED_STATS`]: one block of stats per difficulty
    /// tier, each with its own maps.
    #[br(if(player::has_tiered_stats(version)))]
    pub tiered_stats: Option<TieredStats>,
    /// [`player::STATS_FLOAT`] up to the tiered layout: one flat stat array.
    #[br(if(version >= player::STATS_FLOAT && !player::has_tiered_stats(version)))]
    pub stats: Option<FloatList>,
    #[br(if((player::STATS_LEGACY..player::STATS_FLOAT).contains(&version)))]
    pub legacy_stats: Option<LegacyStats>,
    #[br(if(version >= player::FIRST_SPAWN))]
    pub first_spawn: Option<Bool>,
    #[br(args(version))]
    pub worlds: WorldDataList,
    pub name: CsString,
    pub player_id: i64,
    pub start_seed: CsString,
    #[br(if(version >= player::STATS_FLOAT), args(version))]
    pub extended: Option<ExtendedProfile>,
    #[br(temp)]
    #[bw(calc = Bool(player_data.is_some()))]
    has_player_data: Bool,
    #[br(if(has_player_data.0))]
    pub player_data: Option<ByteArray>,
}

/// `PlayerProfile.m_playerStats` from version 46: a stat count, a tier
/// count, then that many [`TierStats`]. Tier 0 is the raw total; the rest
/// are per achievement difficulty (`Achievements.GetCurrentAchievementDifficultyIndex`).
#[binrw]
#[brw(little)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TieredStats {
    #[br(temp)]
    #[bw(calc = tiers.first().map_or(0, |t| t.values.len() as i32))]
    stat_count: i32,
    #[br(temp)]
    #[bw(calc = tiers.len() as i32)]
    tier_count: i32,
    #[br(count = tier_count, args { inner: (stat_count,) })]
    pub tiers: Vec<TierStats>,
}

/// One tier's `PlayerStats`.
#[binrw]
#[brw(little)]
#[br(import(stat_count: i32))]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TierStats {
    /// One value per `PlayerStatType`, in enum order.
    #[br(count = stat_count)]
    pub values: Vec<f32>,
    /// World name -> seconds played.
    pub known_worlds: StringFloatMap,
    pub known_world_keys: StringFloatMap,
    pub known_commands: StringFloatMap,
    /// `m_enemyStats`: one map per enemy group.
    pub enemy_stats: StringFloatMapList,
    pub item_pickup_stats: StringFloatMap,
    pub item_craft_stats: StringFloatMap,
    pub pickable_stats: StringFloatMap,
    pub food_eaten_stats: StringFloatMap,
    pub pieces_placed_stats: StringFloatMap,
}

/// `i32` count followed by that many [`StringFloatMap`]s.
#[binrw]
#[brw(little)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StringFloatMapList {
    #[br(temp)]
    #[bw(calc = maps.len() as i32)]
    count: i32,
    #[br(count = count)]
    pub maps: Vec<StringFloatMap>,
}

#[binrw]
#[brw(little)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FloatList {
    #[br(temp)]
    #[bw(calc = values.len() as i32)]
    count: i32,
    #[br(count = count)]
    pub values: Vec<f32>,
}

#[binrw]
#[brw(little)]
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct LegacyStats {
    pub enemy_kills: i32,
    pub deaths: i32,
    pub crafts_or_upgrades: i32,
    pub builds: i32,
}

#[binrw]
#[brw(little)]
#[br(import(version: i32))]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorldDataList {
    #[br(temp)]
    #[bw(calc = worlds.len() as i32)]
    count: i32,
    #[br(count = count, args { inner: (version,) })]
    pub worlds: Vec<WorldPlayerData>,
}

#[binrw]
#[brw(little)]
#[br(import(version: i32))]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorldPlayerData {
    pub world_uid: i64,
    pub have_custom_spawn_point: Bool,
    pub spawn_point: Vec3,
    pub have_logout_point: Bool,
    pub logout_point: Vec3,
    #[br(if(version >= player::DEATH_POINT))]
    pub death: Option<DeathPoint>,
    pub home_point: Vec3,
    #[br(if(version >= player::MAP_DATA))]
    pub map_data: Option<OptionalBytes>,
}

#[binrw]
#[brw(little)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeathPoint {
    pub have_death_point: Bool,
    pub death_point: Vec3,
}

#[binrw]
#[brw(little)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OptionalBytes {
    #[br(temp)]
    #[bw(calc = Bool(data.is_some()))]
    present: Bool,
    #[br(if(present.0))]
    pub data: Option<ByteArray>,
}

#[binrw]
#[brw(little)]
#[br(import(version: i32))]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExtendedProfile {
    pub used_cheats: Bool,
    /// Unix seconds of the creation date.
    pub date_created: i64,
    /// Profile-level maps, until the tiered layout moved them into tier 0.
    #[br(if(!player::has_tiered_stats(version)), args(version))]
    pub known: Option<KnownMaps>,
}

#[binrw]
#[brw(little)]
#[br(import(version: i32))]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KnownMaps {
    pub known_worlds: StringFloatMap,
    pub known_world_keys: StringFloatMap,
    pub known_commands: StringFloatMap,
    #[br(if(version >= player::EXTENDED_STATS))]
    pub tracked: Option<TrackedStats>,
}

#[binrw]
#[brw(little)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrackedStats {
    pub enemy_stats: StringFloatMap,
    pub item_pickup_stats: StringFloatMap,
    pub item_craft_stats: StringFloatMap,
}

/// The leading fields of the opaque `Player.Save` blob (`Player.Load`),
/// decoded as far as the active food slots. Skills and everything after them
/// (custom data, current stamina and eitr) are left alone.
#[binrw]
#[brw(little)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerDataHead {
    pub version: i32,
    #[br(if(version >= player_data::MAX_HEALTH))]
    pub max_health: Option<f32>,
    pub health: f32,
    #[br(if(version >= player_data::MAX_STAMINA))]
    pub max_stamina: Option<f32>,
    #[br(if((player_data::FIRST_SPAWN..player_data::FIRST_SPAWN_MOVED).contains(&version)))]
    pub first_spawn_legacy: Option<Bool>,
    #[br(if(version >= player_data::TIME_SINCE_DEATH))]
    pub time_since_death: Option<f32>,
    #[br(if(version >= player_data::GUARDIAN_POWER))]
    pub guardian_power: Option<CsString>,
    #[br(if(version >= player_data::GUARDIAN_POWER_COOLDOWN))]
    pub guardian_power_cooldown: Option<f32>,
    #[br(if(version == player_data::LEGACY_ZDOID))]
    pub legacy_zdoid: Option<LegacyZdoid>,
    pub inventory: Inventory,
    pub known_recipes: StringList,
    /// Station names alone, before the game recorded the level reached.
    #[br(if(version < player_data::KNOWN_STATION_LEVELS))]
    pub known_stations_legacy: Option<StringList>,
    #[br(if(version >= player_data::KNOWN_STATION_LEVELS))]
    pub known_stations: Option<StringIntMap>,
    pub known_materials: StringList,
    #[br(if(!(player_data::TUTORIALS_DROPPED..player_data::TUTORIALS_RESTORED).contains(&version)))]
    pub shown_tutorials: Option<StringList>,
    #[br(if(version >= player_data::UNIQUES))]
    pub uniques: Option<StringList>,
    #[br(if(version >= player_data::TROPHIES))]
    pub trophies: Option<StringList>,
    /// `Heightmap.Biome` values, one per discovered biome, until
    /// [`player_data::BIOME_NAMES`].
    #[br(if(version >= player_data::KNOWN_BIOMES && !player_data::has_biome_names(version)))]
    pub known_biomes: Option<IntList>,
    /// Biome names (`BiomeSector.GetBiomeName`), from
    /// [`player_data::BIOME_NAMES`].
    #[br(if(player_data::has_biome_names(version)))]
    pub known_biome_names: Option<StringList>,
    #[br(if(version >= player_data::KNOWN_TEXTS))]
    pub known_texts: Option<StringStringMap>,
    #[br(if(version >= player_data::BEARD_AND_HAIR))]
    pub appearance: Option<Appearance>,
    #[br(if(version >= player_data::COLORS))]
    pub colors: Option<Colors>,
    #[br(if(version >= player_data::MODEL_INDEX))]
    pub model_index: Option<i32>,
    /// The food currently being digested: up to three entries, each burning
    /// down to nothing.
    #[br(if(version >= player_data::FOODS), args(version))]
    pub foods: Option<FoodList>,
}

#[binrw]
#[brw(little)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Appearance {
    pub beard: CsString,
    pub hair: CsString,
}

#[binrw]
#[brw(little)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Colors {
    pub skin: Vec3,
    pub hair: Vec3,
}

#[binrw]
#[brw(little)]
#[br(import(version: i32))]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FoodList {
    #[br(temp)]
    #[bw(calc = items.len() as i32)]
    count: i32,
    #[br(count = count, args { inner: (version,) })]
    pub items: Vec<Food>,
}

/// One digesting food item (`Player.Food`). What follows the name changed
/// twice: six or seven unlabelled floats, then the health and stamina the
/// item was still granting, then the remaining burn time on its own.
#[binrw]
#[brw(little)]
#[br(import(version: i32))]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Food {
    /// Item prefab name, e.g. `CookedMeat`.
    pub name: CsString,
    /// Seconds of digestion left, counting down to zero.
    #[br(if(version >= player_data::FOOD_TIME))]
    pub time: Option<f32>,
    #[br(if((player_data::FOOD_NAMED..player_data::FOOD_TIME).contains(&version)))]
    pub health: Option<f32>,
    #[br(if((player_data::FOOD_STAMINA..player_data::FOOD_TIME).contains(&version)))]
    pub stamina: Option<f32>,
    #[br(if(version < player_data::FOOD_NAMED), args(version))]
    pub legacy: Option<LegacyFood>,
}

/// The pre-[`player_data::FOOD_NAMED`] payload, whose fields the game reads
/// and discards.
#[binrw]
#[brw(little)]
#[br(import(version: i32))]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LegacyFood {
    pub values: [f32; 6],
    #[br(if(version >= player_data::FOOD_LEGACY_EXTRA))]
    pub extra: Option<f32>,
}

#[binrw]
#[brw(little)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LegacyZdoid {
    pub user_id: i64,
    pub id: u32,
}

/// `PlayerStatType` names in enum order (index = stat slot).
pub const STAT_NAMES: &[&str] = &[
    "Deaths",
    "CraftsOrUpgrades",
    "Builds",
    "Jumps",
    "Cheats",
    "EnemyHits",
    "EnemyKills",
    "EnemyKillsLastHits",
    "PlayerHits",
    "PlayerKills",
    "HitsTakenEnemies",
    "HitsTakenPlayers",
    "ItemsPickedUp",
    "Crafts",
    "Upgrades",
    "PortalsUsed",
    "DistanceTraveled",
    "DistanceWalk",
    "DistanceRun",
    "DistanceSail",
    "DistanceAir",
    "TimeInBase",
    "TimeOutOfBase",
    "Sleep",
    "ItemStandUses",
    "ArmorStandUses",
    "WorldLoads",
    "TreeChops",
    "Tree",
    "TreeTier0",
    "TreeTier1",
    "TreeTier2",
    "TreeTier3",
    "TreeTier4",
    "TreeTier5",
    "LogChops",
    "Logs",
    "MineHits",
    "Mines",
    "MineTier0",
    "MineTier1",
    "MineTier2",
    "MineTier3",
    "MineTier4",
    "MineTier5",
    "RavenHits",
    "RavenTalk",
    "RavenAppear",
    "CreatureTamed",
    "FoodEaten",
    "SkeletonSummons",
    "ArrowsShot",
    "TombstonesOpenedOwn",
    "TombstonesOpenedOther",
    "TombstonesFit",
    "DeathByUndefined",
    "DeathByEnemyHit",
    "DeathByPlayerHit",
    "DeathByFall",
    "DeathByDrowning",
    "DeathByBurning",
    "DeathByFreezing",
    "DeathByPoisoned",
    "DeathBySmoke",
    "DeathByWater",
    "DeathByEdgeOfWorld",
    "DeathByImpact",
    "DeathByCart",
    "DeathByTree",
    "DeathBySelf",
    "DeathByStructural",
    "DeathByTurret",
    "DeathByBoat",
    "DeathByStalagtite",
    "DoorsOpened",
    "DoorsClosed",
    "BeesHarvested",
    "SapHarvested",
    "TurretAmmoAdded",
    "TurretTrophySet",
    "TrapArmed",
    "TrapTriggered",
    "PlaceStacks",
    "PortalDungeonIn",
    "PortalDungeonOut",
    "BossKills",
    "BossLastHits",
    "SetGuardianPower",
    "SetPowerEikthyr",
    "SetPowerElder",
    "SetPowerBonemass",
    "SetPowerModer",
    "SetPowerYagluth",
    "SetPowerQueen",
    "SetPowerAshlands",
    "SetPowerDeepNorth",
    "UseGuardianPower",
    "UsePowerEikthyr",
    "UsePowerElder",
    "UsePowerBonemass",
    "UsePowerModer",
    "UsePowerYagluth",
    "UsePowerQueen",
    "UsePowerAshlands",
    "UsePowerDeepNorth",
    "DeathByCatapult",
    "DeathByCinderFire",
    "DeathByAshlandsOcean",
    "DeathByIncinerator",
    "CraftFood",
    "CraftFoodBonus",
    "CraftGrill",
    "CraftGrillBurnt",
    "CraftGrillBonus",
    "CraftWeapon",
    "CraftArmor",
    "CraftTrinket",
    "CraftAmmo",
    "CraftMaterial",
    "CraftTool",
    "CraftTorch",
    "CraftBait",
    "CraftOther",
    "HarvestCrop",
    "HarvestBerry",
    "HarvestMushroom",
    "HarvestVine",
    "HarvestBonus",
    "ConsecutiveDaysSurvived",
    "ConsecutiveDaysSurvivedMax",
    "MaxBuildingHeight",
    "MaxBuildingHeightWorld",
    "MaxComfort",
    "TreasureBuriedFound",
    "TreasureDungeonFound",
    "TreasureLocationFound",
    "LeviathanSink",
    "LavaLeviathanSink",
    "ExploreNorth",
    "ExploreSouth",
    "ExploreEast",
    "ExploreWest",
    "ExploreNorthNoMap",
    "ExploreSouthNoMap",
    "ExploreEastNoMap",
    "ExploreWestNoMap",
    "BossKillMultiplayer",
    "BossKillSolo",
    "VillagePointsMax",
    "DeepestDungeon",
    "DistanceSailHelm",
    "PlayerSpawn",
    "DeathByTreeTier0",
    "DeathByTreeTier1",
    "DeathByTreeTier2",
    "DeathByTreeTier3",
    "DeathByTreeTier4",
    "DeathByTreeTier5",
    "FishHooked",
    "FishLost",
    "FishCaught",
    "FishCaughtTier0",
    "FishCaughtTier1",
    "FishCaughtTier2",
    "FishCaughtTier3",
    "FishCaughtTier4",
    "FishCaughtTier5",
    "FishCaughtTier6",
    "BuiltPieces",
    "BuiltPiecesNoDebt",
    "BuildPiecesRemoved",
    "BuildClusterMisc",
    "BuildClusterCrafting",
    "BuildClusterBuilding",
    "BuildClusterFloor",
    "BuildClusterWall",
    "BuildClusterRoof",
    "BuildClusterArchitecture",
    "BuildClusterFurniture",
    "BuildClusterLighting",
    "BuildClusterDecor",
    "BuildClusterStorage",
    "BuildClusterTransport",
    "BuildClusterFood",
    "BuildClusterMeads",
    "BuildClusterFeasts",
    "BuildClusterDefense",
    "BuildClusterStacks",
    "BuildClusterStairs",
    "BuildClusterDoors",
    "BuildClusterSeasonal",
    "TamedPetting",
    "TamedCommand",
    "TreeFir",
    "TreeOak",
    "TreePine",
    "TreeAshlands",
    "TreeYggdrasilShoot",
    "TreeSwamp",
    "TreeBeech",
    "TreeBirch",
    "TreeSnowFir",
    "TreeSnowPine",
    "DeathByDrawBridge",
    "DeathByAshlandsLava",
];

pub fn stat_index(name: &str) -> usize {
    STAT_NAMES
        .iter()
        .position(|n| *n == name)
        .expect("known stat name")
}

pub fn stat_name(index: usize) -> String {
    STAT_NAMES
        .get(index)
        .map_or_else(|| format!("Stat{index}"), |n| (*n).to_owned())
}

/// A parsed `.fch` file.
#[derive(Clone, Debug, PartialEq)]
pub struct CharacterFile {
    pub profile: PlayerProfile,
    /// Whether the stored SHA-512 matched the payload.
    pub hash_ok: bool,
}

impl CharacterFile {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut cursor = Cursor::new(bytes);
        let len = i32::read_le(&mut cursor).context("reading .fch payload length")?;
        let len = usize::try_from(len).context("negative .fch payload length")?;
        let start = 4;
        let end = start + len;
        if end > bytes.len() {
            bail!(
                ".fch payload length {len} exceeds file size {}",
                bytes.len()
            );
        }
        let payload = &bytes[start..end];
        cursor.set_position(end as u64);
        let hash_len = i32::read_le(&mut cursor).context("reading .fch hash length")?;
        let hash_len = usize::try_from(hash_len).context("negative .fch hash length")?;
        let hash_start = end + 4;
        let hash_end = hash_start + hash_len;
        if hash_end > bytes.len() {
            bail!(
                ".fch hash length {hash_len} exceeds file size {}",
                bytes.len()
            );
        }
        let stored_hash = &bytes[hash_start..hash_end];
        let hash_ok = Sha512::digest(payload).as_slice() == stored_hash;

        let mut payload_cursor = Cursor::new(payload);
        let profile =
            PlayerProfile::read_le(&mut payload_cursor).context("parsing .fch payload")?;
        let consumed = payload_cursor.position() as usize;
        if consumed != len {
            bail!(
                ".fch payload has {} unparsed trailing bytes",
                len - consumed
            );
        }
        Ok(CharacterFile { profile, hash_ok })
    }

    pub fn to_file_bytes(&self) -> Result<Vec<u8>> {
        let mut payload = Cursor::new(Vec::new());
        self.profile
            .write_le(&mut payload)
            .context("serializing .fch payload")?;
        let payload = payload.into_inner();
        let hash = Sha512::digest(&payload);
        let mut out = Cursor::new(Vec::with_capacity(payload.len() + hash.len() + 8));
        (payload.len() as i32).write_le(&mut out)?;
        std::io::Write::write_all(&mut out, &payload)?;
        (hash.len() as i32).write_le(&mut out)?;
        std::io::Write::write_all(&mut out, hash.as_slice())?;
        Ok(out.into_inner())
    }

    /// The decoded front of the in-world player blob: `None` when the profile
    /// carries no player data at all, `Some(Err)` when it is there but does
    /// not match the layout for its version.
    pub fn player_data_head(&self) -> Option<Result<PlayerDataHead>> {
        let data = self.profile.player_data.as_ref()?;
        Some(
            PlayerDataHead::read_le(&mut Cursor::new(&data.data))
                .context("parsing the in-world player data"),
        )
    }
}

impl PlayerProfile {
    /// The raw stat values, whichever layout holds them: tier 0 of the
    /// tiered stats, the flat array, or the four legacy counters spread
    /// into their slots. Empty for a profile too old to have any.
    pub fn stat_values(&self) -> Vec<f32> {
        if let Some(tiers) = &self.tiered_stats {
            return tiers
                .tiers
                .first()
                .map(|t| t.values.clone())
                .unwrap_or_default();
        }
        if let Some(s) = &self.stats {
            return s.values.clone();
        }
        let Some(l) = &self.legacy_stats else {
            return Vec::new();
        };
        let mut v = vec![0.0; player::STAT_COUNT];
        v[stat_index("Deaths")] = l.deaths as f32;
        v[stat_index("CraftsOrUpgrades")] = l.crafts_or_upgrades as f32;
        v[stat_index("Builds")] = l.builds as f32;
        v[stat_index("EnemyKills")] = l.enemy_kills as f32;
        v
    }

    /// World name -> seconds played, from wherever the profile keeps it.
    pub fn known_worlds(&self) -> &[StringFloatEntry] {
        if let Some(t) = self.tiered_stats.as_ref().and_then(|t| t.tiers.first()) {
            return &t.known_worlds.entries;
        }
        self.extended
            .as_ref()
            .and_then(|e| e.known.as_ref())
            .map_or(&[], |k| k.known_worlds.entries.as_slice())
    }

    /// Fill fields absent from older versions with the game's defaults and
    /// stamp the current version. Mirrors `PlayerProfile.LoadPlayerFromDisk`
    /// followed by `SavePlayerToDisk`: older stats land in tier 0 of the
    /// tiered layout, along with the profile-level maps.
    pub fn upgrade(&mut self) {
        let mut values = self.stat_values();
        values.resize(player::STAT_COUNT, 0.0);
        self.legacy_stats = None;
        self.stats = None;
        let known = self
            .extended
            .as_mut()
            .and_then(|e| e.known.take())
            .unwrap_or_default();
        if self.tiered_stats.is_none() {
            let mut tiers: Vec<TierStats> = (0..player::STAT_TIERS)
                .map(|_| TierStats {
                    values: vec![0.0; player::STAT_COUNT],
                    enemy_stats: StringFloatMapList {
                        maps: vec![StringFloatMap::default(); player::ENEMY_STAT_GROUPS],
                    },
                    ..TierStats::default()
                })
                .collect();
            let tracked = known.tracked.unwrap_or_default();
            tiers[0].values = values;
            tiers[0].known_worlds = known.known_worlds;
            tiers[0].known_world_keys = known.known_world_keys;
            tiers[0].known_commands = known.known_commands;
            tiers[0].enemy_stats.maps[0] = tracked.enemy_stats;
            tiers[0].item_pickup_stats = tracked.item_pickup_stats;
            tiers[0].item_craft_stats = tracked.item_craft_stats;
            self.tiered_stats = Some(TieredStats { tiers });
        }
        self.first_spawn.get_or_insert(Bool(false));
        for w in &mut self.worlds.worlds {
            w.death.get_or_insert_with(DeathPoint::default);
            w.map_data.get_or_insert_with(OptionalBytes::default);
        }
        self.extended.get_or_insert_with(|| ExtendedProfile {
            // The game uses 2021-02-02 for profiles predating the field.
            date_created: 1_612_224_000,
            ..ExtendedProfile::default()
        });
        self.version = player::CURRENT;
    }
}
