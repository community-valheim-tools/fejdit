# clean-netobj-corruption

Before game build 0.218.21 (2024-08-19) the per-object property store was not
reset between sessions, and receiving an object over the network did not clear
what was already stored under its id. A client carrying leftovers from a
previous world merged them into whatever it took ownership of, and the server
saved the union: building pieces near a player taming an animal ended up with
that animal's (or any other object's) properties.

The command removes properties and connections that do not belong on the
object carrying them and reports how many were removed from how many objects
(`--detailed` breaks it down by prefab and key). Two rule sets, each
switchable off:

- **stale**: keys the game itself discards on load (`ZDOHelper.s_stripOldData`)
  and empty values (`""`, identity rotations, empty byte arrays).
- **foreign**: keys that belong to component classes the object's prefab does
  not have. `data/prefab_components.txt` maps prefabs to their components
  (from the game's prefab assets) and `data/component_keys.txt` maps classes
  to the keys they use (from the game code, base classes included);
  `data/component_key_patterns.txt` covers keys built at run time such as
  `3_durability`. Keys the tool cannot name, keys used only by non-component
  code, and prefabs it does not know are never touched, so modded data
  survives. `--allow-key NAME` protects a key outright.

  Not every key is a `GetStableHashCode` of a name. `ZSyncAnimation` stores
  each synced animation parameter under `438569 + CRC32(name)` (Unity's
  `Animator.StringToHash`), so those keys resolve through
  `data/anim_params.txt` instead and are attributed to `ZSyncAnimation`.
  Without that, every animation parameter on every creature is an unnameable
  hash, and an unnameable key can never be judged: on a 1.8M-object world this
  one family accounted for 176k of the 179k unattributable properties.

  The same rule covers ZDO connections, which the game keeps in the same
  leaked store and saves alongside the property tables.
  `data/connection_types.txt` lists the classes allowed on each side of each
  connection type: a `Portal` on anything without `TeleportWorld` goes, and so
  does a `Spawned` on anything without `CreatureSpawner`. Where the game does
  not constrain the far end -- a spawner's target is matched only on
  `ZNetView`, and `ZSyncTransform` attaches to any networked parent -- the
  side is listed empty and never removed. Connection types the tool does not
  know are left alone. `--allow-key Portal` protects a connection type by
  name, and `--keep-foreign` turns the whole rule off.

Regenerate the tables for a new game version with
`tools/gen_tables.py <reference tree> [output dir]`; `fejdit world keys` shows what a
prefab carries before and after, listing connections as `conn:Portal`,
`conn:Portal|Target` and so on beside the property keys. `--key` filters on
property keys only.

The tables are one snapshot of one game version, but the command is pointed at
saves from older builds. That only risks real data in two ways: a prefab has
*lost* a component that owned a key an older save still carries, or a
component has stopped mentioning a key that another component still uses.
1.0 did the second with `ItemDrop`'s per-key item data, which is why
`component_key_patterns.txt` pins those keys to `ItemDrop` by hand. Every
other kind of drift lands on `UnknownPrefab` or `Unattributed`, both of which
are kept. `tools/era_table_diff.py <reference repo> <commit-ish> [--save
WORLD.db]` regenerates the tables from an older commit and reports both
kinds of pair, and with `--save` runs the cleaner under both sets of tables
and diffs what each would remove. It exits non-zero when the current tables would strip something
the older ones considered legitimate. Worth running after regenerating the
tables for a new game version.
