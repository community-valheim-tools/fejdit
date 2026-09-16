//! `.db` world data, the single-file layout of world versions up to 39
//! (`ZNet.LoadOldWorld`; the last build to write it was 0.221.x).
//!
//! Sections in order: version, net time, ZDO stream (`ZDOMan.Load`), zone
//! system state (`ZoneSystem.LoadOld`), random event state
//! (`RandEventSystem`), and from version 35 an optional persistent-event
//! blob (`PersistentEventSystem`).

use anyhow::{Context, Result, bail};
use binrw::io::{Cursor, Read, Seek, SeekFrom};
use binrw::{BinRead, BinResult, BinWrite, binrw};
use serde::Serialize;

use super::primitives::{Bool, ByteArray, CsString, StringList, Vec2i, Vec3};
use super::versions::world;
use super::zdo::Zdo;

#[binrw]
#[brw(little)]
#[derive(Clone, Debug, PartialEq)]
pub struct WorldDb {
    #[br(assert(
        version >= world::MIN_SUPPORTED,
        "world data version {} is older than the oldest supported version {}; load and save it in the game once to upgrade it",
        version, world::MIN_SUPPORTED
    ))]
    pub version: i32,
    #[br(if(version >= world::NET_TIME))]
    pub net_time: Option<f64>,
    #[br(args(version))]
    #[bw(args(*version))]
    pub zdos: ZdoSection,
    #[br(if(version >= world::ZONE_SYSTEM), args(version))]
    pub zone_system: Option<ZoneSystemData>,
    #[br(if(version >= world::RAND_EVENTS), args(version))]
    pub rand_events: Option<RandEventData>,
    /// `PersistentEventSystem` state: a length-prefixed, Brotli-compressed
    /// JSON document, kept opaque. The class arrived with 1.0 and reads the
    /// section only when the file has bytes left, so a `.db` of this version
    /// written by an earlier build ends before it.
    #[br(if(version >= world::PERSISTENT_EVENTS), parse_with = read_trailing_bytes)]
    pub persistent_events: Option<ByteArray>,
}

/// A byte array that may simply not be there: `None` when fewer than four
/// bytes remain, mirroring `PersistentEventSystem.Load`.
#[binrw::parser(reader, endian)]
fn read_trailing_bytes() -> BinResult<Option<ByteArray>> {
    if remaining(reader)? < 4 {
        return Ok(None);
    }
    ByteArray::read_options(reader, endian, ()).map(Some)
}

fn remaining<R: Read + Seek>(reader: &mut R) -> BinResult<u64> {
    let pos = reader.stream_position()?;
    let end = reader.seek(SeekFrom::End(0))?;
    reader.seek(SeekFrom::Start(pos))?;
    Ok(end - pos)
}

/// `ZDOMan.SaveAsync` / `ZDOMan.Load` for version 31+.
#[binrw]
#[brw(little)]
#[br(import(version: i32))]
#[bw(import(version: i32))]
#[derive(Clone, Debug, PartialEq)]
pub struct ZdoSection {
    pub session_id: i64,
    pub next_uid: u32,
    #[br(temp)]
    #[bw(calc = zdos.len() as i32)]
    count: i32,
    #[br(count = count, args { inner: (version,) })]
    #[bw(args(version))]
    pub zdos: Vec<Zdo>,
}

#[binrw]
#[brw(little)]
#[br(import(version: i32))]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ZoneSystemData {
    #[br(temp)]
    #[bw(calc = generated_zones.len() as i32)]
    zone_count: i32,
    #[br(count = zone_count)]
    pub generated_zones: Vec<Vec2i>,
    /// Always written as 0 by the current game.
    #[br(if(version >= world::ZONE_PGW_VERSION))]
    pub pgw_version: Option<i32>,
    #[br(if(version >= world::ZONE_LOCATION_VERSION))]
    pub location_version: Option<i32>,
    #[br(if(version >= world::ZONE_GLOBAL_KEYS))]
    pub global_keys: Option<StringList>,
    #[br(if(version >= world::ZONE_LOCATIONS_GENERATED))]
    pub locations_generated: Option<Bool>,
    #[br(if(version >= world::ZONE_LOCATIONS), args(version))]
    pub locations: Option<LocationList>,
}

#[binrw]
#[brw(little)]
#[br(import(version: i32))]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LocationList {
    #[br(temp)]
    #[bw(calc = locations.len() as i32)]
    count: i32,
    #[br(count = count, args { inner: (version,) })]
    pub locations: Vec<LocationInstance>,
}

/// One placed or planned location. The prefab is named outright up to
/// version 39 and by `GetStableHashCode` from [`world::LOCATION_HASH`]; only
/// one of the two fields is ever set.
#[binrw]
#[brw(little)]
#[br(import(version: i32))]
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct LocationInstance {
    #[br(if(version < world::LOCATION_HASH))]
    pub prefab_name: Option<CsString>,
    #[br(if(version >= world::LOCATION_HASH))]
    pub prefab_hash: Option<i32>,
    pub position: Vec3,
    #[br(if(version >= world::ZONE_LOCATION_PLACED))]
    pub placed: Option<Bool>,
}

#[binrw]
#[brw(little)]
#[br(import(version: i32))]
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct RandEventData {
    pub event_timer: f32,
    #[br(if(version >= world::RAND_EVENT_DETAILS))]
    pub current: Option<RandEvent>,
}

#[binrw]
#[brw(little)]
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct RandEvent {
    /// Empty when no event is active.
    pub name: CsString,
    pub time: f32,
    pub position: Vec3,
}

impl WorldDb {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut cursor = Cursor::new(bytes);
        let db = WorldDb::read_le(&mut cursor).context("parsing .db")?;
        let consumed = cursor.position() as usize;
        if consumed != bytes.len() {
            bail!(
                ".db has {} unparsed trailing bytes after offset {consumed}",
                bytes.len() - consumed
            );
        }
        Ok(db)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut cursor = Cursor::new(Vec::new());
        self.write_le(&mut cursor).context("serializing .db")?;
        Ok(cursor.into_inner())
    }

    /// Fill fields absent from older versions with the game's defaults and
    /// stamp the newest version this layout can carry
    /// ([`world::LEGACY_CURRENT`]). The writer follows `self.version`, so this
    /// is also what re-encodes ZDO property counts in the wide form; writing
    /// without it keeps whatever encoding the file was read with. Moving on
    /// to the current version means changing layout: see
    /// [`super::world::WorldData::upgrade`].
    pub fn upgrade(&mut self) {
        self.version = world::LEGACY_CURRENT;
        self.net_time.get_or_insert(0.0);
        let zone = self.zone_system.get_or_insert_with(ZoneSystemData::default);
        zone.pgw_version.get_or_insert(0);
        zone.location_version.get_or_insert(0);
        zone.global_keys.get_or_insert_with(StringList::default);
        zone.locations_generated.get_or_insert(Bool(false));
        let locations = zone.locations.get_or_insert_with(LocationList::default);
        for loc in &mut locations.locations {
            loc.placed.get_or_insert(Bool(false));
        }
        let events = self.rand_events.get_or_insert_with(RandEventData::default);
        events.current.get_or_insert_with(RandEvent::default);
    }
}
