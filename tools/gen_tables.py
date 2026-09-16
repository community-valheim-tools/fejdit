#!/usr/bin/env python3
"""Regenerate the hash/name and schema tables under data/ from a decompiled
Valheim reference tree (Scripts/ with the C# sources, Assets/ with prefab
YAML whose `m_Script:` lines carry `# ClassName (assembly_valheim)` comments).

    tools/gen_tables.py path/to/decompiled-valheim [output-dir]

Writes: prefab_names.txt, creature_prefabs.txt, zdo_keys.txt,
prefab_components.txt, component_keys.txt, anim_params.txt. With no output
directory these land in data/; era_table_diff.py passes one so it can build
the tables for an older game version without disturbing the checked-in set.
component_key_patterns.txt and connection_types.txt are hand-maintained and
left alone.
"""

import os
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
DATA = HERE.parent / "data"  # overridden by main()

CREATURE_CLASSES = ("Character", "Humanoid", "Player", "Tameable", "MonsterAI", "AnimalAI", "BaseAI", "Procreation")
SPAWN_LIST_CLASSES = {"SpawnSystemList", "RandEventSystem", "LocationList"}
SCRIPT_COMMENT = re.compile(r"m_Script:.*# (\w+) \((assembly_valheim|assembly_utils|Assembly-CSharp)\)")
# AssetRipper annotates every asset reference with its path, so a spawn list's
# entries can be read without resolving GUIDs.
PREFAB_REF = re.compile(r"m_prefab: \{[^}]*\}\s*#\s*\S*/(\w+)\.prefab")
SYNC_LIST = re.compile(r"^\s*m_sync(?:Bools|Floats|Ints):\s*$")
SYNC_ITEM = re.compile(r"^\s*-\s*(\w+)\s*$")
# Animator parameter names passed by name in the game code. The receiver has
# to look like an animation object, or the same call shapes pick up settings
# keys such as GetBool("AntiAliasing").
# Animator parameters older builds synced that the current code no longer
# names. `anim_speed` was ZSyncAnimation.s_animSpeedID until 1.0, which strips
# it from pre-1.0 worlds on conversion; every creature saved before then still
# carries it, and an unnameable key can never be judged.
LEGACY_ANIM_PARAMS = {"anim_speed"}
ANIM_CALL = re.compile(
    r'(?:\w*[Aa]nim\w*\.(?:SetBool|SetFloat|SetInt|SetTrigger|ResetTrigger|GetBool|GetFloat|GetInt)'
    r'|GetHash)\(\s*"(\w+)"'
)
CLASS_DECL = re.compile(r"^public (?:abstract |static |sealed |partial )*class (\w+)(?:\s*:\s*([\w.]+))?", re.M)


def read(path):
    with open(path, errors="ignore") as fh:
        return fh.read()


def gen_prefabs(assets):
    components = {}
    spawned = set()   # prefabs a SpawnSystemList can spawn
    anim_params = set()
    for root, _dirs, files in os.walk(assets):
        for f in files:
            if not f.endswith(".prefab"):
                continue
            comps = set()
            refs = set()
            lines = open(os.path.join(root, f), errors="ignore").read().splitlines()
            for i, line in enumerate(lines):
                m = SCRIPT_COMMENT.search(line)
                if m:
                    comps.add(m.group(1))
                m = PREFAB_REF.search(line)
                if m:
                    refs.add(m.group(1))
                if SYNC_LIST.match(line):
                    for j in range(i + 1, len(lines)):
                        item = SYNC_ITEM.match(lines[j])
                        if not item:
                            break
                        anim_params.add(item.group(1))
            components[f[:-7]] = sorted(comps)
            # Every asset that holds SpawnData: the biome spawn lists, the
            # event lists behind "e_" keys, and the per-location lists.
            if comps & SPAWN_LIST_CLASSES:
                spawned |= refs
    names = sorted(components)
    (DATA / "prefab_names.txt").write_text("\n".join(names) + "\n")
    # SpawnSystem keys are "b_"/"e_" + prefab name + list index, and a
    # SpawnSystemList is not limited to things with creature components: the
    # base list carries Fish1, Seagal and odin, whose keys would otherwise
    # stay unnamed and therefore unjudgeable.
    creatures = sorted(
        {p for p in names if any(c in CREATURE_CLASSES for c in components[p])}
        | (spawned & set(names))
    )
    (DATA / "creature_prefabs.txt").write_text("\n".join(creatures) + "\n")
    gen_anim_params(assets.parent / "Scripts", anim_params)
    with open(DATA / "prefab_components.txt", "w") as fh:
        fh.write("# prefab<TAB>comma-separated MonoBehaviour classes found anywhere in the prefab (generated from game assets)\n")
        for p in names:
            fh.write(f"{p}\t{','.join(components[p])}\n")
    print(f"{len(names)} prefabs, {len(creatures)} creatures")


def gen_anim_params(scripts, from_prefabs):
    """Animator parameter names, whose ZDO keys are 438569 + CRC32(name)."""
    params = set(from_prefabs) | LEGACY_ANIM_PARAMS
    for root, _dirs, files in os.walk(scripts):
        for f in files:
            if not f.endswith(".cs"):
                continue
            src = read(os.path.join(root, f))
            if "ZSyncAnimation" not in src and "Animator" not in src:
                continue
            params.update(ANIM_CALL.findall(src))
    (DATA / "anim_params.txt").write_text("\n".join(sorted(params)) + "\n")
    print(f"{len(params)} animator parameters")


def gen_keys(scripts):
    zdovars = read(scripts / "assembly_valheim" / "ZDOVars.cs")
    var_names = {m.group(1): m.group(2) for m in re.finditer(r'public static readonly int (s_\w+) = "([^"]*)"', zdovars)}
    pairs = {
        m.group(1): (m.group(2), m.group(3))
        for m in re.finditer(
            r'KeyValuePair<int, int> (s_\w+) = new KeyValuePair<int, int>\("([^"]+)"\.GetStableHashCode\(\), "([^"]+)"\.GetStableHashCode\(\)\)',
            zdovars,
        )
    }
    classes = {}  # name -> (base, keys)
    all_keys = set(var_names.values())
    for a, b in pairs.values():
        all_keys.update((a, b))
    for root, _dirs, files in os.walk(scripts):
        for f in files:
            if not f.endswith(".cs"):
                continue
            src = read(os.path.join(root, f))
            decls = list(CLASS_DECL.finditer(src))
            for i, cm in enumerate(decls):
                cls, base = cm.group(1), cm.group(2)
                body = src[cm.end() : decls[i + 1].start() if i + 1 < len(decls) else len(src)]
                keys = set()
                for m in re.finditer(r"ZDOVars\.(s_\w+)", body):
                    n = m.group(1)
                    if n in var_names:
                        keys.add(var_names[n])
                    elif n in pairs:
                        keys.update(pairs[n])
                for m in re.finditer(r'\.(?:Get(?:Int|Float|Long|String|Bool|Vec3|Quaternion|ByteArray|ZDOID)|Set|Remove\w*)\(\s*"([^"]+)"', body):
                    keys.add(m.group(1))
                for m in re.finditer(r'GetHashZDOID\("([^"]+)"\)', body):
                    keys.update((m.group(1) + "_u", m.group(1) + "_i"))
                for m in re.finditer(r'"([^"]*)"\.GetStableHashCode\(\)', body):
                    keys.add(m.group(1))
                # Key names held in a string constant and passed by variable,
                # e.g. RandomMaterialValues' s_randSeedString = "RandMatSeed".
                # Only counted when the constant is really handed to a ZDO
                # accessor, or every unrelated const string joins the table.
                for m in re.finditer(r'(?:const|static readonly)\s+string\s+(\w+)\s*=\s*"([^"]+)"', body):
                    if re.search(r'(?:Set|Get\w*|Remove\w*)\(\s*' + re.escape(m.group(1)) + r'\b', body):
                        keys.add(m.group(2))
                # An interpolated key such as $"{arg}data_{i}" is scraped as
                # its literal source text. The real keys it stands for are
                # indexed ones, which component_key_patterns.txt covers with
                # data_# and #_data_#, so the placeholder is only noise.
                keys = {k for k in keys if "{" not in k and "}" not in k}
                all_keys.update(keys)
                prev_base, prev_keys = classes.get(cls, (None, set()))
                classes[cls] = (base or prev_base, prev_keys | keys)
    with open(DATA / "component_keys.txt", "w") as fh:
        fh.write("# class<TAB>base class<TAB>comma-separated ZDO keys the class reads or writes (generated from game code)\n")
        for c in sorted(classes):
            base, keys = classes[c]
            if keys or (base and base != "MonoBehaviour"):
                fh.write(f"{c}\t{base or ''}\t{','.join(sorted(keys))}\n")
    (DATA / "zdo_keys.txt").write_text("\n".join(sorted(k for k in all_keys if k)) + "\n")
    print(f"{len(classes)} classes, {sum(1 for c in classes.values() if c[1])} with keys, {len(all_keys)} keys")


def main():
    if len(sys.argv) not in (2, 3):
        sys.exit(__doc__)
    global DATA
    ref = Path(sys.argv[1]).expanduser()
    if len(sys.argv) == 3:
        DATA = Path(sys.argv[2]).expanduser()
        DATA.mkdir(parents=True, exist_ok=True)
    gen_prefabs(ref / "Assets")
    gen_keys(ref / "Scripts")


if __name__ == "__main__":
    main()
