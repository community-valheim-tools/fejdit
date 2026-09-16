//! Leaf types matching C# `BinaryReader`/`ZPackage` encodings.

use binrw::io::{Read, Seek, Write};
use binrw::{BinRead, BinResult, BinWrite, Endian, binrw};
use serde::Serialize;

/// C# `BinaryWriter.Write(string)`: 7-bit varint byte length, then UTF-8.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct CsString(pub String);

impl CsString {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for CsString {
    fn from(s: &str) -> Self {
        CsString(s.to_owned())
    }
}

impl From<String> for CsString {
    fn from(s: String) -> Self {
        CsString(s)
    }
}

impl std::fmt::Display for CsString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn read_7bit_len<R: Read + Seek>(reader: &mut R) -> BinResult<usize> {
    let mut value: u32 = 0;
    let mut shift = 0;
    loop {
        let byte = u8::read_le(reader)?;
        value |= u32::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return Ok(value as usize);
        }
        shift += 7;
        if shift > 28 {
            return Err(binrw::Error::AssertFail {
                pos: reader.stream_position()?,
                message: "7-bit length prefix longer than 5 bytes".into(),
            });
        }
    }
}

fn write_7bit_len<W: Write + Seek>(writer: &mut W, mut value: usize) -> BinResult<()> {
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        if value == 0 {
            byte.write_le(writer)?;
            return Ok(());
        }
        (byte | 0x80).write_le(writer)?;
    }
}

impl BinRead for CsString {
    type Args<'a> = ();

    fn read_options<R: Read + Seek>(reader: &mut R, _: Endian, _: ()) -> BinResult<Self> {
        let pos = reader.stream_position()?;
        let len = read_7bit_len(reader)?;
        let mut buf = vec![0u8; len];
        reader.read_exact(&mut buf)?;
        String::from_utf8(buf)
            .map(CsString)
            .map_err(|e| binrw::Error::Custom {
                pos,
                err: Box::new(e),
            })
    }
}

impl BinWrite for CsString {
    type Args<'a> = ();

    fn write_options<W: Write + Seek>(&self, writer: &mut W, _: Endian, _: ()) -> BinResult<()> {
        write_7bit_len(writer, self.0.len())?;
        writer.write_all(self.0.as_bytes())?;
        Ok(())
    }
}

/// A `bool` stored as one byte.
#[binrw]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Bool(
    #[br(map = |b: u8| b != 0)]
    #[bw(map = |b: &bool| u8::from(*b))]
    pub bool,
);

/// `i32` length prefix followed by raw bytes (`ZPackage.ReadByteArray`).
#[binrw]
#[brw(little)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ByteArray {
    #[br(temp)]
    #[bw(calc = data.len() as i32)]
    len: i32,
    #[br(count = len)]
    pub data: Vec<u8>,
}

/// `i32` count followed by strings.
#[binrw]
#[brw(little)]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct StringList {
    #[br(temp)]
    #[bw(calc = items.len() as i32)]
    len: i32,
    #[br(count = len)]
    pub items: Vec<CsString>,
}

/// `i32` count followed by `i32`s.
#[binrw]
#[brw(little)]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct IntList {
    #[br(temp)]
    #[bw(calc = items.len() as i32)]
    len: i32,
    #[br(count = len)]
    pub items: Vec<i32>,
}

/// `i32` count followed by `(string, i32)` pairs.
#[binrw]
#[brw(little)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StringIntMap {
    #[br(temp)]
    #[bw(calc = entries.len() as i32)]
    len: i32,
    #[br(count = len)]
    pub entries: Vec<StringIntEntry>,
}

#[binrw]
#[brw(little)]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct StringIntEntry {
    pub key: CsString,
    pub value: i32,
}

/// `i32` count followed by `(string, f32)` pairs.
#[binrw]
#[brw(little)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StringFloatMap {
    #[br(temp)]
    #[bw(calc = entries.len() as i32)]
    len: i32,
    #[br(count = len)]
    pub entries: Vec<StringFloatEntry>,
}

#[binrw]
#[brw(little)]
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct StringFloatEntry {
    pub key: CsString,
    pub value: f32,
}

/// `i32` count followed by `(string, string)` pairs.
#[binrw]
#[brw(little)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StringStringMap {
    #[br(temp)]
    #[bw(calc = entries.len() as i32)]
    len: i32,
    #[br(count = len)]
    pub entries: Vec<StringStringEntry>,
}

#[binrw]
#[brw(little)]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct StringStringEntry {
    pub key: CsString,
    pub value: CsString,
}

#[binrw]
#[brw(little)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[binrw]
#[brw(little)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Vec2i {
    pub x: i32,
    pub y: i32,
}

#[binrw]
#[brw(little)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Vec2s {
    pub x: i16,
    pub y: i16,
}

#[binrw]
#[brw(little)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Quat {
    pub const IDENTITY: Quat = Quat {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    };
}

/// `ZPackage.ReadNumItems`: one byte, or two bytes big-endian with the high
/// bit set when the count is 128 or more. Worlds older than
/// `versions::world::ZDO_WIDE_COUNTS` always used a single byte.
pub fn read_num_items<R: Read + Seek>(reader: &mut R, wide: bool) -> BinResult<usize> {
    let first = u8::read_le(reader)?;
    if wide && first & 0x80 != 0 {
        let second = u8::read_le(reader)?;
        Ok(((usize::from(first) & 0x7F) << 8) | usize::from(second))
    } else {
        Ok(usize::from(first))
    }
}

/// Counterpart to [`read_num_items`]. `wide` must match the version the count
/// is being written for: a pre-`ZDO_WIDE_COUNTS` reader takes the first byte
/// at face value, so emitting the two-byte form into such a file desynchronises
/// the whole ZDO stream.
pub fn write_num_items<W: Write + Seek>(writer: &mut W, count: usize, wide: bool) -> BinResult<()> {
    let limit = if wide { 0x8000 } else { 0x100 };
    if count < 128 || (!wide && count < limit) {
        (count as u8).write_le(writer)
    } else if count < limit {
        ((count >> 8) as u8 | 0x80).write_le(writer)?;
        (count as u8).write_le(writer)
    } else {
        Err(binrw::Error::AssertFail {
            pos: writer.stream_position()?,
            message: format!(
                "property table with {count} entries exceeds the {} limit of this world version",
                limit - 1
            ),
        })
    }
}

impl Vec3 {
    /// The zone the point falls in (`ZoneSystem.GetZone`): 64 m zones,
    /// centred so that zone 0 spans -32..32. The game computes the division
    /// in double precision and narrows before flooring, so this does too.
    pub fn zone(&self) -> Vec2s {
        let cell = |v: f32| ((f64::from(v) + 32.0) / 64.0) as f32;
        Vec2s {
            x: cell(self.x).floor() as i32 as i16,
            y: cell(self.z).floor() as i32 as i16,
        }
    }

    /// The compact form `ZDO.Save` prefers for a position
    /// (`Utils.SmallPosition`): two shorts when the height is exactly +0.0
    /// and both horizontal coordinates are whole numbers a short can hold.
    /// The game additionally clamps a coordinate beyond +-20000 down to that
    /// bound so it still fits; nothing here does, because the full form is
    /// lossless and the reader takes either. A file the game wrote already
    /// carries the clamped value, so it round-trips through here unchanged.
    pub fn small(&self) -> Option<Vec2s> {
        if self.y.to_bits() != 0 {
            return None;
        }
        let x = self.x as i16;
        let z = self.z as i16;
        (f32::from(x) == self.x && f32::from(z) == self.z).then_some(Vec2s { x, y: z })
    }

    /// `Utils.CloseToZero`: every component is under half a degree, which is
    /// what the compact rotation encoding cannot represent anyway. `ZDO.Save`
    /// omits such a rotation rather than writing a zero.
    pub fn close_to_zero(&self) -> bool {
        [self.x, self.y, self.z]
            .iter()
            .all(|v| ((v * 2.0).abs() as i32) < 1)
    }
}

impl Vec2s {
    /// A compact position back to its point: `(x, 0, z)`.
    pub fn expand(self) -> Vec3 {
        Vec3 {
            x: f32::from(self.x),
            y: 0.0,
            z: f32::from(self.y),
        }
    }
}

/// `ZPackage.ReadSmallRotation`: Euler angles in half-degree steps. One
/// short with the high bit set holds a yaw-only rotation; otherwise two
/// shorts hold three 10-bit fields, `x | y << 10 | z << 20`, high half
/// first.
pub fn read_small_rotation<R: Read + Seek>(reader: &mut R) -> BinResult<Vec3> {
    let first = u32::from(u16::read_le(reader)?);
    if first & 0x8000 != 0 {
        return Ok(Vec3 {
            x: 0.0,
            y: (first & 0x7FFF) as f32 * 0.5,
            z: 0.0,
        });
    }
    let packed = (first << 16) | u32::from(u16::read_le(reader)?);
    Ok(Vec3 {
        x: (packed & 0x3FF) as f32 * 0.5,
        y: ((packed >> 10) & 0x3FF) as f32 * 0.5,
        z: ((packed >> 20) & 0x3FF) as f32 * 0.5,
    })
}

impl Vec3 {
    /// What a rotation comes back as after the compact encoding: half-degree
    /// steps, with a negligible pitch and roll dropped together
    /// ([`write_small_rotation`] then [`read_small_rotation`]).
    pub fn quantized_rotation(&self) -> Vec3 {
        let x = (self.x * 2.0) as u32;
        let y = (self.y * 2.0) as u32;
        let z = (self.z * 2.0) as u32;
        let negligible = |v: u32| v <= 1 || v >= 719;
        if negligible(x) && negligible(z) {
            return Vec3 {
                x: 0.0,
                y: (y & 0x7FFF) as f32 * 0.5,
                z: 0.0,
            };
        }
        Vec3 {
            x: (x & 0x3FF) as f32 * 0.5,
            y: (y & 0x3FF) as f32 * 0.5,
            z: (z & 0x3FF) as f32 * 0.5,
        }
    }
}

/// `ZPackage.WriteSmallRotation`. The game doubles the angles and truncates
/// to whole half-degrees; a pitch or roll that truncates to 0, 1 or anything
/// from 719 up (that is, within half a degree of a full turn either way) is
/// treated as zero and dropped in favour of the yaw-only form.
pub fn write_small_rotation<W: Write + Seek>(writer: &mut W, rotation: &Vec3) -> BinResult<()> {
    let x = (rotation.x * 2.0) as u32;
    let y = (rotation.y * 2.0) as u32;
    let z = (rotation.z * 2.0) as u32;
    let negligible = |v: u32| v <= 1 || v >= 719;
    if negligible(x) && negligible(z) {
        return ((y | 0x8000) as u16).write_le(writer);
    }
    let packed = x | (y << 10) | (z << 20);
    ((packed >> 16) as u16).write_le(writer)?;
    (packed as u16).write_le(writer)
}
