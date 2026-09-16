//! A single ZDO (network object) in the world version 31+ inline format
//! (`ZDO.Load` / `ZDO.Save`).
//!
//! Layout: `u16` flags, then up to version 39 the sector, then the position,
//! prefab hash, optional rotation, then one property table per flagged value
//! type. Each table is a `NumItems` count followed by `(i32 key hash, value)`
//! pairs. From [`world::ZDO_COMPACT`] the sector is gone (the game derives it
//! from the position), a position with no height and whole-number
//! coordinates is two shorts flagged `SMALL_POSITION`, and the rotation is
//! packed into two or four bytes of half-degree steps.

use binrw::io::{Read, Seek, Write};
use binrw::{BinRead, BinResult, BinWrite, Endian, binrw};
use serde::Serialize;

use super::primitives::{
    ByteArray, CsString, Quat, Vec2s, Vec3, read_num_items, read_small_rotation, write_num_items,
    write_small_rotation,
};
use super::versions::world;

pub mod flags {
    pub const CONNECTION: u16 = 0x0001;
    pub const FLOATS: u16 = 0x0002;
    pub const VEC3S: u16 = 0x0004;
    pub const QUATS: u16 = 0x0008;
    pub const INTS: u16 = 0x0010;
    pub const LONGS: u16 = 0x0020;
    pub const STRINGS: u16 = 0x0040;
    pub const BYTE_ARRAYS: u16 = 0x0080;
    pub const PERSISTENT: u16 = 0x0100;
    pub const DISTANT: u16 = 0x0200;
    pub const TYPE_SHIFT: u16 = 10;
    pub const TYPE_MASK: u16 = 0x3;
    pub const ROTATION: u16 = 0x1000;
    /// The position is stored as two shorts (`Utils.SmallPosition`).
    pub const SMALL_POSITION: u16 = 0x2000;
}

/// `ZDOExtraData.ConnectionType` plus optional `Target` bit (0x10).
///
/// `hash` pairs the two sides of one connection within a single save; it is
/// regenerated on every save (`ZDOExtraData.RegenerateConnectionHashData`)
/// and carries no meaning across files, so nothing here judges it.
#[binrw]
#[brw(little)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Connection {
    pub kind: u8,
    pub hash: i32,
}

impl Connection {
    /// Set on the object a connection points *at* rather than the one that
    /// established it.
    pub const TARGET: u8 = 0x10;

    /// The connection type with the `Target` bit masked off.
    pub fn base_kind(self) -> u8 {
        self.kind & !Self::TARGET
    }

    pub fn is_target(self) -> bool {
        self.kind & Self::TARGET != 0
    }

    /// `ZDOHelper.ToStringFast`, or `None` for a type this tool does not know.
    pub fn type_name(self) -> Option<&'static str> {
        match self.base_kind() {
            1 => Some("Portal"),
            2 => Some("SyncTransform"),
            3 => Some("Spawned"),
            _ => None,
        }
    }

    /// How the connection is shown in reports: `Portal`, `Portal|Target`, or
    /// `#<kind>` when the type is unknown.
    pub fn label(self) -> String {
        let base = match self.type_name() {
            Some(name) => name.to_owned(),
            None => format!("#{}", self.base_kind()),
        };
        if self.is_target() {
            format!("{base}|Target")
        } else {
            base
        }
    }
}

/// Property table for one value type: `(key hash, value)` pairs in file order.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PropTable<T> {
    pub entries: Vec<(i32, T)>,
}

impl<T> Default for PropTable<T> {
    fn default() -> Self {
        PropTable {
            entries: Vec::new(),
        }
    }
}

impl<T> PropTable<T> {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn get(&self, key: i32) -> Option<&T> {
        self.entries.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
    }

    pub fn contains(&self, key: i32) -> bool {
        self.entries.iter().any(|(k, _)| *k == key)
    }
}

impl<T> BinRead for PropTable<T>
where
    T: for<'a> BinRead<Args<'a> = ()>,
{
    /// `(world version,)`: selects the count encoding.
    type Args<'a> = (i32,);

    fn read_options<R: Read + Seek>(
        reader: &mut R,
        _: Endian,
        (version,): (i32,),
    ) -> BinResult<Self> {
        let count = read_num_items(reader, version >= world::ZDO_WIDE_COUNTS)?;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let key = i32::read_le(reader)?;
            let value = T::read_le(reader)?;
            entries.push((key, value));
        }
        Ok(PropTable { entries })
    }
}

impl<T> BinWrite for PropTable<T>
where
    T: for<'a> BinWrite<Args<'a> = ()>,
{
    /// `(world version,)`: selects the count encoding, as when reading.
    type Args<'a> = (i32,);

    fn write_options<W: Write + Seek>(
        &self,
        writer: &mut W,
        _: Endian,
        (version,): (i32,),
    ) -> BinResult<()> {
        if self.entries.is_empty() {
            return Ok(());
        }
        write_num_items(
            writer,
            self.entries.len(),
            version >= world::ZDO_WIDE_COUNTS,
        )?;
        for (key, value) in &self.entries {
            key.write_le(writer)?;
            value.write_le(writer)?;
        }
        Ok(())
    }
}

#[binrw]
#[brw(little)]
#[br(import(version: i32))]
#[bw(import(version: i32))]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Zdo {
    #[br(temp)]
    #[bw(calc = pack_flags(version, persistent, distant, object_type, position, rotation, connection, floats, vec3s, quats, ints, longs, strings, byte_arrays))]
    flags: u16,
    #[br(calc = flags & flags::PERSISTENT != 0)]
    #[bw(ignore)]
    pub persistent: bool,
    #[br(calc = flags & flags::DISTANT != 0)]
    #[bw(ignore)]
    pub distant: bool,
    /// `ZDO.ObjectType`: 0 Default, 1 Prioritized, 2 Solid, 3 Terrain.
    #[br(calc = ((flags >> flags::TYPE_SHIFT) & flags::TYPE_MASK) as u8)]
    #[bw(ignore)]
    pub object_type: u8,
    /// The zone the object was in when saved. Only the single-file layout
    /// stores it; the chunked layout derives it from the position, so it is
    /// `None` for anything read from a `.chunk`. Writing in the old layout
    /// computes it from the position when it is missing.
    #[br(if(version < world::ZDO_COMPACT))]
    #[bw(write_with = write_sector, args(version, position))]
    pub sector: Option<Vec2s>,
    #[br(temp, if(flags & flags::SMALL_POSITION != 0))]
    #[bw(ignore)]
    small_position: Option<Vec2s>,
    #[br(if(small_position.is_none(), small_position.map(Vec2s::expand).unwrap_or_default()))]
    #[bw(write_with = write_position, args(version))]
    pub position: Vec3,
    pub prefab: i32,
    /// Euler angles; absent when the rotation is zero.
    #[br(if(flags & flags::ROTATION != 0), parse_with = read_rotation, args(version))]
    #[bw(write_with = write_rotation, args(version))]
    pub rotation: Option<Vec3>,
    #[br(if(flags & flags::CONNECTION != 0))]
    pub connection: Option<Connection>,
    #[br(if(flags & flags::FLOATS != 0), args(version))]
    #[bw(args(version))]
    pub floats: PropTable<f32>,
    #[br(if(flags & flags::VEC3S != 0), args(version))]
    #[bw(args(version))]
    pub vec3s: PropTable<Vec3>,
    #[br(if(flags & flags::QUATS != 0), args(version))]
    #[bw(args(version))]
    pub quats: PropTable<Quat>,
    #[br(if(flags & flags::INTS != 0), args(version))]
    #[bw(args(version))]
    pub ints: PropTable<i32>,
    #[br(if(flags & flags::LONGS != 0), args(version))]
    #[bw(args(version))]
    pub longs: PropTable<i64>,
    #[br(if(flags & flags::STRINGS != 0), args(version))]
    #[bw(args(version))]
    pub strings: PropTable<CsString>,
    #[br(if(flags & flags::BYTE_ARRAYS != 0), args(version))]
    #[bw(args(version))]
    pub byte_arrays: PropTable<ByteArray>,
}

/// Whether a ZDO written for `version` stores its position as two shorts.
fn stores_small_position(version: i32, position: &Vec3) -> bool {
    version >= world::ZDO_COMPACT && position.small().is_some()
}

#[binrw::writer(writer, endian)]
fn write_sector(sector: &Option<Vec2s>, version: i32, position: &Vec3) -> BinResult<()> {
    if version < world::ZDO_COMPACT {
        sector
            .unwrap_or_else(|| position.zone())
            .write_options(writer, endian, ())?;
    }
    Ok(())
}

#[binrw::writer(writer, endian)]
fn write_position(position: &Vec3, version: i32) -> BinResult<()> {
    match position.small().filter(|_| version >= world::ZDO_COMPACT) {
        Some(small) => small.write_options(writer, endian, ()),
        None => position.write_options(writer, endian, ()),
    }
}

#[binrw::parser(reader, endian)]
fn read_rotation(version: i32) -> BinResult<Option<Vec3>> {
    if version >= world::ZDO_COMPACT {
        read_small_rotation(reader).map(Some)
    } else {
        Vec3::read_options(reader, endian, ()).map(Some)
    }
}

#[binrw::writer(writer, endian)]
fn write_rotation(rotation: &Option<Vec3>, version: i32) -> BinResult<()> {
    match rotation {
        Some(r) if version >= world::ZDO_COMPACT => write_small_rotation(writer, r),
        Some(r) => r.write_options(writer, endian, ()),
        None => Ok(()),
    }
}

#[allow(clippy::too_many_arguments)]
fn pack_flags(
    version: i32,
    persistent: &bool,
    distant: &bool,
    object_type: &u8,
    position: &Vec3,
    rotation: &Option<Vec3>,
    connection: &Option<Connection>,
    floats: &PropTable<f32>,
    vec3s: &PropTable<Vec3>,
    quats: &PropTable<Quat>,
    ints: &PropTable<i32>,
    longs: &PropTable<i64>,
    strings: &PropTable<CsString>,
    byte_arrays: &PropTable<ByteArray>,
) -> u16 {
    let mut f = 0u16;
    if connection.is_some() {
        f |= flags::CONNECTION;
    }
    if !floats.is_empty() {
        f |= flags::FLOATS;
    }
    if !vec3s.is_empty() {
        f |= flags::VEC3S;
    }
    if !quats.is_empty() {
        f |= flags::QUATS;
    }
    if !ints.is_empty() {
        f |= flags::INTS;
    }
    if !longs.is_empty() {
        f |= flags::LONGS;
    }
    if !strings.is_empty() {
        f |= flags::STRINGS;
    }
    if !byte_arrays.is_empty() {
        f |= flags::BYTE_ARRAYS;
    }
    if *persistent {
        f |= flags::PERSISTENT;
    }
    if *distant {
        f |= flags::DISTANT;
    }
    f |= (u16::from(*object_type) & flags::TYPE_MASK) << flags::TYPE_SHIFT;
    if rotation.is_some() {
        f |= flags::ROTATION;
    }
    if stores_small_position(version, position) {
        f |= flags::SMALL_POSITION;
    }
    f
}

/// Which property table a key lives in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub enum PropKind {
    Float,
    Vec3,
    Quat,
    Int,
    Long,
    String,
    ByteArray,
}

impl PropKind {
    pub fn label(self) -> &'static str {
        match self {
            PropKind::Float => "float",
            PropKind::Vec3 => "vec3",
            PropKind::Quat => "quat",
            PropKind::Int => "int",
            PropKind::Long => "long",
            PropKind::String => "string",
            PropKind::ByteArray => "bytes",
        }
    }
}

/// Borrowed view of one property value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PropValue<'a> {
    Float(f32),
    Vec3(&'a Vec3),
    Quat(&'a Quat),
    Int(i32),
    Long(i64),
    String(&'a str),
    ByteArray(&'a [u8]),
}

impl Zdo {
    /// The zone the object belongs to: the stored sector when the file had
    /// one, otherwise the zone of the position, which is what the chunked
    /// layout uses (`ZDO.GetSector`).
    pub fn zone(&self) -> Vec2s {
        self.sector.unwrap_or_else(|| self.position.zone())
    }

    /// Reduce the object to what the compact record can carry, so that the
    /// struct matches what the game will read back: the stored sector goes
    /// (the chunked layout derives it), a rotation under half a degree goes
    /// (`ZDO.Save` omits it), and any other rotation is rounded to the half
    /// degrees the encoding has. Applied when a world moves to the chunked
    /// layout.
    pub fn compact(&mut self) {
        self.sector = None;
        self.rotation = self
            .rotation
            .filter(|r| !r.close_to_zero())
            .map(|r| r.quantized_rotation());
    }

    pub fn string(&self, key: i32) -> Option<&str> {
        self.strings.get(key).map(CsString::as_str)
    }

    pub fn int(&self, key: i32) -> Option<i32> {
        self.ints.get(key).copied()
    }

    pub fn long(&self, key: i32) -> Option<i64> {
        self.longs.get(key).copied()
    }

    pub fn float(&self, key: i32) -> Option<f32> {
        self.floats.get(key).copied()
    }

    pub fn bytes(&self, key: i32) -> Option<&[u8]> {
        self.byte_arrays.get(key).map(|b| b.data.as_slice())
    }

    pub fn has_key(&self, key: i32) -> bool {
        self.floats.contains(key)
            || self.vec3s.contains(key)
            || self.quats.contains(key)
            || self.ints.contains(key)
            || self.longs.contains(key)
            || self.strings.contains(key)
            || self.byte_arrays.contains(key)
    }

    pub fn property_count(&self) -> usize {
        self.floats.len()
            + self.vec3s.len()
            + self.quats.len()
            + self.ints.len()
            + self.longs.len()
            + self.strings.len()
            + self.byte_arrays.len()
    }

    /// Every property as `(kind, key, value)`, table by table.
    pub fn properties(&self) -> impl Iterator<Item = (PropKind, i32, PropValue<'_>)> {
        let f = self
            .floats
            .entries
            .iter()
            .map(|(k, v)| (PropKind::Float, *k, PropValue::Float(*v)));
        let v3 = self
            .vec3s
            .entries
            .iter()
            .map(|(k, v)| (PropKind::Vec3, *k, PropValue::Vec3(v)));
        let q = self
            .quats
            .entries
            .iter()
            .map(|(k, v)| (PropKind::Quat, *k, PropValue::Quat(v)));
        let i = self
            .ints
            .entries
            .iter()
            .map(|(k, v)| (PropKind::Int, *k, PropValue::Int(*v)));
        let l = self
            .longs
            .entries
            .iter()
            .map(|(k, v)| (PropKind::Long, *k, PropValue::Long(*v)));
        let s = self
            .strings
            .entries
            .iter()
            .map(|(k, v)| (PropKind::String, *k, PropValue::String(v.as_str())));
        let b = self.byte_arrays.entries.iter().map(|(k, v)| {
            (
                PropKind::ByteArray,
                *k,
                PropValue::ByteArray(v.data.as_slice()),
            )
        });
        f.chain(v3).chain(q).chain(i).chain(l).chain(s).chain(b)
    }

    /// Keep only properties for which `keep` returns true. Returns how many
    /// were removed.
    pub fn retain_properties(
        &mut self,
        mut keep: impl FnMut(PropKind, i32, PropValue<'_>) -> bool,
    ) -> usize {
        let before = self.property_count();
        self.floats
            .entries
            .retain(|(k, v)| keep(PropKind::Float, *k, PropValue::Float(*v)));
        self.vec3s
            .entries
            .retain(|(k, v)| keep(PropKind::Vec3, *k, PropValue::Vec3(v)));
        self.quats
            .entries
            .retain(|(k, v)| keep(PropKind::Quat, *k, PropValue::Quat(v)));
        self.ints
            .entries
            .retain(|(k, v)| keep(PropKind::Int, *k, PropValue::Int(*v)));
        self.longs
            .entries
            .retain(|(k, v)| keep(PropKind::Long, *k, PropValue::Long(*v)));
        self.strings
            .entries
            .retain(|(k, v)| keep(PropKind::String, *k, PropValue::String(v.as_str())));
        self.byte_arrays
            .entries
            .retain(|(k, v)| keep(PropKind::ByteArray, *k, PropValue::ByteArray(&v.data)));
        before - self.property_count()
    }
}
