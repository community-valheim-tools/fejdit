# fix-armor-stands

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
