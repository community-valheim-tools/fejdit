//! Serialized inventories and single items (`Inventory.Save` / `Inventory.Load`,
//! `ItemDrop.ItemData.Save` / `.Load`).
//!
//! An inventory is an `i32` item-data version followed by its items. Up to
//! version 107 each item is the verbose [`LegacyItem`] behind an `i32` count;
//! from [`inventory::COMPACT`] the count is a `u16` and each item is the
//! [`CompactItem`], a flags byte plus only the fields that differ from their
//! defaults, naming the prefab by hash.
//!
//! Where they turn up: a container's `items` property (a base64 string up to
//! world version 39, a raw byte array from 40), the player's own inventory in
//! a `.fch`, and, from world version 40, a single item's `itemData` byte
//! array on an `ItemDrop` ZDO (one version byte, then a compact item).

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use binrw::io::{Cursor, Read, Seek, Write};
use binrw::{BinRead, BinResult, BinWrite, Endian, binrw};
use serde::Serialize;

use super::primitives::{
    Bool, CsString, StringStringEntry, StringStringMap, Vec2i, read_num_items, write_num_items,
};
use super::versions::inventory;
use crate::hash::stable_hash;
use crate::names::NameTable;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Inventory {
    pub version: i32,
    pub items: Vec<Item>,
}

/// One item in whichever record its inventory's version uses.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Legacy(LegacyItem),
    Compact(CompactItem),
}

/// The item record of versions up to 107 (`Inventory.LoadOld`).
#[binrw]
#[brw(little)]
#[br(import(version: i32))]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LegacyItem {
    /// Drop prefab name, e.g. `SwordIron`. Empty for items missing a prefab.
    pub name: CsString,
    pub stack: i32,
    pub durability: f32,
    pub grid_pos: Vec2i,
    pub equipped: Bool,
    #[br(if(version >= inventory::QUALITY))]
    pub quality: Option<i32>,
    #[br(if(version >= inventory::VARIANT))]
    pub variant: Option<i32>,
    #[br(if(version >= inventory::CRAFTER))]
    pub crafter: Option<Crafter>,
    #[br(if(version >= inventory::CUSTOM_DATA))]
    pub custom_data: Option<StringStringMap>,
    #[br(if(version >= inventory::WORLD_LEVEL))]
    pub world_level: Option<i32>,
    #[br(if(version >= inventory::PICKED_UP))]
    pub picked_up: Option<Bool>,
    #[br(if(inventory::has_cheated(version)))]
    pub cheated: Option<Bool>,
}

/// The item record from version 108 (`ItemDrop.ItemData.Save` / `.Load`).
/// Each optional field is present exactly when its bit in the flags byte is
/// set, and the game sets a bit only for a value that is not the default.
#[binrw]
#[brw(little)]
#[br(import(version: i32))]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompactItem {
    /// Durability in hundredths, truncated (`(int)(m_durability * 100f)`).
    pub durability_hundredths: i32,
    pub grid_x: u8,
    pub grid_y: u8,
    pub world_level: u8,
    #[br(temp)]
    #[bw(calc = pack_item_flags(picked_up, equipped, quality, stack, variant, crafter, prefab_hash, custom_data))]
    flags: u8,
    #[br(calc = flags & item_flags::PICKED_UP != 0)]
    #[bw(ignore)]
    pub picked_up: bool,
    #[br(calc = flags & item_flags::EQUIPPED != 0)]
    #[bw(ignore)]
    pub equipped: bool,
    /// Absent when 1.
    #[br(if(flags & item_flags::QUALITY != 0))]
    pub quality: Option<u16>,
    /// Absent when 1.
    #[br(if(flags & item_flags::STACK != 0))]
    pub stack: Option<u16>,
    /// Absent when 0.
    #[br(if(flags & item_flags::VARIANT != 0))]
    pub variant: Option<i32>,
    /// Absent when the crafter id is 0, name and all.
    #[br(if(flags & item_flags::CRAFTER != 0))]
    pub crafter: Option<Crafter>,
    /// `GetStableHashCode` of the drop prefab name; absent when the item has
    /// no prefab.
    #[br(if(flags & item_flags::PREFAB != 0))]
    pub prefab_hash: Option<i32>,
    #[br(if(flags & item_flags::CUSTOM_DATA != 0))]
    pub custom_data: Option<PackedStringMap>,
    /// Bit 0 is `m_cheated`.
    #[br(if(inventory::has_cheated(version)))]
    pub cheated_flags: Option<u8>,
}

mod item_flags {
    pub const PICKED_UP: u8 = 0x01;
    pub const EQUIPPED: u8 = 0x02;
    pub const QUALITY: u8 = 0x04;
    pub const STACK: u8 = 0x08;
    pub const VARIANT: u8 = 0x10;
    pub const CRAFTER: u8 = 0x20;
    pub const PREFAB: u8 = 0x40;
    pub const CUSTOM_DATA: u8 = 0x80;
}

#[allow(clippy::too_many_arguments)]
fn pack_item_flags(
    picked_up: &bool,
    equipped: &bool,
    quality: &Option<u16>,
    stack: &Option<u16>,
    variant: &Option<i32>,
    crafter: &Option<Crafter>,
    prefab_hash: &Option<i32>,
    custom_data: &Option<PackedStringMap>,
) -> u8 {
    let mut f = 0;
    if *picked_up {
        f |= item_flags::PICKED_UP;
    }
    if *equipped {
        f |= item_flags::EQUIPPED;
    }
    if quality.is_some() {
        f |= item_flags::QUALITY;
    }
    if stack.is_some() {
        f |= item_flags::STACK;
    }
    if variant.is_some() {
        f |= item_flags::VARIANT;
    }
    if crafter.is_some() {
        f |= item_flags::CRAFTER;
    }
    if prefab_hash.is_some() {
        f |= item_flags::PREFAB;
    }
    if custom_data.is_some() {
        f |= item_flags::CUSTOM_DATA;
    }
    f
}

#[binrw]
#[brw(little)]
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Crafter {
    pub id: i64,
    pub name: CsString,
}

/// `(string, string)` pairs behind a `NumItems` count rather than an `i32`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PackedStringMap {
    pub entries: Vec<StringStringEntry>,
}

impl BinRead for PackedStringMap {
    type Args<'a> = ();

    fn read_options<R: Read + Seek>(reader: &mut R, endian: Endian, _: ()) -> BinResult<Self> {
        let count = read_num_items(reader, true)?;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            entries.push(StringStringEntry::read_options(reader, endian, ())?);
        }
        Ok(PackedStringMap { entries })
    }
}

impl BinWrite for PackedStringMap {
    type Args<'a> = ();

    fn write_options<W: Write + Seek>(
        &self,
        writer: &mut W,
        endian: Endian,
        _: (),
    ) -> BinResult<()> {
        write_num_items(writer, self.entries.len(), true)?;
        for entry in &self.entries {
            entry.write_options(writer, endian, ())?;
        }
        Ok(())
    }
}

/// A single item as stored on an `ItemDrop` ZDO's `itemData` byte array
/// (`ItemDrop.SaveToZDO`): a one-byte item-data version, then the item.
#[binrw]
#[brw(little)]
#[derive(Clone, Debug, PartialEq)]
pub struct SingleItem {
    pub version: u8,
    #[br(args(i32::from(version)))]
    pub item: CompactItem,
}

impl SingleItem {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut cursor = Cursor::new(bytes);
        let item = SingleItem::read_le(&mut cursor).context("parsing item data")?;
        if cursor.position() as usize != bytes.len() {
            bail!("item data has unparsed trailing bytes");
        }
        Ok(item)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut cursor = Cursor::new(Vec::new());
        self.write_le(&mut cursor)
            .context("serializing item data")?;
        Ok(cursor.into_inner())
    }
}

impl BinRead for Inventory {
    type Args<'a> = ();

    fn read_options<R: Read + Seek>(reader: &mut R, endian: Endian, _: ()) -> BinResult<Self> {
        let version = i32::read_options(reader, endian, ())?;
        let mut items = Vec::new();
        if version >= inventory::COMPACT {
            let count = u16::read_options(reader, endian, ())?;
            for _ in 0..count {
                items.push(Item::Compact(CompactItem::read_options(
                    reader,
                    endian,
                    (version,),
                )?));
            }
        } else {
            let count = i32::read_options(reader, endian, ())?;
            for _ in 0..count.max(0) {
                items.push(Item::Legacy(LegacyItem::read_options(
                    reader,
                    endian,
                    (version,),
                )?));
            }
        }
        Ok(Inventory { version, items })
    }
}

impl BinWrite for Inventory {
    type Args<'a> = ();

    fn write_options<W: Write + Seek>(
        &self,
        writer: &mut W,
        endian: Endian,
        _: (),
    ) -> BinResult<()> {
        self.version.write_options(writer, endian, ())?;
        let pos = writer.stream_position()?;
        let compact = self.version >= inventory::COMPACT;
        if compact {
            let count = u16::try_from(self.items.len()).map_err(|_| binrw::Error::AssertFail {
                pos,
                message: format!("{} items do not fit a 16-bit count", self.items.len()),
            })?;
            count.write_options(writer, endian, ())?;
        } else {
            (self.items.len() as i32).write_options(writer, endian, ())?;
        }
        for item in &self.items {
            match (item, compact) {
                (Item::Compact(c), true) => c.write_options(writer, endian, ())?,
                (Item::Legacy(l), false) => l.write_options(writer, endian, ())?,
                _ => {
                    return Err(binrw::Error::AssertFail {
                        pos,
                        message: format!(
                            "item record does not match inventory version {}",
                            self.version
                        ),
                    });
                }
            }
        }
        Ok(())
    }
}

impl Inventory {
    /// A container's `items` string, as stored up to world version 39.
    pub fn from_base64(text: &str) -> Result<Self> {
        let bytes = BASE64.decode(text).context("decoding inventory base64")?;
        Self::parse(&bytes)
    }

    /// A raw inventory package: a container's `items` byte array from world
    /// version 40, or the player's inventory inside a `.fch`.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut cursor = Cursor::new(bytes);
        let inv = Inventory::read_le(&mut cursor).context("parsing inventory package")?;
        let consumed = cursor.position() as usize;
        if consumed != bytes.len() {
            bail!(
                "inventory package has {} unparsed trailing bytes",
                bytes.len() - consumed
            );
        }
        Ok(inv)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut cursor = Cursor::new(Vec::new());
        self.write_le(&mut cursor)
            .context("serializing inventory")?;
        Ok(cursor.into_inner())
    }

    /// Re-encode in the current compact form, as `ZDOMan.ConvertInventories`
    /// does when it loads a pre-1.0 world: every item is loaded into an
    /// `ItemData` and saved again. Items with no prefab name are dropped,
    /// because `Inventory.LoadOld` never adds them.
    pub fn to_compact(&self) -> Inventory {
        Inventory {
            version: inventory::CURRENT,
            items: self
                .items
                .iter()
                .filter_map(|item| match item {
                    Item::Compact(c) => Some(Item::Compact(c.clone())),
                    Item::Legacy(l) if l.name.as_str().is_empty() => None,
                    Item::Legacy(l) => Some(Item::Compact(l.to_compact())),
                })
                .collect(),
        }
    }
}

impl LegacyItem {
    /// `Inventory.LoadOld` + `AddItem` + `ItemData.Save`: which fields the
    /// compact record keeps follows the game's not-the-default tests, and
    /// the crafter name goes with the id.
    pub fn to_compact(&self) -> CompactItem {
        let crafter = self.crafter.as_ref().filter(|c| c.id != 0).cloned();
        let custom_data = self
            .custom_data
            .as_ref()
            .filter(|d| !d.entries.is_empty())
            .map(|d| PackedStringMap {
                entries: d.entries.clone(),
            });
        CompactItem {
            durability_hundredths: (self.durability * 100.0) as i32,
            grid_x: self.grid_pos.x as u8,
            grid_y: self.grid_pos.y as u8,
            world_level: self.world_level.unwrap_or(0) as u8,
            picked_up: self.picked_up.is_some_and(|b| b.0),
            equipped: self.equipped.0,
            quality: Some(self.quality.unwrap_or(1) as u16).filter(|q| *q != 1),
            stack: Some(self.stack as u16).filter(|s| *s != 1),
            variant: self.variant.filter(|v| *v != 0),
            crafter,
            prefab_hash: Some(stable_hash(self.name.as_str())),
            custom_data,
            cheated_flags: Some(u8::from(self.cheated.is_some_and(|b| b.0))),
        }
    }
}

impl Item {
    /// The prefab name, resolved through `names` when the record stores a
    /// hash. Empty for an item with no prefab.
    pub fn name(&self, names: &NameTable) -> String {
        match self {
            Item::Legacy(l) => l.name.0.clone(),
            Item::Compact(c) => c.prefab_hash.map_or(String::new(), |h| names.display(h)),
        }
    }

    /// The prefab hash, whichever way the record names it.
    pub fn prefab_hash(&self) -> Option<i32> {
        match self {
            Item::Legacy(l) if l.name.as_str().is_empty() => None,
            Item::Legacy(l) => Some(stable_hash(l.name.as_str())),
            Item::Compact(c) => c.prefab_hash,
        }
    }

    pub fn stack(&self) -> i32 {
        match self {
            Item::Legacy(l) => l.stack,
            Item::Compact(c) => c.stack.map_or(1, i32::from),
        }
    }

    pub fn quality(&self) -> i32 {
        match self {
            Item::Legacy(l) => l.quality.unwrap_or(1),
            Item::Compact(c) => c.quality.map_or(1, i32::from),
        }
    }

    pub fn equipped(&self) -> bool {
        match self {
            Item::Legacy(l) => l.equipped.0,
            Item::Compact(c) => c.equipped,
        }
    }

    pub fn crafter_name(&self) -> &str {
        let crafter = match self {
            Item::Legacy(l) => l.crafter.as_ref(),
            Item::Compact(c) => c.crafter.as_ref(),
        };
        crafter.map_or("", |c| c.name.as_str())
    }
}
