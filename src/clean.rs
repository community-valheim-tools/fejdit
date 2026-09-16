//! Rules for removing properties that should not be on a ZDO, and a report of
//! what was removed.
//!
//! Background: before game build 0.218.21 (2024-08-19) the static per-ZDO
//! property store was not reset between sessions and deserializing a
//! networked ZDO did not clear what was already stored under its id. A client
//! carrying leftovers from a previous world merged them into whatever objects
//! it took ownership of, and the server then saved the union. The result is
//! arbitrary properties from unrelated objects on, for example, every building
//! piece near a player who was taming an animal.
//!
//! Two kinds of rule exist:
//! * `Stale`: keys the game itself drops on load (`ZDOHelper.s_stripOldData`
//!   and friends) plus the empty values `ZDO.Strip` discards. Removing them is
//!   always safe because the game would do it anyway.
//! * `Foreign`: keys that belong to component classes the object's prefab
//!   does not have (see `schema`), and ZDO connections whose type no
//!   component of the prefab can establish or receive. Keys this tool cannot
//!   name, connection types it does not know, keys used only by non-component
//!   code, and prefabs it does not know are left alone.
//!
//! Connections are covered because they leaked too: the 0.218.21 fix added
//! `s_connectionsHashData.Clear()` to `ZDOExtraData.Reset()` and a call to
//! `Reset()` in `ZDOMan.ShutDown`, so before it `s_connections` survived
//! across sessions alongside the typed stores.

use std::collections::{BTreeMap, HashSet};

use serde::Serialize;

use crate::format::primitives::Quat;
use crate::format::zdo::{Connection, PropKind, PropValue, Zdo};
use crate::hash::stable_hash;
use crate::names::NameTable;
use crate::schema::{Schema, Verdict};

/// Keys the game strips from every ZDO on load (`ZDOHelper.s_stripOldData`).
pub const STALE_KEYS: &[&str] = &[
    "generated",
    "patrolSpawnPoint",
    "autoDespawn",
    "targetHear",
    "targetSee",
    "burnt0",
    "burnt1",
    "burnt2",
    "burnt3",
    "burnt4",
    "burnt5",
    "burnt6",
    "burnt7",
    "burnt8",
    "burnt9",
    "burnt10",
    "LookDir",
    "RideSpeed",
];

/// Legacy ZDOID pairs stripped from the long table (`s_stripOldLongData`).
pub const STALE_LONG_KEYS: &[&str] = &[
    "user_u",
    "user_i",
    "RodOwner_u",
    "RodOwner_i",
    "CatchID_u",
    "CatchID_i",
];

/// Keys stripped from the byte-array table (`s_stripOldDataByteArray`).
pub const STALE_BYTE_ARRAY_KEYS: &[&str] = &["health"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub enum RuleId {
    /// Keys the game discards on load anyway.
    Stale,
    /// Keys belonging to components the prefab does not have.
    Foreign,
}

impl RuleId {
    pub fn label(self) -> &'static str {
        match self {
            RuleId::Stale => "stale keys the game strips on load",
            RuleId::Foreign => "keys and connections the object cannot have",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Report {
    pub zdos_scanned: usize,
    pub properties_removed: usize,
    pub zdos_touched: usize,
    pub by_rule: BTreeMap<RuleId, usize>,
    /// prefab name -> `kind:key` (or `conn:<type>`) -> count
    pub by_prefab_key: BTreeMap<String, BTreeMap<String, usize>>,
    /// Named keys that no known class uses, seen while scanning (never removed).
    pub unattributed_keys: BTreeMap<String, usize>,
    /// ZDOs whose prefab is not in the component table (never touched).
    pub unknown_prefab_zdos: usize,
}

/// Which rules to apply.
#[derive(Clone, Debug)]
pub struct Options {
    pub stale: bool,
    pub foreign: bool,
    /// Key names, and connection type labels such as `Portal`, that are never
    /// removed.
    pub allow_keys: Vec<String>,
    /// Record per-prefab, per-key counts in the report.
    pub detailed: bool,
}

struct Rules<'a> {
    stale: HashSet<i32>,
    stale_long: HashSet<i32>,
    stale_bytes: HashSet<i32>,
    allow: HashSet<i32>,
    /// `allow_keys` verbatim, for matching connection labels by name.
    allow_names: HashSet<String>,
    schema: &'a Schema,
    names: &'a NameTable,
}

/// One thing the pass took off a ZDO, kept cheap so labels are only built
/// when the report asks for them.
enum Removed {
    Prop(PropKind, i32),
    Conn(Connection),
}

impl Removed {
    fn label(&self, names: &NameTable) -> String {
        match self {
            Removed::Prop(kind, key) => format!("{}:{}", kind.label(), names.display(*key)),
            Removed::Conn(conn) => format!("conn:{}", conn.label()),
        }
    }
}

impl Rules<'_> {
    /// Mirrors `ZDO.Strip*` for one property.
    fn stale(&self, kind: PropKind, key: i32, value: PropValue<'_>) -> bool {
        if self.stale.contains(&key) {
            return true;
        }
        match (kind, value) {
            (PropKind::Long, _) => self.stale_long.contains(&key),
            (PropKind::Quat, PropValue::Quat(q)) => *q == Quat::IDENTITY,
            (PropKind::String, PropValue::String(s)) => s.is_empty(),
            (PropKind::ByteArray, PropValue::ByteArray(b)) => {
                b.is_empty() || self.stale_bytes.contains(&key)
            }
            _ => false,
        }
    }

    fn judge(&self, prefab: i32, key: i32) -> Verdict {
        self.schema.judge(prefab, key, self.names.resolve(key))
    }

    /// Whether a connection does not belong on this prefab. Anything the
    /// schema cannot speak to, and anything the user protected by name, stays.
    fn connection_is_foreign(&self, prefab: i32, conn: Connection) -> bool {
        if self.allow_names.contains(&conn.label())
            || conn
                .type_name()
                .is_some_and(|n| self.allow_names.contains(n))
        {
            return false;
        }
        matches!(
            self.schema
                .judge_connection(prefab, conn.type_name(), conn.is_target()),
            Verdict::Foreign(_)
        )
    }
}

/// Apply the rules to every ZDO in place and describe what was removed.
/// Takes the objects however they are held: a `&mut Vec<Zdo>`, or
/// `WorldData::zdos_mut()` for either world layout.
pub fn clean<'a>(
    zdos: impl IntoIterator<Item = &'a mut Zdo>,
    names: &NameTable,
    schema: &Schema,
    opts: &Options,
) -> Report {
    let hashes = |keys: &[&str]| keys.iter().map(|k| stable_hash(k)).collect::<HashSet<_>>();
    let rules = Rules {
        stale: hashes(STALE_KEYS),
        stale_long: hashes(STALE_LONG_KEYS),
        stale_bytes: hashes(STALE_BYTE_ARRAY_KEYS),
        allow: opts.allow_keys.iter().map(|k| stable_hash(k)).collect(),
        allow_names: opts.allow_keys.iter().cloned().collect(),
        schema,
        names,
    };
    let mut report = Report::default();
    for zdo in zdos {
        report.zdos_scanned += 1;
        let prefab = zdo.prefab;
        if opts.foreign && !schema.knows_prefab(prefab) {
            report.unknown_prefab_zdos += 1;
        }
        let mut removals: Vec<(RuleId, Removed)> = Vec::new();
        let mut unattributed: Vec<i32> = Vec::new();
        let mut removed = zdo.retain_properties(|kind, key, value| {
            if rules.allow.contains(&key) {
                return true;
            }
            let rule = if opts.stale && rules.stale(kind, key, value) {
                Some(RuleId::Stale)
            } else if opts.foreign {
                match rules.judge(prefab, key) {
                    Verdict::Foreign(_) => Some(RuleId::Foreign),
                    Verdict::Unattributed => {
                        unattributed.push(key);
                        None
                    }
                    _ => None,
                }
            } else {
                None
            };
            match rule {
                Some(r) => {
                    removals.push((r, Removed::Prop(kind, key)));
                    false
                }
                None => true,
            }
        });
        // The connection sits outside the property tables (`ZDO.Save` writes
        // it from its own store), so `retain_properties` cannot see it.
        if opts.foreign
            && let Some(conn) = zdo.connection
            && rules.connection_is_foreign(prefab, conn)
        {
            zdo.connection = None;
            removals.push((RuleId::Foreign, Removed::Conn(conn)));
            removed += 1;
        }
        for key in unattributed {
            *report
                .unattributed_keys
                .entry(names.display(key))
                .or_default() += 1;
        }
        if removed > 0 {
            report.zdos_touched += 1;
            report.properties_removed += removed;
            let prefab_name = opts.detailed.then(|| names.display(prefab));
            for (rule, what) in removals {
                *report.by_rule.entry(rule).or_default() += 1;
                if let Some(prefab) = &prefab_name {
                    *report
                        .by_prefab_key
                        .entry(prefab.clone())
                        .or_default()
                        .entry(what.label(names))
                        .or_default() += 1;
                }
            }
        }
    }
    report
}
