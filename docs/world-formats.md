# The two world layouts

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
