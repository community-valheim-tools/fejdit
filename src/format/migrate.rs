//! What `ZDOMan.Load` does to a pre-1.0 world's objects before the game
//! saves it in the chunked layout. These are data migrations rather than
//! format changes, but a chunked world that skipped them is broken: 1.0 reads
//! a container's `items` as a byte array and an item stand's `item` as a
//! hash, so the old string forms would simply be ignored and every chest
//! would open empty.
//!
//! * `ConvertInventories`: an `ItemDrop` ZDO's scattered `stack`,
//!   `durability`, `crafterName`, ... keys become one `itemData` byte array,
//!   and a container's base64 `items` string becomes a raw byte array in the
//!   compact item format.
//! * `ConvertPrefabStrings`: `Content`, `item` and `<n>_item` change from
//!   prefab-name strings to `GetStableHashCode` ints.
//!
//! An armor stand's slots, stored as `<n>_durability`, `<n>_crafterName` and
//! so on by 0.221's indexed `ItemDrop.SaveToZDO`, are folded into the
//! `<n>_itemData` blob 1.0 reads. Game 1.0.12 folds them too (its
//! `ConvertInventories` takes an index and runs for -1 through 13); 1.0.0
//! through 1.0.11 converted only the unindexed keys, which is why armor came
//! off a stand with default stats and no crafter there, and why
//! [`crate::repair`] exists for worlds those builds converted.
//!
//! Two steps still go beyond the game. `ConvertPrefabStrings` hashes only
//! `0_item` through `5_item`, while a stand has up to [`ITEM_SLOTS`] of them,
//! so slots 6 and up keep a prefab-name string that 1.0's `ArmorStand`, which
//! reads the key with `GetInt`, cannot see: every `<n>_item` is hashed here.
//! And the fold drops the indexed keys it consumed, where the game removes
//! only the unindexed spelling of them.
//!
//! The rest of `RemoveObsoleteData` only drops keys the new build no longer
//! reads; nothing depends on it, so it is left alone and the objects keep
//! what they had.

use std::sync::OnceLock;

use super::inventory::{CompactItem, Crafter, Inventory, PackedStringMap, SingleItem};
use super::primitives::{ByteArray, CsString, StringStringEntry};
use super::versions::{inventory, world};
use super::zdo::Zdo;
use crate::hash::stable_hash;

/// Slots an armor stand can have, and so the number of `<n>_` key sets an
/// object can carry. `ArmorStand.prefab` has 14 in game 1.0 (7 in 0.221) and
/// the gendered stands 9; `ZDOMan.ConvertInventories` loops index 0 through
/// 13 for the same reason.
pub const ITEM_SLOTS: i32 = 14;

/// How far `ZDOMan.ConvertPrefabStrings` gets: it hashes `0_item` through
/// `5_item` and no further, leaving the rest of a stand's slots as strings.
pub const GAME_HASHED_SLOTS: i32 = 6;

/// Apply every migration to one object. `world_version` is the version the
/// object was read at; the `cheated` flag is only picked up from the two
/// public-test versions that stored it as a key.
pub fn migrate_zdo(zdo: &mut Zdo, world_version: i32) {
    convert_item_keys(zdo, world_version);
    convert_armor_stand_slots(zdo, world_version);
    convert_container(zdo);
    convert_prefab_strings(zdo);
}

/// The item data 0.221 stored as separate keys, read off an object. `prefix`
/// is empty for an item lying in the world and `"<n>_"` for slot `n` of an
/// armor stand; `prefab_name` is the drop prefab, known only for a slot.
/// `None` when there is no such item, which `stack` decides as it does in
/// `ZDOMan.ConvertInventories`.
pub fn item_from_keys(
    zdo: &Zdo,
    prefix: &str,
    prefab_name: Option<&str>,
    world_version: i32,
) -> Option<CompactItem> {
    let key = |name: &str| stable_hash(&format!("{prefix}{name}"));
    let stack = zdo.int(key("stack"))?;
    let quality = zdo.int(key("quality")).unwrap_or(1);
    let variant = zdo.int(key("variant")).unwrap_or(0);
    let crafter_id = zdo.long(key("crafterID")).unwrap_or(0);
    let crafter_name = zdo.string(key("crafterName")).unwrap_or("").to_owned();
    let data_count = zdo.int(key("dataCount")).unwrap_or(0);
    let mut custom = Vec::new();
    for i in 0..data_count {
        custom.push(StringStringEntry {
            key: CsString::from(zdo.string(key(&format!("data_{i}"))).unwrap_or("")),
            value: CsString::from(zdo.string(key(&format!("data__{i}"))).unwrap_or("")),
        });
    }
    let stored_bool = |name: &str| zdo.int(key(name)).is_some_and(|v| v != 0);
    let cheated = matches!(world_version, 38 | 39) && stored_bool("cheated");
    Some(CompactItem {
        durability_hundredths: (zdo.float(key("durability")).unwrap_or(1.0) * 100.0) as i32,
        grid_x: 0,
        grid_y: 0,
        world_level: zdo.int(key("worldLevel")).unwrap_or(0) as u8,
        picked_up: stored_bool("pickedUp"),
        equipped: false,
        quality: Some(quality as u16).filter(|q| *q != 1),
        stack: Some(stack as u16).filter(|s| *s != 1),
        variant: Some(variant).filter(|v| *v != 0),
        crafter: (crafter_id != 0).then_some(Crafter {
            id: crafter_id,
            name: CsString(crafter_name),
        }),
        prefab_hash: prefab_name.map(stable_hash),
        custom_data: (!custom.is_empty()).then_some(PackedStringMap { entries: custom }),
        cheated_flags: Some(u8::from(cheated)),
    })
}

/// Store `item` under `<prefix>itemData` and drop the separate keys it was
/// built from, as `ConvertInventories` does: `quality` and `variant` stay as
/// ints when they are not the default, since `SaveToZDO` only writes them on
/// change.
pub fn store_item(zdo: &mut Zdo, prefix: &str, item: &CompactItem) {
    let key = |name: &str| stable_hash(&format!("{prefix}{name}"));
    let blob = SingleItem {
        version: inventory::CURRENT as u8,
        item: item.clone(),
    }
    .to_bytes()
    .expect("an in-memory item serializes");
    set_bytes(zdo, key("itemData"), blob);
    let data_count = item.custom_data.as_ref().map_or(0, |d| d.entries.len());
    let mut drop_ints = vec![
        key("stack"),
        key("dataCount"),
        key("worldLevel"),
        key("pickedUp"),
        key("cheated"),
    ];
    if item.quality.is_none() {
        drop_ints.push(key("quality"));
    }
    if item.variant.is_none() {
        drop_ints.push(key("variant"));
    }
    zdo.ints.entries.retain(|(k, _)| !drop_ints.contains(k));
    zdo.floats.entries.retain(|(k, _)| *k != key("durability"));
    zdo.longs.entries.retain(|(k, _)| *k != key("crafterID"));
    let mut drop_strings = vec![key("crafterName")];
    for i in 0..data_count {
        drop_strings.push(key(&format!("data_{i}")));
        drop_strings.push(key(&format!("data__{i}")));
    }
    zdo.strings
        .entries
        .retain(|(k, _)| !drop_strings.contains(k));
}

/// `ConvertInventories`, first half: any object carrying an `int:stack` is
/// an item lying in the world. The `ItemData` the game builds there has no
/// drop prefab (the ZDO's own prefab is the item), so neither does the blob.
fn convert_item_keys(zdo: &mut Zdo, world_version: i32) {
    if let Some(item) = item_from_keys(zdo, "", None, world_version) {
        store_item(zdo, "", &item);
    }
}

/// Fold each armor-stand slot's separate keys into `<n>_itemData`, which is
/// what 1.0 reads when the item is taken down. Runs before the prefab
/// strings are hashed, so `<n>_item` still names the prefab.
///
/// A slot is folded only when it names an item. 1.0.12 goes by `<n>_stack`
/// instead, so it writes a blob for a slot whose item was taken down long ago
/// and whose stats are stale leftovers; `ArmorStand` looks at `<n>_item`
/// before it reads any of that, so the blob would never be seen.
fn convert_armor_stand_slots(zdo: &mut Zdo, world_version: i32) {
    for (slot, key) in slot_item_keys().iter().enumerate() {
        let Some(name) = zdo
            .string(*key)
            .filter(|n| !n.is_empty())
            .map(str::to_owned)
        else {
            continue;
        };
        let prefix = format!("{slot}_");
        if let Some(item) = item_from_keys(zdo, &prefix, Some(&name), world_version) {
            store_item(zdo, &prefix, &item);
        }
    }
}

/// `ConvertInventories`, second half: a container's base64 `items` string
/// becomes a raw package in the compact item format. An inventory the tool
/// cannot decode is left as it is, which is also what the game does with a
/// string that decodes to nothing.
fn convert_container(zdo: &mut Zdo) {
    let items = stable_hash("items");
    let Some(text) = zdo.string(items) else {
        return;
    };
    if text.is_empty() {
        zdo.strings.entries.retain(|(k, _)| *k != items);
        return;
    }
    let Ok(inventory) = Inventory::from_base64(text) else {
        return;
    };
    let bytes = inventory
        .to_compact()
        .to_bytes()
        .expect("an in-memory inventory serializes");
    set_bytes(zdo, items, bytes);
    zdo.strings.entries.retain(|(k, _)| *k != items);
}

/// `ConvertPrefabStrings`, carried past `5_item` to every slot a stand can
/// have: see [`hash_prefab_string`].
fn convert_prefab_strings(zdo: &mut Zdo) {
    for key in [stable_hash("Content"), stable_hash("item")]
        .iter()
        .chain(slot_item_keys())
    {
        hash_prefab_string(zdo, *key);
    }
}

/// The `<n>_item` key for every slot. Each object in the world is looked up
/// under all of them twice over during a conversion, so they are hashed once.
pub fn slot_item_keys() -> &'static [i32; ITEM_SLOTS as usize] {
    static KEYS: OnceLock<[i32; ITEM_SLOTS as usize]> = OnceLock::new();
    KEYS.get_or_init(|| std::array::from_fn(|slot| stable_hash(&format!("{slot}_item"))))
}

/// Replace the prefab name stored as a string under `key` with the hash 1.0
/// reads, as `ConvertPrefabStrings` does, and report the name it converted.
/// An empty string is dropped without leaving an int behind, which is how the
/// game spells "no item here".
pub fn hash_prefab_string(zdo: &mut Zdo, key: i32) -> Option<String> {
    let name = zdo.string(key).map(str::to_owned)?;
    zdo.strings.entries.retain(|(k, _)| *k != key);
    if name.is_empty() {
        return None;
    }
    set_int(zdo, key, stable_hash(&name));
    Some(name)
}

fn set_bytes(zdo: &mut Zdo, key: i32, data: Vec<u8>) {
    match zdo.byte_arrays.entries.iter_mut().find(|(k, _)| *k == key) {
        Some((_, v)) => v.data = data,
        None => zdo.byte_arrays.entries.push((key, ByteArray { data })),
    }
}

fn set_int(zdo: &mut Zdo, key: i32, value: i32) {
    match zdo.ints.entries.iter_mut().find(|(k, _)| *k == key) {
        Some((_, v)) => *v = value,
        None => zdo.ints.entries.push((key, value)),
    }
}

/// Whether an object read at `version` still needs these migrations.
pub fn needs_migration(version: i32) -> bool {
    version < world::CHUNKED_SAVE
}
