//! Repairs for damage the game's own migrations leave behind.
//!
//! When game 1.0.0 through 1.0.11 loads a world saved by 0.221 or earlier,
//! `ZDOMan.ConvertInventories` folds every loose item's separate keys into one
//! `itemData` blob, but it only handles the unindexed keys. An armor stand
//! keeps its armor in slots, stored by the indexed `ItemDrop.SaveToZDO` as
//! `<n>_durability`, `<n>_quality`, `<n>_crafterID` and so on. Those are left
//! where they are, `RemoveObsoleteData` strips `<n>_crafterName` outright, and
//! from then on `ArmorStand` reads only `<n>_itemData`, which the conversion
//! never wrote. The stand still shows the armor (its `<n>_item` hash
//! survived), but taking it down hands back a piece with default durability,
//! quality 1 and no crafter. 1.0.12 folds the slots itself, so only worlds
//! the earlier builds converted need this; the conversion is gated on the
//! world's version, so it never runs over such a world again.
//!
//! [`restore_armor_stands`] puts the blobs back from the world as it was
//! before the game converted it, matching stands by prefab and position and
//! slots by the item they hold.
//!
//! No build hashes past `5_item` (`ConvertPrefabStrings` stops there), so
//! slots 6 and up come out of any conversion still holding a prefab-name
//! string, which `ArmorStand.GetInt` reads as "empty" - the armor is invisible
//! and cannot be taken down at all. Those keys are converted here too.

use std::collections::HashMap;

use serde::Serialize;

use crate::format::migrate::{
    ITEM_SLOTS, hash_prefab_string, item_from_keys, slot_item_keys, store_item,
};
use crate::format::world::WorldData;
use crate::format::zdo::Zdo;
use crate::hash::stable_hash;
use crate::names::NameTable;

#[derive(Clone, Debug, Default, Serialize)]
pub struct Report {
    /// Stands in the old world holding at least one item.
    pub old_stands: usize,
    /// Of those, stands found at the same place in the new world.
    pub stands_matched: usize,
    /// Slots whose blob was written.
    pub slots_restored: usize,
    /// Slots left alone because the new world already has a blob for them:
    /// the item was placed again after the conversion, or an earlier run
    /// already restored it.
    pub slots_already_restored: usize,
    /// Slots left alone because the new stand holds a different item, or
    /// none, there now.
    pub slots_changed: usize,
    /// Slots in the old world with no item data to restore from.
    pub slots_without_data: usize,
    /// Slots whose `<n>_item` was still a prefab-name string in the new world,
    /// because the game's conversion hashes only the first six, and has been
    /// converted to the hash `ArmorStand` reads.
    pub slot_items_hashed: usize,
    /// Old stands with no counterpart in the new world (moved or gone).
    pub stands_unmatched: usize,
    /// One line per restored slot, when asked for.
    pub restored: Vec<RestoredSlot>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RestoredSlot {
    pub stand: String,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub slot: i32,
    pub item: String,
    pub quality: i32,
    pub durability: f32,
    pub crafter: String,
}

/// Where a stand is, as a map key: prefab and exact position. The
/// conversion keeps positions bit for bit (the compact form only applies
/// to whole numbers), so exact equality is the right test.
fn place(z: &Zdo) -> (i32, [u32; 3]) {
    (
        z.prefab,
        [
            z.position.x.to_bits(),
            z.position.y.to_bits(),
            z.position.z.to_bits(),
        ],
    )
}

/// The prefab name in slot `n` of a pre-1.0 stand.
fn old_slot_item(z: &Zdo, slot: usize) -> Option<&str> {
    z.string(slot_item_keys()[slot]).filter(|s| !s.is_empty())
}

/// Copy each armor-stand slot's item data from `old` (the world before the
/// game converted it) into `new` (the converted world), where the slot still
/// holds the same item and has no blob yet. `detailed` records every
/// restored slot in the report.
pub fn restore_armor_stands(
    old: &WorldData,
    new: &mut WorldData,
    names: &NameTable,
    detailed: bool,
) -> Report {
    let old_version = old.version();
    let mut stands: HashMap<(i32, [u32; 3]), Vec<&Zdo>> = HashMap::new();
    for z in old.zdos() {
        if (0..ITEM_SLOTS as usize).any(|n| old_slot_item(z, n).is_some()) {
            stands.entry(place(z)).or_default().push(z);
        }
    }
    let mut report = Report {
        old_stands: stands.values().map(Vec::len).sum(),
        ..Report::default()
    };

    for target in new.zdos_mut() {
        let Some(candidates) = stands.get_mut(&place(target)) else {
            continue;
        };
        // Two stands on the same spot are matched in file order.
        if candidates.is_empty() {
            continue;
        }
        let source = candidates.remove(0);
        report.stands_matched += 1;
        for slot in 0..ITEM_SLOTS as usize {
            let Some(name) = old_slot_item(source, slot) else {
                continue;
            };
            let prefix = format!("{slot}_");
            let item_key = slot_item_keys()[slot];
            // A slot past the sixth still names its item with a string: the
            // game's conversion hashes no further. Give it the hash the stand
            // reads, whether or not there is a blob to restore below.
            if target.int(item_key).is_none() && target.string(item_key) == Some(name) {
                hash_prefab_string(target, item_key);
                report.slot_items_hashed += 1;
            }
            if target.int(item_key) != Some(stable_hash(name)) {
                report.slots_changed += 1;
                continue;
            }
            if target
                .byte_arrays
                .contains(stable_hash(&format!("{prefix}itemData")))
            {
                report.slots_already_restored += 1;
                continue;
            }
            let Some(item) = item_from_keys(source, &prefix, Some(name), old_version) else {
                report.slots_without_data += 1;
                continue;
            };
            store_item(target, &prefix, &item);
            report.slots_restored += 1;
            if detailed {
                report.restored.push(RestoredSlot {
                    stand: names.display(target.prefab),
                    x: target.position.x,
                    y: target.position.y,
                    z: target.position.z,
                    slot: slot as i32,
                    item: name.to_owned(),
                    quality: item.quality.map_or(1, i32::from),
                    durability: item.durability_hundredths as f32 / 100.0,
                    crafter: item
                        .crafter
                        .as_ref()
                        .map_or(String::new(), |c| c.name.0.clone()),
                });
            }
        }
    }
    report.stands_unmatched = stands.values().map(Vec::len).sum();
    report
}
