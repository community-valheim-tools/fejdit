# Internals

How the source is arranged, for anyone reading or extending it.

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
