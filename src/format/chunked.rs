//! The chunked world layout of game 1.0 (`Version.World.ChunkedSave`, 40+).
//!
//! `<worlds>/<name>/` holds, for the current save number `N`:
//!
//! * `_main.N.fwl2`: the metadata, same payload as a `.fwl` ([`super::fwl`]).
//! * `_main.N.db2`: everything from the old `.db` that is not an object
//!   ([`Db2`]).
//! * `_main.N.chunks`: which `.chunk` files make up the world and how many
//!   objects each holds ([`ChunkMap`]).
//! * `_main.N.ok`: written last, so its presence means the save completed.
//! * `<yy>_<xx>__<size>_<version>.chunk`: the objects of one chunk of the
//!   map ([`ChunkFile`]). The map is 512 x 512 zones; a size-0 chunk is 8 x 8
//!   zones and each size up doubles that. Portals live in a chunk of their
//!   own so the game can connect them before anything else loads.
//!
//! Every save bumps `N` and the version of each chunk it rewrote, then
//! deletes the previous save's files (`ZNet.SaveWorldThread`).

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use binrw::io::Cursor;
use binrw::{BinRead, BinWrite, binrw};
use serde::Serialize;

use super::compress::Gzipped;
use super::db::{RandEventData, WorldDb};
use super::migrate;
use super::primitives::{Bool, ByteArray, StringList, Vec2s, Vec3};
use super::versions::{CHUNK_VERSION, world};
use super::zdo::Zdo;
use crate::hash::stable_hash;

/// Prefabs `Game.m_portalPrefabs` lists; their objects are kept apart in
/// [`ChunkIndex::PORTALS`] (`ZDOMan.LoadChunks`, `GetSaveClonePerChunk`).
pub const PORTAL_PREFABS: &[&str] = &["portal", "portal_wood", "portal_stone"];

/// Zones per side of a size-0 chunk (`ZoneSystem.c_ZonesPerChunk`).
pub const ZONES_PER_CHUNK: u32 = 8;
/// Zones per side of the whole map (`ZoneSystem.SectorToIndex`).
pub const ZONES_PER_SIDE: u32 = 512;
/// Size-0 chunks per side of the map.
pub const CHUNKS_PER_SIDE: u32 = ZONES_PER_SIDE / ZONES_PER_CHUNK;
/// Largest chunk size (`ZoneSystem.c_MaxChunkSize`).
pub const MAX_CHUNK_SIZE: u8 = 3;
/// A merged chunk may not hold this many objects
/// (`ZDOMan.c_MaxNumberOfZDOsPerBiggerChunks`).
pub const MAX_ZDOS_PER_MERGED_CHUNK: i32 = 100_000;

// ---------------------------------------------------------------- _main.N.db2

/// `_main.N.db2` (`ZNet.SaveWorldThread` / `ZNet.LoadWorld`).
#[binrw]
#[brw(little)]
#[derive(Clone, Debug, PartialEq)]
pub struct Db2 {
    #[br(assert(
        version >= world::CHUNKED_SAVE,
        "a .db2 carries world version {} but the chunked layout starts at {}",
        version, world::CHUNKED_SAVE
    ))]
    pub version: i32,
    pub net_time: f64,
    pub zone_system: Gzipped<ZoneSystemPacked>,
    #[br(args(version))]
    pub rand_events: RandEventData,
    /// `PersistentEventSystem` state: Brotli-compressed JSON, kept opaque.
    pub persistent_events: ByteArray,
}

/// `ZoneSystem.Save` / `ZoneSystem.Load`, the content of the gzip section.
/// Like the old section minus the always-zero `pgw_version`, with zones as
/// shorts and locations by prefab hash.
#[binrw]
#[brw(little)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ZoneSystemPacked {
    #[br(temp)]
    #[bw(calc = generated_zones.len() as i32)]
    zone_count: i32,
    #[br(count = zone_count)]
    pub generated_zones: Vec<Vec2s>,
    pub location_version: i32,
    pub global_keys: StringList,
    pub locations_generated: Bool,
    #[br(temp)]
    #[bw(calc = locations.len() as i32)]
    location_count: i32,
    #[br(count = location_count)]
    pub locations: Vec<PackedLocation>,
}

#[binrw]
#[brw(little)]
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct PackedLocation {
    pub prefab_hash: i32,
    pub position: Vec3,
    pub placed: Bool,
}

impl Db2 {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut cursor = Cursor::new(bytes);
        let db = Db2::read_le(&mut cursor).context("parsing .db2")?;
        let consumed = cursor.position() as usize;
        if consumed != bytes.len() {
            bail!(
                ".db2 has {} unparsed trailing bytes after offset {consumed}",
                bytes.len() - consumed
            );
        }
        Ok(db)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut cursor = Cursor::new(Vec::new());
        self.write_le(&mut cursor).context("serializing .db2")?;
        Ok(cursor.into_inner())
    }
}

// ---------------------------------------------------------------- _main.N.chunks

/// `_main.N.chunks` (`ChunkSaveMapping.Save` / `.Load`).
#[binrw]
#[brw(little)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChunkMap {
    pub version: u16,
    /// Objects across every chunk; the game reads it to size its lists.
    pub total_zdos: i32,
    #[br(temp)]
    #[bw(calc = chunks.len() as i32)]
    count: i32,
    #[br(count = count)]
    pub chunks: Vec<ChunkInfo>,
}

#[binrw]
#[brw(little)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ChunkInfo {
    pub index: ChunkIndex,
    /// Bumped on every save that rewrites the chunk; part of the file name,
    /// so a rewrite never overwrites the previous save's file.
    pub version: u32,
    pub num_zdos: i32,
}

/// `ZoneSystem.ChunkIndex`: the low byte is the x coordinate and the high
/// byte the y coordinate of the chunk on the 64 x 64 grid of size-0 chunks,
/// and `size` says how many of those it merges (`2^size` per side). A bigger
/// chunk's coordinate is the grid position of its first size-0 chunk, so its
/// low `size` bits are zero on both axes.
#[binrw]
#[brw(little)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize)]
pub struct ChunkIndex {
    pub chunk: u16,
    pub size: u8,
}

impl ChunkIndex {
    /// `ZoneSystem.ChunkPortal`: where every portal is kept. It shares a
    /// number with the size-0 chunk at grid (1, 0), a corner of the map far
    /// outside any playable area.
    pub const PORTALS: ChunkIndex = ChunkIndex { chunk: 1, size: 0 };

    pub fn from_xy(x: u32, y: u32, size: u8) -> Self {
        ChunkIndex {
            chunk: (x + (y << 8)) as u16,
            size,
        }
    }

    pub fn x(self) -> u32 {
        u32::from(self.chunk & 0xFF)
    }

    pub fn y(self) -> u32 {
        u32::from(self.chunk >> 8)
    }

    /// Size-0 chunks per side.
    pub fn span(self) -> u32 {
        1 << self.size
    }

    /// The zone at the chunk's lower corner (`ZoneSystem.GetZoneFromChunk`).
    pub fn zone_origin(self) -> (i32, i32) {
        let half = (ZONES_PER_SIDE / 2) as i32;
        (
            (self.x() * ZONES_PER_CHUNK) as i32 - half,
            (self.y() * ZONES_PER_CHUNK) as i32 - half,
        )
    }

    /// `ChunkSaveMapping.GetChunkFilename`.
    pub fn file_name(self, version: u32) -> String {
        format!(
            "{:02x}_{:02x}__{}_{}.chunk",
            self.y(),
            self.x(),
            self.size,
            version
        )
    }
}

impl ChunkInfo {
    pub fn file_name(&self) -> String {
        self.index.file_name(self.version)
    }
}

impl ChunkMap {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut cursor = Cursor::new(bytes);
        let map = ChunkMap::read_le(&mut cursor).context("parsing .chunks")?;
        let consumed = cursor.position() as usize;
        if consumed != bytes.len() {
            bail!(
                ".chunks has {} unparsed trailing bytes",
                bytes.len() - consumed
            );
        }
        Ok(map)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut cursor = Cursor::new(Vec::new());
        self.write_le(&mut cursor).context("serializing .chunks")?;
        Ok(cursor.into_inner())
    }
}

// ---------------------------------------------------------------- *.chunk

/// One `.chunk` file (`ZDOMan.SaveChunk` / `LoadChunks`). The version is the
/// file's own: the game reads each chunk's objects at the version its header
/// gives, so a chunk written by an older build stays readable next to newer
/// ones.
#[binrw]
#[brw(little)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChunkFile {
    pub version: i16,
    #[br(temp)]
    #[bw(calc = zdos.len() as i32)]
    count: i32,
    #[br(count = count, args { inner: (i32::from(version),) })]
    #[bw(args(i32::from(*version)))]
    pub zdos: Vec<Zdo>,
}

impl ChunkFile {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut cursor = Cursor::new(bytes);
        let file = ChunkFile::read_le(&mut cursor).context("parsing chunk")?;
        let consumed = cursor.position() as usize;
        if consumed != bytes.len() {
            bail!(
                "chunk has {} unparsed trailing bytes after offset {consumed}",
                bytes.len() - consumed
            );
        }
        Ok(file)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut cursor = Cursor::new(Vec::new());
        self.write_le(&mut cursor).context("serializing chunk")?;
        Ok(cursor.into_inner())
    }
}

/// `_main.N.ok`: one `i32` world version, written after everything else.
#[binrw]
#[brw(little)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OkMarker {
    pub version: i32,
}

// ---------------------------------------------------------------- the whole directory

/// Everything in a chunked world directory except the metadata, in memory.
#[derive(Clone, Debug, PartialEq)]
pub struct ChunkedDb {
    pub db2: Db2,
    /// The version the `.chunks` map was written with.
    pub map_version: u16,
    /// In `.chunks` order, which is the order the game loads them in.
    pub chunks: Vec<Chunk>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Chunk {
    pub info: ChunkInfo,
    pub file: ChunkFile,
}

/// `_main.<N>.<ext>`.
pub fn main_file_name(save_number: u32, ext: &str) -> String {
    format!("_main.{save_number}.{ext}")
}

impl ChunkedDb {
    /// Read save number `save_number` from `dir`.
    pub fn read_dir(dir: &Path, save_number: u32) -> Result<Self> {
        let read = |name: String| {
            let path = dir.join(&name);
            std::fs::read(&path).with_context(|| format!("reading {}", path.display()))
        };
        let db2_bytes = read(main_file_name(save_number, "db2"))?;
        let db2 = Db2::parse(&db2_bytes)
            .with_context(|| format!("parsing {}", main_file_name(save_number, "db2")))?;
        let map_bytes = read(main_file_name(save_number, "chunks"))?;
        let map = ChunkMap::parse(&map_bytes)
            .with_context(|| format!("parsing {}", main_file_name(save_number, "chunks")))?;
        let mut chunks = Vec::with_capacity(map.chunks.len());
        for info in map.chunks {
            let name = info.file_name();
            let bytes = read(name.clone())?;
            let file = ChunkFile::parse(&bytes).with_context(|| format!("parsing {name}"))?;
            if file.zdos.len() != info.num_zdos as usize {
                bail!(
                    "{name} holds {} objects but the chunk map says {}",
                    file.zdos.len(),
                    info.num_zdos
                );
            }
            chunks.push(Chunk { info, file });
        }
        Ok(ChunkedDb {
            db2,
            map_version: map.version,
            chunks,
        })
    }

    pub fn zdo_count(&self) -> usize {
        self.chunks.iter().map(|c| c.file.zdos.len()).sum()
    }

    pub fn zdos(&self) -> impl Iterator<Item = &Zdo> {
        self.chunks.iter().flat_map(|c| c.file.zdos.iter())
    }

    pub fn zdos_mut(&mut self) -> impl Iterator<Item = &mut Zdo> {
        self.chunks.iter_mut().flat_map(|c| c.file.zdos.iter_mut())
    }

    /// The `.chunks` map describing the chunks held here, counts included.
    pub fn chunk_map(&self) -> ChunkMap {
        let chunks: Vec<ChunkInfo> = self
            .chunks
            .iter()
            .map(|c| ChunkInfo {
                num_zdos: c.file.zdos.len() as i32,
                ..c.info
            })
            .collect();
        ChunkMap {
            version: self.map_version,
            total_zdos: chunks.iter().map(|c| c.num_zdos).sum(),
            chunks,
        }
    }

    /// Every file of the directory except the metadata, as
    /// `(file name, bytes)`, for save number `save_number`.
    pub fn files(&self, save_number: u32) -> Result<Vec<(String, Vec<u8>)>> {
        let mut files = vec![
            (main_file_name(save_number, "db2"), self.db2.to_bytes()?),
            (
                main_file_name(save_number, "chunks"),
                self.chunk_map().to_bytes()?,
            ),
        ];
        for chunk in &self.chunks {
            files.push((chunk.info.file_name(), chunk.file.to_bytes()?));
        }
        let mut ok = Cursor::new(Vec::new());
        OkMarker {
            version: self.db2.version,
        }
        .write_le(&mut ok)?;
        files.push((main_file_name(save_number, "ok"), ok.into_inner()));
        Ok(files)
    }

    /// What 1.0 makes of a single-file world the first time it saves it:
    /// the objects are migrated ([`migrate`]), compacted, and dealt into
    /// chunks the way `ZDOMan.GetSaveClonePerChunk` deals a freshly loaded
    /// world (nothing merged yet, everything dirty), and the rest of the
    /// `.db` moves into the `.db2`.
    pub fn from_legacy(db: &WorldDb) -> Self {
        let source_version = db.version;
        let mut zdos = db.zdos.zdos.clone();
        if migrate::needs_migration(source_version) {
            for zdo in &mut zdos {
                migrate::migrate_zdo(zdo, source_version);
            }
        }
        for zdo in &mut zdos {
            zdo.compact();
        }
        let chunks = partition(&zdos)
            .into_iter()
            .map(|(index, members)| Chunk {
                info: ChunkInfo {
                    index,
                    version: 1,
                    num_zdos: members.len() as i32,
                },
                file: ChunkFile {
                    version: CHUNK_VERSION as i16,
                    zdos: members.into_iter().map(|i| zdos[i].clone()).collect(),
                },
            })
            .collect();
        let zone = db.zone_system.clone().unwrap_or_default();
        let zone_system = ZoneSystemPacked {
            generated_zones: zone
                .generated_zones
                .iter()
                .map(|z| Vec2s {
                    x: z.x as i16,
                    y: z.y as i16,
                })
                .collect(),
            location_version: zone.location_version.unwrap_or(0),
            global_keys: zone.global_keys.unwrap_or_default(),
            locations_generated: zone.locations_generated.unwrap_or(Bool(false)),
            locations: zone
                .locations
                .map(|l| l.locations)
                .unwrap_or_default()
                .iter()
                .map(|l| PackedLocation {
                    prefab_hash: l.prefab_hash.unwrap_or_else(|| {
                        stable_hash(l.prefab_name.as_ref().map_or("", |n| n.as_str()))
                    }),
                    position: l.position,
                    placed: l.placed.unwrap_or(Bool(false)),
                })
                .collect(),
        };
        let mut rand_events = db.rand_events.clone().unwrap_or_default();
        rand_events.current.get_or_insert_with(Default::default);
        ChunkedDb {
            db2: Db2 {
                version: world::CURRENT,
                net_time: db.net_time.unwrap_or(0.0),
                zone_system: Gzipped(zone_system),
                rand_events,
                persistent_events: db.persistent_events.clone().unwrap_or_default(),
            },
            map_version: CHUNK_VERSION as u16,
            chunks,
        }
    }
}

// ---------------------------------------------------------------- dealing objects into chunks

/// `ZoneSystem.GetSectorIndex` for a position: the zone's index on the
/// 512 x 512 grid, or 0 for anything off the grid. Single precision, unlike
/// `GetZone`, which is what `Zdo::zone` mirrors.
pub fn sector_index(position: &Vec3) -> u32 {
    let x = ((position.x + 32.0) / 64.0).floor() as i32;
    let y = ((position.z + 32.0) / 64.0).floor() as i32;
    sector_to_index(x, y)
}

/// `ZoneSystem.SectorToIndex`.
pub fn sector_to_index(zone_x: i32, zone_y: i32) -> u32 {
    let half = (ZONES_PER_SIDE / 2) as i32;
    let x = (zone_x + half) as u32;
    let y = (zone_y + half) as u32;
    if x >= ZONES_PER_SIDE || y >= ZONES_PER_SIDE {
        0
    } else {
        y * ZONES_PER_SIDE + x
    }
}

/// Deal `zdos` (given by index) into chunks as `ZDOMan.GetSaveClonePerChunk`
/// does for a world with no chunk map yet: count objects per size-0 chunk,
/// merge 2 x 2 blocks upward while a merged chunk stays under
/// [`MAX_ZDOS_PER_MERGED_CHUNK`] (never the first row or column of a level,
/// which the game skips), then list size 0 first and the portal chunk last.
/// Within a chunk, objects come zone by zone, rows first, in file order.
fn partition(zdos: &[Zdo]) -> Vec<(ChunkIndex, Vec<usize>)> {
    let portal_hashes: Vec<i32> = PORTAL_PREFABS.iter().map(|p| stable_hash(p)).collect();
    let mut by_sector: HashMap<u32, Vec<usize>> = HashMap::new();
    let mut portals: Vec<(u32, usize)> = Vec::new();
    for (i, zdo) in zdos.iter().enumerate() {
        let sector = sector_index(&zdo.position);
        if portal_hashes.contains(&zdo.prefab) {
            portals.push((sector, i));
        } else {
            by_sector.entry(sector).or_default().push(i);
        }
    }

    let side = CHUNKS_PER_SIDE as usize;
    let mut counts: [Vec<i32>; 4] = [
        vec![0; side * side],
        vec![0; (side / 2) * (side / 2)],
        vec![0; (side / 4) * (side / 4)],
        vec![0; (side / 8) * (side / 8)],
    ];
    for (sector, members) in &by_sector {
        let cx = (sector % ZONES_PER_SIDE / ZONES_PER_CHUNK) as usize;
        let cy = (sector / ZONES_PER_SIDE / ZONES_PER_CHUNK) as usize;
        counts[0][cy * side + cx] += members.len() as i32;
    }
    for level in 0..3 {
        let (fine, coarse) = counts.split_at_mut(level + 1);
        decide_chunk_size(side >> level, &mut fine[level], &mut coarse[0]);
    }

    let mut out = Vec::new();
    for (level, level_counts) in counts.iter().enumerate() {
        let size = side >> level;
        let span = (side / size) as u32;
        for (i, &count) in level_counts.iter().enumerate() {
            if count <= 0 {
                continue;
            }
            let x = (i % size) as u32 * span;
            let y = (i / size) as u32 * span;
            let index = ChunkIndex::from_xy(x, y, level as u8);
            let (zx, zy) = index.zone_origin();
            let zones = (ZONES_PER_CHUNK * span) as i32;
            let mut members = Vec::with_capacity(count as usize);
            for j in zy..zy + zones {
                for k in zx..zx + zones {
                    if let Some(list) = by_sector.get(&sector_to_index(k, j)) {
                        members.extend_from_slice(list);
                    }
                }
            }
            out.push((index, members));
        }
    }
    if !portals.is_empty() {
        // `m_portalObjects` is a map keyed by sector, so portals come out
        // grouped by sector in order of first appearance.
        let mut order: Vec<u32> = Vec::new();
        let mut grouped: HashMap<u32, Vec<usize>> = HashMap::new();
        for (sector, i) in portals {
            if !grouped.contains_key(&sector) {
                order.push(sector);
            }
            grouped.entry(sector).or_default().push(i);
        }
        let members = order
            .iter()
            .flat_map(|s| grouped.remove(s).unwrap_or_default())
            .collect();
        out.push((ChunkIndex::PORTALS, members));
    }
    out
}

/// `ZDOMan.DecideChunkSize` with no previous chunk map: every 2 x 2 block of
/// the finer level that has objects but fewer than the limit, and is not in
/// the first row or column, becomes one chunk of the coarser level.
fn decide_chunk_size(size: usize, fine: &mut [i32], coarse: &mut [i32]) {
    let mut block = 0;
    for by in (0..size).step_by(2) {
        let row = by * size;
        for bx in (0..size).step_by(2) {
            if coarse[block] >= 0 {
                let cells = [row + bx, row + bx + 1, row + size + bx, row + size + bx + 1];
                let sum: i32 = cells.iter().map(|&c| fine[c]).sum();
                if bx != 0 && by != 0 && sum > 0 && sum < MAX_ZDOS_PER_MERGED_CHUNK {
                    coarse[block] = sum;
                    for c in cells {
                        fine[c] = 0;
                    }
                } else {
                    coarse[block] = -100_000_000;
                }
            }
            block += 1;
        }
    }
}
