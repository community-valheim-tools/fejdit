//! Which ZDO keys each prefab may legitimately carry.
//!
//! Built from two generated tables: `prefab_components.txt` (prefab -> the
//! MonoBehaviour classes on it, from the game's prefab assets) and
//! `component_keys.txt` (class -> base class -> keys it reads or writes,
//! from the game code), plus hand-curated `component_key_patterns.txt` for
//! keys the game builds at run time such as `3_durability` or `slot2`.
//!
//! A key is *foreign* on a prefab when it is known to belong to some
//! component class and none of the prefab's components (or their base
//! classes) is one of them. Keys used only by non-component code (`Game`,
//! `ZDOMan`, ...) and keys this tool cannot name are never foreign, which
//! keeps modded data safe.
//!
//! ZDO connections are judged the same way from `connection_types.txt`, which
//! lists the classes allowed on each side of each connection type.

use std::collections::{HashMap, HashSet};

use crate::hash::{animator_hash, animator_key, stable_hash};

const PREFAB_COMPONENTS: &str = include_str!("../data/prefab_components.txt");
const COMPONENT_KEYS: &str = include_str!("../data/component_keys.txt");
const KEY_PATTERNS: &str = include_str!("../data/component_key_patterns.txt");
const CONNECTION_TYPES: &str = include_str!("../data/connection_types.txt");
const ANIM_PARAMS: &str = include_str!("../data/anim_params.txt");

/// The one class that keys ZDO properties by animator parameter hash.
const ANIM_CLASS: &str = "ZSyncAnimation";

#[derive(Debug)]
pub struct Schema {
    /// prefab hash -> indices into `classes`
    prefabs: HashMap<i32, Vec<usize>>,
    classes: Vec<Class>,
    class_index: HashMap<String, usize>,
    /// key hash -> classes that use it (component classes only)
    key_owners: HashMap<i32, Vec<usize>>,
    /// key hash -> a class that uses it but is never a component (allowed anywhere)
    universal_keys: HashSet<i32>,
    /// (class index, pattern) for dynamically named keys
    patterns: Vec<(usize, Pattern)>,
    /// connection type name -> (classes allowed on the source side, on the
    /// target side). An empty list means the side is unconstrained.
    connections: HashMap<String, (Vec<usize>, Vec<usize>)>,
    /// `ZSyncAnimation` keys, which are animator parameter hashes rather than
    /// `GetStableHashCode` of a name, so `key_owners` cannot hold them.
    anim_keys: HashSet<i32>,
    anim_class: Option<usize>,
}

#[derive(Debug, Default)]
struct Class {
    name: String,
    base: Option<usize>,
    /// Appears on at least one prefab.
    is_component: bool,
}

#[derive(Debug)]
struct Pattern(String);

impl Pattern {
    /// `#` matches one or more ASCII digits, `*` matches anything.
    fn matches(&self, name: &str) -> bool {
        fn go(p: &[u8], s: &[u8]) -> bool {
            match p.first() {
                None => s.is_empty(),
                Some(b'*') => (0..=s.len()).any(|i| go(&p[1..], &s[i..])),
                Some(b'#') => {
                    let digits = s.iter().take_while(|c| c.is_ascii_digit()).count();
                    (1..=digits).any(|i| go(&p[1..], &s[i..]))
                }
                Some(c) => s.first() == Some(c) && go(&p[1..], &s[1..]),
            }
        }
        go(self.0.as_bytes(), name.as_bytes())
    }
}

/// Why a key or connection is acceptable (or not) on a prefab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The prefab is not in the table; nothing can be said.
    UnknownPrefab,
    /// The key hash could not be named (or the connection type is not one
    /// this tool knows); nothing can be said.
    UnknownKey,
    /// The key is used by non-component code or a pattern that applies
    /// anywhere, or the connection side is unconstrained.
    Universal,
    /// One of the prefab's components uses this key or connection.
    Owned(String),
    /// Known key or connection, but only used by components the prefab lacks.
    Foreign(Vec<String>),
    /// Named key that no class in the table uses.
    Unattributed,
}

impl Schema {
    pub fn builtin() -> Self {
        let mut classes: Vec<Class> = Vec::new();
        let mut class_index: HashMap<String, usize> = HashMap::new();
        let mut intern = |classes: &mut Vec<Class>, name: &str| -> usize {
            if let Some(&i) = class_index.get(name) {
                return i;
            }
            classes.push(Class {
                name: name.to_owned(),
                ..Class::default()
            });
            class_index.insert(name.to_owned(), classes.len() - 1);
            classes.len() - 1
        };

        // Class -> base -> keys.
        let mut class_keys: Vec<(usize, Vec<String>)> = Vec::new();
        for line in COMPONENT_KEYS
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        {
            let mut parts = line.split('\t');
            let (Some(name), Some(base), Some(keys)) = (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let idx = intern(&mut classes, name);
            if !base.is_empty() && base != "MonoBehaviour" {
                let b = intern(&mut classes, base);
                classes[idx].base = Some(b);
            }
            class_keys.push((idx, keys.split(',').map(str::to_owned).collect()));
        }

        // Prefab -> components.
        let mut prefabs: HashMap<i32, Vec<usize>> = HashMap::new();
        for line in PREFAB_COMPONENTS
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        {
            let mut parts = line.split('\t');
            let (Some(prefab), Some(comps)) = (parts.next(), parts.next()) else {
                continue;
            };
            let idxs: Vec<usize> = comps
                .split(',')
                .filter(|c| !c.is_empty())
                .map(|c| {
                    let i = intern(&mut classes, c);
                    classes[i].is_component = true;
                    i
                })
                .collect();
            prefabs.insert(stable_hash(prefab), idxs);
        }
        // A base class of a component counts as a component too.
        for i in 0..classes.len() {
            if classes[i].is_component {
                let mut b = classes[i].base;
                while let Some(bi) = b {
                    classes[bi].is_component = true;
                    b = classes[bi].base;
                }
            }
        }

        let mut key_owners: HashMap<i32, Vec<usize>> = HashMap::new();
        let mut universal_keys = HashSet::new();
        for (idx, keys) in &class_keys {
            for key in keys {
                let h = stable_hash(key);
                if classes[*idx].is_component {
                    key_owners.entry(h).or_default().push(*idx);
                } else {
                    universal_keys.insert(h);
                }
            }
        }

        let mut patterns = Vec::new();
        for line in KEY_PATTERNS
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        {
            let mut parts = line.split('\t');
            let (Some(class), Some(pat)) = (parts.next(), parts.next()) else {
                continue;
            };
            let idx = intern(&mut classes, class);
            patterns.push((idx, Pattern(pat.trim().to_owned())));
        }

        let mut connections: HashMap<String, (Vec<usize>, Vec<usize>)> = HashMap::new();
        for line in CONNECTION_TYPES
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        {
            let mut parts = line.split('\t');
            let (Some(name), Some(source), Some(target)) =
                (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let mut classes_of = |list: &str| -> Vec<usize> {
                list.split(',')
                    .map(str::trim)
                    .filter(|c| !c.is_empty())
                    .map(|c| intern(&mut classes, c))
                    .collect()
            };
            let side = (classes_of(source), classes_of(target));
            connections.insert(name.trim().to_owned(), side);
        }

        let mut anim_keys = HashSet::new();
        for param in ANIM_PARAMS.lines().map(str::trim).filter(|l| !l.is_empty()) {
            anim_keys.insert(animator_key(param));
            anim_keys.insert(animator_hash(param));
        }
        let anim_class = class_index.get(ANIM_CLASS).copied();

        Schema {
            prefabs,
            classes,
            class_index,
            key_owners,
            universal_keys,
            patterns,
            connections,
            anim_keys,
            anim_class,
        }
    }

    pub fn knows_prefab(&self, prefab: i32) -> bool {
        self.prefabs.contains_key(&prefab)
    }

    pub fn class_id(&self, name: &str) -> Option<usize> {
        self.class_index.get(name).copied()
    }

    /// Whether `prefab` carries `class` (or a subclass of it). False for a
    /// prefab the table does not know, so callers that need to tell "no" from
    /// "cannot say" must ask [`Schema::knows_prefab`] first.
    pub fn prefab_has_class(&self, prefab: i32, class: &str) -> bool {
        self.class_id(class)
            .is_some_and(|c| self.prefab_has(prefab, c))
    }

    /// Whether the prefab has `class` or a subclass of it.
    fn prefab_has(&self, prefab: i32, class: usize) -> bool {
        let Some(comps) = self.prefabs.get(&prefab) else {
            return false;
        };
        comps.iter().any(|&c| {
            let mut cur = Some(c);
            while let Some(i) = cur {
                if i == class {
                    return true;
                }
                cur = self.classes[i].base;
            }
            false
        })
    }

    /// Judge `key` (with its resolved `name`, if any) on `prefab`.
    pub fn judge(&self, prefab: i32, key: i32, name: Option<&str>) -> Verdict {
        if !self.knows_prefab(prefab) {
            return Verdict::UnknownPrefab;
        }
        // Animator keys are checked first: they are not name hashes, so they
        // would otherwise fall through as unnameable and never be judged.
        if self.anim_keys.contains(&key) {
            return match self.anim_class {
                Some(c) if self.prefab_has(prefab, c) => {
                    Verdict::Owned(self.classes[c].name.clone())
                }
                Some(c) => Verdict::Foreign(vec![self.classes[c].name.clone()]),
                None => Verdict::UnknownKey,
            };
        }
        // Gather every class that uses the key, by exact hash and by pattern.
        let mut owners: Vec<usize> = self.key_owners.get(&key).cloned().unwrap_or_default();
        let mut universal = self.universal_keys.contains(&key);
        let named = name.is_some();
        if let Some(name) = name {
            for (class, pat) in &self.patterns {
                if pat.matches(name) {
                    if self.classes[*class].is_component {
                        owners.push(*class);
                    } else {
                        universal = true;
                    }
                }
            }
        }
        if let Some(&o) = owners.iter().find(|&&o| self.prefab_has(prefab, o)) {
            return Verdict::Owned(self.classes[o].name.clone());
        }
        // Component ownership is judged before the universal list because
        // debug commands (`Terminal`) reference many component keys too.
        if !owners.is_empty() {
            return Verdict::Foreign(
                owners
                    .iter()
                    .map(|&o| self.classes[o].name.clone())
                    .collect(),
            );
        }
        if universal {
            return Verdict::Universal;
        }
        if named {
            Verdict::Unattributed
        } else {
            Verdict::UnknownKey
        }
    }

    /// Judge one side of a ZDO connection on `prefab`. `type_name` is
    /// `Connection::type_name()`; `is_target` picks which side's class list
    /// applies. An unknown type, an unknown prefab, or a side the game does
    /// not constrain all come back as something other than `Foreign`, so
    /// nothing is removed on a guess.
    pub fn judge_connection(
        &self,
        prefab: i32,
        type_name: Option<&str>,
        is_target: bool,
    ) -> Verdict {
        if !self.knows_prefab(prefab) {
            return Verdict::UnknownPrefab;
        }
        let Some(name) = type_name else {
            return Verdict::UnknownKey;
        };
        let Some((source, target)) = self.connections.get(name) else {
            return Verdict::UnknownKey;
        };
        let allowed = if is_target { target } else { source };
        if allowed.is_empty() {
            return Verdict::Universal;
        }
        match allowed.iter().find(|&&c| self.prefab_has(prefab, c)) {
            Some(&c) => Verdict::Owned(self.classes[c].name.clone()),
            None => Verdict::Foreign(
                allowed
                    .iter()
                    .map(|&c| self.classes[c].name.clone())
                    .collect(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_match_digits_and_wildcards() {
        assert!(Pattern("#_durability".into()).matches("3_durability"));
        assert!(!Pattern("#_durability".into()).matches("_durability"));
        assert!(Pattern("slot#".into()).matches("slot12"));
        assert!(!Pattern("slot#".into()).matches("slotstatus1"));
        assert!(Pattern("b_*".into()).matches("b_Greydwarf21"));
        assert!(Pattern("*.m_*".into()).matches("WearNTear.m_health"));
    }

    #[test]
    fn chest_owns_items_but_not_tamed() {
        let s = Schema::builtin();
        let chest = stable_hash("piece_chest_wood");
        assert_eq!(
            s.judge(chest, stable_hash("items"), Some("items")),
            Verdict::Owned("Container".into())
        );
        assert!(matches!(
            s.judge(chest, stable_hash("tamed"), Some("tamed")),
            Verdict::Foreign(_)
        ));
        let boar = stable_hash("Boar");
        assert_eq!(
            s.judge(boar, stable_hash("tamed"), Some("tamed")),
            Verdict::Owned("Character".into())
        );
        assert_eq!(
            s.judge(boar, stable_hash("TameTimeLeft"), None),
            Verdict::Owned("Tameable".into())
        );
        let coins = stable_hash("Coins");
        assert_eq!(
            s.judge(coins, stable_hash("data_0"), Some("data_0")),
            Verdict::Owned("ItemDrop".into())
        );
        assert_eq!(
            s.judge(stable_hash("NoSuchPrefab"), stable_hash("tamed"), None),
            Verdict::UnknownPrefab
        );
    }
}
