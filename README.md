# fejdit

Command line tool for querying and editing Valheim (Fejd) save files:
worlds, in both the `.fwl` + `.db` pair of builds up to 0.221 and the
world directory of game 1.0, and characters (`.fch`).

```
fejdit world info   <WORLD> [--json] [--meta-only]
fejdit world rename <WORLD> <NEW NAME> (--in-place | --output DIR) [--keep-file-name] [--upgrade]
fejdit world find   <WORLD> [--item GLOB]... [--crafter GLOB] [--json]
fejdit world clean-netobj-corruption <WORLD> (--in-place | --output FILE|DIR | --dry-run) [--detailed] [--upgrade]
fejdit world keys   <WORLD> [--prefab GLOB] [--key NAME]
fejdit world fix-armor-stands <OLD WORLD> <NEW WORLD> (--in-place | --output DIR | --dry-run) [--detailed]
fejdit character info   <FILE.fch> [--json] [--all-stats]
fejdit character rename <FILE.fch> <NEW NAME> (--in-place | --output FILE)
```

`<WORLD>` is the path to either file of the pair, or the bare stem, or a 1.0
world directory (or any `_main.<N>.*` or `.chunk` file inside one); the rest
is found next to it.

Mutating commands never overwrite anything unless told to: `--output` writes
elsewhere, `--in-place` replaces the input and keeps a timestamped
`.fejdit-bak` copy unless `--no-backup` is given. Nothing that is not a
regular file is ever replaced. For a file pair, `clean-netobj-corruption`
rewrites only the `.db`, so its `--output` takes a `.db` path, or a directory
to write `<world>.db` into, and refuses a `.fwl`. For a world directory it
rewrites only the `.chunk` files whose objects changed, and `--output` names
the directory to write a complete copy of the world into.

The two `world` commands also leave the save's format alone. A world written
by an older build is read, edited and written back at its own version and in
its own layout, so editing it does not change which game builds can open it
-- `Version.IsWorldVersionCompatible` refuses anything newer than the build's
own. `--upgrade` opts in to what the game itself does the next time it saves:
for a `.fwl` + `.db` pair that is the 1.0 world directory, objects migrated
and dealt into chunks, so the output becomes a directory named after the
world (beside the pair when in place, which then retires the pair to
`.fejdit-bak` files). (`character rename` still upgrades unconditionally: an
old `.fch` needs a real migration -- the legacy counters into the stat array,
and since 1.0 the stat array into the first of ten difficulty tiers -- and
there is no version-preserving path for it yet.)

## fix-armor-stands

When game 1.0.0 through 1.0.11 loads a world saved by 0.221 or earlier it
folds every loose item's separate keys into one `itemData` blob, but only the
unindexed ones. Armor on an armor stand lives in slots, stored as
`<n>_durability`, `<n>_quality`, `<n>_crafterID` and so on. The conversion
leaves those where they are, strips each `<n>_crafterName`, and from then on
the stand reads only `<n>_itemData`, which was never written. The stand still
shows the armor, but taking it down hands back a piece at default durability,
quality 1, with no crafter. Game 1.0.12 folds the slots itself, so worlds it
converts are fine -- but the conversion runs once, when a world still in the
old layout is loaded, so it can do nothing for a world an earlier 1.0 build
has already converted.

There is a second, older gap that no build has closed. A stand has up to 14
slots, and `ConvertPrefabStrings` hashes only `0_item` through `5_item`, so a
slot past the sixth comes out of the conversion still naming its item with a
string. `ArmorStand` reads that key as an int, sees nothing, and the armor is
invisible and cannot be taken down at all. The command converts those keys
too, whether or not it has a blob to restore for the slot.

The command takes the world as it was before the conversion (the backup the
game made, or your own copy of the `.fwl` + `.db` pair) and the converted
world directory, matches stands by prefab and position and slots by the item
they hold, and writes each slot's blob from the old keys: prefab, durability,
quality, variant, crafter id and name, custom data. A slot that holds a
different item now, or already has a blob because the item was placed again
since, is left alone, and so is a stand that has moved. Both worlds must have
the same uid. Only the chunk files holding a repaired stand are rewritten.

`--upgrade` on the other commands does both steps itself, so a world converted
by this tool rather than by the game never needs this one.

## The two world layouts

Up to game 0.221 (world version 39) a world is `<name>.fwl` beside
`<name>.db`, the latter one stream of every object. Game 1.0 (world version
40 and up) keeps a world in `worlds_local/<name>/`:

- `_main.<N>.fwl2`: the metadata, the old `.fwl` payload plus the list of
  players who have joined.
- `_main.<N>.db2`: net time, the zone system (gzip-compressed), the random
  event and the persistent-event blob.
- `_main.<N>.chunks`: which `.chunk` files make up the world and how many
  objects each holds.
- `<yy>_<xx>__<size>_<version>.chunk`: the objects of one chunk of the 512 x
  512 zone map. A size-0 chunk is 8 x 8 zones and each size up doubles the
  side; the game merges quiet regions into bigger chunks and keeps portals in
  a chunk of their own. Only the chunks that changed are rewritten on save,
  each under a new version number.
- `_main.<N>.ok`: written last; its presence means save `N` completed.

`N` is the save number, bumped on every save; the tool reads the highest `N`
that has its `.ok` marker. Inside a chunk the object record is smaller than
before: no stored sector, a position of two shorts when it has no height and
whole-number coordinates, and a rotation packed into half-degree steps.
Containers hold their inventory as a byte array rather than a base64 string,
loose items pack their stack, durability and crafter into one `itemData`
blob, and item stands name their item by hash; `--upgrade` performs those
migrations exactly as `ZDOMan` does when it loads an old world, and `find`
reads both forms. It goes one step further than the game for armor stands:
1.0 converts only the unindexed item keys and strips each slot's
`<n>_crafterName`, so armor taken off a stand in a game-converted world comes
back with default stats and no crafter, while `--upgrade` folds every slot
into the `<n>_itemData` blob 1.0 reads.

Since a gzip stream depends on who wrote it, a `.db2` written here decodes
to the same data as the game's without being the same bytes. Everything else
round-trips byte for byte.

## clean-netobj-corruption

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

## Layout

- `src/format/` – declarative `binrw` schemas for each file. Version-dependent
  fields are `Option`s gated on named constants from `versions.rs`, so the
  structs double as documentation of when each field appeared. `db.rs` and
  `fwl.rs` are the file pair, `chunked.rs` the 1.0 directory (with the
  chunk-dealing logic of `ZDOMan.GetSaveClonePerChunk`), `migrate.rs` the
  object migrations 1.0 applies to an old world, and `world.rs` the enum
  commands work on so they need not care which layout they got.
- `src/clean.rs` – the property-removal rules behind `clean-netobj-corruption`.
- `src/names.rs` + `data/` – hash-to-name tables. Prefab names come from the
  game's asset tree, property keys from the game code, animator parameters
  from both. Derived forms the game builds at run time (`key_u`, `3_key`,
  `slot0`, `b_Boar2`) are generated from the base names. Names in `data/` can
  be supplemented at run time with `--names FILE`.
- `src/hash.rs` – the two hashes that become ZDO keys: Valheim's
  `GetStableHashCode` and, for `ZSyncAnimation`, Unity's
  `Animator.StringToHash`.

## Building

```
cargo build --release
```

Requires Rust 1.88 or newer. The result is a single static binary in
`target/release/fejdit`.
