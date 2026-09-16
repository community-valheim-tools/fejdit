# fejdit

Command line tool for querying and editing Valheim (Fejd) save files:
worlds, in both the `.fwl` + `.db` pair of builds up to 0.221 and the world
directory of game 1.0, and characters (`.fch`).

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

`<WORLD>` is either file of the pair, the bare stem, or a 1.0 world directory
(or any `_main.<N>.*` or `.chunk` file inside one); the rest is found next to
it.

## Writing

Mutating commands never overwrite anything unless told to. `--output` writes
elsewhere; `--in-place` replaces the input, keeping a timestamped
`.fejdit-bak` copy unless `--no-backup` is given. Nothing but a regular file
is ever replaced.

`clean-netobj-corruption` rewrites only what changed. For a file pair that is
the `.db` alone, so its `--output` takes a `.db` path or a directory to write
`<world>.db` into, and refuses a `.fwl`. For a world directory it is only the
`.chunk` files whose objects changed, and `--output` names a directory to
hold a complete copy of the world.

`world rename` and `world clean-netobj-corruption` also leave the save's
format alone. A world is read, edited and written back at its own version and
layout, so editing it never changes which builds can open it
(`Version.IsWorldVersionCompatible` refuses anything newer than the build's
own). `--upgrade` opts into what the game does on its next save: a `.fwl` +
`.db` pair becomes the 1.0 world directory, objects migrated and dealt into
chunks, named after the world and written beside the pair when in place,
which retires the pair to `.fejdit-bak` files.

`character rename` always upgrades. An old `.fch` needs a real migration, the
legacy counters into the stat array and since 1.0 the stat array into the
first of ten difficulty tiers, and there is no version-preserving path for it
yet.

## Repairs

Two commands fix a game bug rather than editing anything you chose:

- `clean-netobj-corruption` removes properties and connections that do not
  belong on the object carrying them, left by a bug in builds before 0.218.21
  that merged one object's stored data into another. `--dry-run` reports
  without writing; `--detailed` breaks the count down by prefab and key.
- `fix-armor-stands` restores the armor-stand item data that game 1.0.0
  through 1.0.11 drops when it converts an older world, and converts the slots
  past the sixth that no build has ever converted. It takes the world as it
  was before the conversion and the converted one. A world converted with
  `--upgrade` here never needs it.

`world keys` lists what a prefab carries, connections as `conn:Portal` or
`conn:Portal|Target` beside the property keys; `--key` filters on property
keys only. `--names FILE` supplements the tables in `data/`.

## Building

```
cargo build --release
```

Requires Rust 1.88 or newer. The result is a single static binary in
`target/release/fejdit`.

## Documentation

- [The two world layouts](docs/world-formats.md): the `.fwl` + `.db` pair, the
  1.0 world directory and its chunk files, and the migrations between them.
- [clean-netobj-corruption](docs/clean-netobj-corruption.md): the game bug, the
  two rule sets, and the name tables the command judges keys against.
- [fix-armor-stands](docs/fix-armor-stands.md): both conversion gaps and how
  the command matches stands and slots.
- [Internals](docs/internals.md): how the source is arranged.
