//! Save-format version gates, one named constant per feature.
//!
//! The game guards each field with a literal (`if (version >= 26)`) or, since
//! 1.0, a member of the `Version.World` / `Version.Player` /
//! `Version.PlayerData` / `Version.Item` enums; here each gets a name so the
//! schema structs read as documentation of when a field appeared. Values come
//! from `Version.cs`, `World.cs`, `ZNet.cs`, `ZDO.cs`, `ZDOMan.cs`,
//! `ZoneSystem.cs`, `RandEventSystem.cs`, `PersistentEventSystem.cs`,
//! `PlayerProfile.cs`, `Player.cs`, `Inventory.cs` and `ItemDrop.cs` in the
//! game's assembly.

/// `m_worldVersion`: shared by the world metadata and world data files.
pub mod world {
    /// `Version.World.DeepNorth`, what 1.0 writes.
    pub const CURRENT: i32 = 41;
    /// Oldest version this tool can read. Older `.db` files store each ZDO as
    /// a separate length-prefixed package with a different layout.
    pub const MIN_SUPPORTED: i32 = ZDO_INLINE_STREAM;
    /// The last version of the single-file `.fwl` + `.db` layout
    /// (`DeepNorthNoChunk2`). From [`CHUNKED_SAVE`] on a world is a directory,
    /// and `World.LoadWorld` refuses a file whose layout disagrees with its
    /// version, so a `.db` can never carry anything newer than this.
    pub const LEGACY_CURRENT: i32 = 39;

    // .fwl
    pub const WORLD_GEN_VERSION: i32 = 26;
    pub const NEEDS_DB: i32 = 30;
    pub const STARTING_GLOBAL_KEYS: i32 = 32;
    /// The `.fwl2` ends with the list of players who have joined the world
    /// (`ZNet.CrossNetworkUserInfo`).
    pub const PLAYER_HISTORY: i32 = 41;

    // .db
    pub const NET_TIME: i32 = 4;
    pub const ZONE_SYSTEM: i32 = 12;
    pub const ZONE_PGW_VERSION: i32 = 13;
    pub const ZONE_GLOBAL_KEYS: i32 = 14;
    pub const RAND_EVENTS: i32 = 15;
    pub const ZONE_LOCATIONS: i32 = 18;
    pub const ZONE_LOCATION_PLACED: i32 = 19;
    pub const ZONE_LOCATIONS_GENERATED: i32 = 20;
    pub const ZONE_LOCATION_VERSION: i32 = 21;
    pub const RAND_EVENT_DETAILS: i32 = 25;
    /// ZDOs are streamed inline with a flags word instead of per-ZDO packages.
    pub const ZDO_INLINE_STREAM: i32 = 31;
    /// Property-table counts use the 1-or-2 byte `NumItems` encoding.
    pub const ZDO_WIDE_COUNTS: i32 = 33;
    /// `PersistentEventSystem` state follows the random-event state. The
    /// class arrived with 1.0, which reads the section only when bytes remain
    /// in the file, so a `.db` this old written by an earlier build simply
    /// ends without it.
    pub const PERSISTENT_EVENTS: i32 = 35;
    /// `Version.World.ChunkedSave`: the world is a directory of `_main.<N>.*`
    /// files plus one `.chunk` file per group of zones.
    pub const CHUNKED_SAVE: i32 = 40;
    /// The compact ZDO record: no stored sector, an optional 2-short
    /// position, and a 2-or-4 byte rotation (`ZDO.Save` / `ZDO.Load`).
    pub const ZDO_COMPACT: i32 = CHUNKED_SAVE;
    /// A location instance names its prefab by hash instead of string.
    pub const LOCATION_HASH: i32 = CHUNKED_SAVE;
}

/// `m_worldGenVersion` stored in the `.fwl`, as stamped into worlds the
/// current game creates (`World..ctor`).
pub const WORLD_GEN_CURRENT: i32 = 2;

/// What the game assumes when the `.fwl` predates `world::WORLD_GEN_VERSION`
/// and carries no `m_worldGenVersion`: `World.LoadWorld` leaves the field at
/// its zero default and saves it back as zero, keeping the world on the
/// legacy generator forever (`WorldGenerator.VersionSetup` widens
/// `m_minMountainDistance` at <= 0 and the Mistlands/swamp thresholds at
/// <= 1). Upgrading such a file must preserve that, not stamp the current
/// version, or unexplored zones generate with different biome placement than
/// the ones already on disk.
pub const WORLD_GEN_LEGACY: i32 = 0;

/// The `.chunks` map, the `.ok` marker and every `.chunk` file open with a
/// world version of their own. 1.0 writes this one everywhere
/// (`ZDOMan.SaveChunk`, `ChunkSaveMapping.Save`, `ZNet.SaveWorldThread`).
pub const CHUNK_VERSION: i32 = world::CURRENT;

/// `m_playerVersion` for `.fch` character files.
pub mod player {
    /// `Version.Player.DeepNorth`.
    pub const CURRENT: i32 = 46;
    pub const STATS_LEGACY: i32 = 28;
    pub const MAP_DATA: i32 = 29;
    pub const DEATH_POINT: i32 = 30;
    pub const STATS_FLOAT: i32 = 38;
    pub const FIRST_SPAWN: i32 = 40;
    pub const EXTENDED_STATS: i32 = 42;
    /// `Version.Player.AbandonedDN`: a public-test build that already wrote
    /// the tiered stats of [`TIERED_STATS`], then was superseded by two
    /// versions that did not. The game special-cases it, so this does too.
    pub const ABANDONED_DN: i32 = 44;
    /// Stats are stored per difficulty tier, each tier carrying its own
    /// known-world and tracked-stat maps, and the profile-level maps of
    /// [`STATS_FLOAT`] / [`EXTENDED_STATS`] are gone.
    pub const TIERED_STATS: i32 = 46;
    /// Number of `PlayerStatType` entries written by the current game.
    pub const STAT_COUNT: usize = 205;
    /// Number of stat tiers written by the current game
    /// (`PlayerProfile.m_playerStats.Length`).
    pub const STAT_TIERS: usize = 10;
    /// Number of enemy-stat maps per tier (`PlayerStats.m_enemyStats`).
    pub const ENEMY_STAT_GROUPS: usize = 5;

    /// Whether a profile of this version stores its stats per tier.
    pub const fn has_tiered_stats(version: i32) -> bool {
        version >= TIERED_STATS || version == ABANDONED_DN
    }
}

/// The version of the opaque `Player.Save` blob stored inside a `.fch`
/// (`Player.Load`). It is versioned separately from the profile around it.
pub mod player_data {
    /// `Version.PlayerData.ChunkedNorth`.
    pub const CURRENT: i32 = 33;
    /// The only version that stores a `ZDOID` before the inventory.
    pub const LEGACY_ZDOID: i32 = 2;
    pub const BEARD_AND_HAIR: i32 = 4;
    pub const COLORS: i32 = 5;
    pub const UNIQUES: i32 = 6;
    pub const MAX_HEALTH: i32 = 7;
    /// A `first spawn` flag lived here from this version until
    /// [`FIRST_SPAWN_MOVED`], where it moved out to the profile.
    pub const FIRST_SPAWN: i32 = 8;
    pub const TROPHIES: i32 = 9;
    pub const MAX_STAMINA: i32 = 10;
    pub const MODEL_INDEX: i32 = 11;
    pub const FOODS: i32 = 12;
    /// A seventh float in each pre-[`FOOD_NAMED`] food entry.
    pub const FOOD_LEGACY_EXTRA: i32 = 13;
    /// Food entries became `(name, nutrition)` instead of six or seven
    /// unlabelled floats.
    pub const FOOD_NAMED: i32 = 14;
    pub const KNOWN_STATION_LEVELS: i32 = 15;
    pub const FOOD_STAMINA: i32 = 16;
    pub const SKILLS: i32 = 17;
    pub const KNOWN_BIOMES: i32 = 18;
    /// Versions 19 and 20 wrote no shown-tutorials list; it came back at
    /// [`TUTORIALS_RESTORED`].
    pub const TUTORIALS_DROPPED: i32 = 19;
    pub const TIME_SINCE_DEATH: i32 = 20;
    pub const TUTORIALS_RESTORED: i32 = 21;
    pub const KNOWN_TEXTS: i32 = 22;
    pub const GUARDIAN_POWER: i32 = 23;
    pub const GUARDIAN_POWER_COOLDOWN: i32 = 24;
    /// A food entry stores its remaining burn time, not its nutrition.
    pub const FOOD_TIME: i32 = 25;
    pub const CUSTOM_DATA: i32 = 26;
    pub const FIRST_SPAWN_MOVED: i32 = 28;
    /// `Version.PlayerData.AbandonedDN`: the public-test build that already
    /// wrote biome names and the build-menu blob of [`BIOME_NAMES`].
    pub const ABANDONED_DN: i32 = 31;
    /// Known biomes are stored by name instead of `Heightmap.Biome` value,
    /// and the blob ends with the build menu's saved state.
    pub const BIOME_NAMES: i32 = 33;

    /// Whether the known-biome list of this version holds names.
    pub const fn has_biome_names(version: i32) -> bool {
        version >= BIOME_NAMES || version == ABANDONED_DN
    }
}

/// `m_itemDataVersion` for serialized inventories (containers, tombstones,
/// the player's own) and single items (`ItemDrop.ItemData.Save`).
pub mod inventory {
    /// `Version.Item.ChunksNCheats`.
    pub const CURRENT: i32 = 109;
    pub const QUALITY: i32 = 101;
    pub const VARIANT: i32 = 102;
    pub const CRAFTER: i32 = 103;
    pub const CUSTOM_DATA: i32 = 104;
    pub const WORLD_LEVEL: i32 = 105;
    pub const PICKED_UP: i32 = 106;
    /// `Version.Item.AbandonedDN`: a public-test build that appended the
    /// `cheated` flag of [`CHEATED`] to the old item layout.
    pub const ABANDONED_DN: i32 = 107;
    /// `Version.Item.Smaller`: the compact item record with a flags byte,
    /// the prefab as a hash, and a 16-bit item count.
    pub const COMPACT: i32 = 108;
    /// Each item ends with a flags byte carrying `m_cheated`.
    pub const CHEATED: i32 = 109;

    /// Whether items of this version end with the cheated flag.
    pub const fn has_cheated(version: i32) -> bool {
        version >= CHEATED || version == ABANDONED_DN
    }
}
