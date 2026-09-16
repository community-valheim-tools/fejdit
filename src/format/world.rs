//! A world's data in either layout, so commands need not care which.

use serde::Serialize;

use super::chunked::ChunkedDb;
use super::db::{RandEvent, WorldDb};
use super::versions::world;
use super::zdo::Zdo;

/// The objects and world state of one save, however it is stored.
#[derive(Clone, Debug, PartialEq)]
pub enum WorldData {
    /// A `.db` beside a `.fwl` (world versions up to 39).
    Db(WorldDb),
    /// A directory of `_main.<N>.*` and `.chunk` files (version 40 up).
    Chunked(ChunkedDb),
}

/// The zone-system figures `world info` reports, common to both layouts.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ZoneSummary {
    pub generated_zones: usize,
    pub global_keys: Vec<String>,
    pub locations: usize,
    pub locations_placed: usize,
}

impl WorldData {
    pub fn version(&self) -> i32 {
        match self {
            WorldData::Db(db) => db.version,
            WorldData::Chunked(c) => c.db2.version,
        }
    }

    pub fn is_chunked(&self) -> bool {
        matches!(self, WorldData::Chunked(_))
    }

    pub fn net_time(&self) -> Option<f64> {
        match self {
            WorldData::Db(db) => db.net_time,
            WorldData::Chunked(c) => Some(c.db2.net_time),
        }
    }

    /// The saving session's id and next object id. The chunked layout does
    /// not store them: every load hands out fresh ids.
    pub fn session(&self) -> Option<(i64, u32)> {
        match self {
            WorldData::Db(db) => Some((db.zdos.session_id, db.zdos.next_uid)),
            WorldData::Chunked(_) => None,
        }
    }

    pub fn zdo_count(&self) -> usize {
        match self {
            WorldData::Db(db) => db.zdos.zdos.len(),
            WorldData::Chunked(c) => c.zdo_count(),
        }
    }

    pub fn zdos(&self) -> Box<dyn Iterator<Item = &Zdo> + '_> {
        match self {
            WorldData::Db(db) => Box::new(db.zdos.zdos.iter()),
            WorldData::Chunked(c) => Box::new(c.zdos()),
        }
    }

    pub fn zdos_mut(&mut self) -> Box<dyn Iterator<Item = &mut Zdo> + '_> {
        match self {
            WorldData::Db(db) => Box::new(db.zdos.zdos.iter_mut()),
            WorldData::Chunked(c) => Box::new(c.zdos_mut()),
        }
    }

    pub fn zone_summary(&self) -> ZoneSummary {
        match self {
            WorldData::Db(db) => {
                let zone = db.zone_system.as_ref();
                let locations = zone.and_then(|z| z.locations.as_ref());
                ZoneSummary {
                    generated_zones: zone.map_or(0, |z| z.generated_zones.len()),
                    global_keys: zone
                        .and_then(|z| z.global_keys.as_ref())
                        .map(|k| k.items.iter().map(|s| s.0.clone()).collect())
                        .unwrap_or_default(),
                    locations: locations.map_or(0, |l| l.locations.len()),
                    locations_placed: locations.map_or(0, |l| {
                        l.locations
                            .iter()
                            .filter(|x| x.placed.is_some_and(|p| p.0))
                            .count()
                    }),
                }
            }
            WorldData::Chunked(c) => {
                let zone = &c.db2.zone_system.0;
                ZoneSummary {
                    generated_zones: zone.generated_zones.len(),
                    global_keys: zone.global_keys.items.iter().map(|s| s.0.clone()).collect(),
                    locations: zone.locations.len(),
                    locations_placed: zone.locations.iter().filter(|l| l.placed.0).count(),
                }
            }
        }
    }

    /// The random event in progress, if any.
    pub fn random_event(&self) -> Option<&RandEvent> {
        let current = match self {
            WorldData::Db(db) => db.rand_events.as_ref()?.current.as_ref()?,
            WorldData::Chunked(c) => c.db2.rand_events.current.as_ref()?,
        };
        (!current.name.0.is_empty()).then_some(current)
    }

    /// Bring the world to the current version, which for a single-file
    /// world means the chunked layout: the objects are migrated and dealt
    /// into chunks as the game does the first time it saves an old world.
    /// A chunked world is already there.
    pub fn upgrade(&mut self) {
        if let WorldData::Db(db) = self {
            *self = WorldData::Chunked(ChunkedDb::from_legacy(db));
        }
    }

    /// Whether [`WorldData::upgrade`] would change anything.
    pub fn is_current(&self) -> bool {
        self.version() >= world::CURRENT && self.is_chunked()
    }
}
