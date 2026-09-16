//! Reverse lookup from stable hashes to prefab names and ZDO property keys.
//!
//! The game only stores hashes. The built-in tables come from `data/`: prefab
//! names extracted from the game's assets and property keys mined from the
//! game code. Names seen in the world itself (inventory item names) are added
//! at runtime so item prefabs missing from the table still resolve.
//!
//! Animator parameters are a separate family: `ZSyncAnimation` keys them by
//! `438569 + CRC32(name)` rather than `GetStableHashCode`, so they are learned
//! by hash and shown as `anim:<name>` to keep the two namespaces apart.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};

use crate::hash::{animator_hash, animator_key, stable_hash};

const PREFAB_NAMES: &str = include_str!("../data/prefab_names.txt");
const ZDO_KEYS: &str = include_str!("../data/zdo_keys.txt");
const CREATURE_PREFABS: &str = include_str!("../data/creature_prefabs.txt");
const ANIM_PARAMS: &str = include_str!("../data/anim_params.txt");

/// Highest slot index generated for indexed keys such as `3_item`.
const INDEXED_KEY_SLOTS: usize = 64;

/// Highest spawner index generated for `SpawnSystem` keys such as
/// `b_Greydwarf21` (`"b_"`/`"e_"` + creature prefab + spawn list index).
const SPAWNER_SLOTS: usize = 256;

#[derive(Clone, Debug, Default)]
pub struct NameTable {
    names: HashMap<i32, String>,
}

impl NameTable {
    pub fn builtin() -> Self {
        let mut table = NameTable::default();
        for line in PREFAB_NAMES.lines() {
            table.learn(line);
        }
        for line in ZDO_KEYS.lines() {
            table.learn_key(line);
        }
        for creature in CREATURE_PREFABS
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
        {
            for i in 0..SPAWNER_SLOTS {
                table.learn(&format!("b_{creature}{i}"));
                table.learn(&format!("e_{creature}{i}"));
            }
        }
        for param in ANIM_PARAMS.lines().map(str::trim).filter(|l| !l.is_empty()) {
            // `anim_speed` is written raw; every other parameter through the
            // offset. Learning both costs nothing and covers either form.
            let display = format!("anim:{param}");
            table.insert(animator_key(param), &display);
            table.insert(animator_hash(param), &display);
        }
        table
    }

    /// Add one name per non-empty, non-comment line of a text file.
    pub fn add_file(&mut self, path: &Path) -> Result<usize> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading names from {}", path.display()))?;
        let mut added = 0;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            self.learn(line);
            self.learn_key(line);
            added += 1;
        }
        Ok(added)
    }

    /// Record a display name for a hash the caller computed itself.
    fn insert(&mut self, hash: i32, display: &str) {
        self.names.entry(hash).or_insert_with(|| display.to_owned());
    }

    pub fn learn(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        self.names
            .entry(stable_hash(name))
            .or_insert_with(|| name.to_owned());
    }

    /// Learn a property key plus the derived forms the game builds from it:
    /// ZDOID pairs (`key_u`, `key_i`), per-slot item data (`3_key`), and the
    /// bare `key + index` concatenation behind `"slot" + i` in
    /// `CookingStation` or `"drop_hash" + i` in `Ragdoll`.
    fn learn_key(&mut self, key: &str) {
        let key = key.trim();
        if key.is_empty() {
            return;
        }
        self.learn(key);
        self.learn(&format!("{key}_u"));
        self.learn(&format!("{key}_i"));
        for i in 0..INDEXED_KEY_SLOTS {
            self.learn(&format!("{i}_{key}"));
            self.learn(&format!("{key}_{i}"));
            self.learn(&format!("{key}__{i}"));
            self.learn(&format!("{key}{i}"));
        }
    }

    pub fn resolve(&self, hash: i32) -> Option<&str> {
        self.names.get(&hash).map(String::as_str)
    }

    /// The name, or `#<hash>` when unknown.
    pub fn display(&self, hash: i32) -> String {
        match self.resolve(hash) {
            Some(name) => name.to_owned(),
            None => format!("#{hash}"),
        }
    }
}
