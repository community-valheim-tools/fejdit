#!/usr/bin/env python3
"""Check the schema tables in data/ against an older game version.

    tools/era_table_diff.py path/to/ReferenceRepo <commit-ish> [--save WORLD.db]

The tables under data/ are one snapshot of one game version, but
`world clean-netobj-corruption` applies them to saves written by older builds.
That is safe by construction in every direction but one. Only `Verdict::Foreign`
removes: a prefab missing from the tables makes the whole ZDO `UnknownPrefab`
and untouched, and a key whose owning class is gone becomes `Unattributed` and
is kept. The exception is a prefab that has *lost* a component which owned a
key an older save still legitimately carries -- that key looks foreign now and
would be stripped.

This regenerates the tables from <commit-ish> and reports exactly those pairs,
restricted to prefabs that persist as ZDOs (the ones with a ZNetView), since
nothing else is ever in a .db. Non-zero exit means a real pair was found.

With --save it also runs the cleaner itself over one world with each set of
tables and diffs the removals, which catches anything the structural pass
misses. That has to swap data/ and rebuild, so it is opt-in; the originals are
restored even when the run fails.
"""

import argparse
import filecmp
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
DATA = ROOT / "data"

# Written by gen_tables.py; the rest of data/ is hand-maintained.
GENERATED = (
    "prefab_names.txt",
    "creature_prefabs.txt",
    "zdo_keys.txt",
    "prefab_components.txt",
    "component_keys.txt",
    "anim_params.txt",
)

# A prefab only reaches a .db if it has one of these, so a component lost from
# anything else (menus, weather, particle effects) cannot cost a save any data.
PERSISTED_BY = "ZNetView"


def run(cmd, **kw):
    """Run `cmd`, showing its stderr only if it fails."""
    kw.setdefault("stderr", subprocess.PIPE)
    kw.setdefault("text", True)
    proc = subprocess.run(cmd, **kw)
    if proc.returncode != 0:
        sys.stderr.write(proc.stderr or "")
        sys.exit(f"failed: {' '.join(str(c) for c in cmd)}")
    return proc


def extract_era(repo, commit, dest):
    """The prefab YAML and C# sources at `commit`, which is all gen_tables reads."""
    print(f"extracting {commit} from {repo} ...", flush=True)
    archive = subprocess.Popen(
        ["git", "-C", str(repo), "archive", commit, "Assets", "Scripts"],
        stdout=subprocess.PIPE,
    )
    untar = subprocess.Popen(
        ["tar", "-x", "-C", str(dest), "--include=*.prefab", "--include=*.cs"],
        stdin=archive.stdout,
    )
    archive.stdout.close()
    if untar.wait() != 0 or archive.wait() != 0:
        sys.exit(f"could not extract {commit} from {repo}")


def load_prefab_components(path):
    out = {}
    for line in Path(path).read_text().splitlines():
        if line.startswith("#") or not line.strip():
            continue
        name, _, comps = line.partition("\t")
        out[name] = set(comps.split(",")) if comps else set()
    return out


def load_key_owners(path):
    """key -> the classes that read or write it."""
    out = {}
    for line in Path(path).read_text().splitlines():
        if line.startswith("#") or not line.strip():
            continue
        fields = line.split("\t")
        if len(fields) >= 3 and fields[2]:
            for key in fields[2].split(","):
                out.setdefault(key, set()).add(fields[0])
    return out


def load_hand_patterns(path):
    """Exact keys the hand-maintained pattern table pins to a class; the
    wildcard patterns are left to the cleaner itself."""
    out = {}
    for line in Path(path).read_text().splitlines():
        if line.startswith("#") or not line.strip():
            continue
        cls, _, pattern = line.partition("\t")
        pattern = pattern.strip()
        if "#" not in pattern and "*" not in pattern:
            out.setdefault(pattern, set()).add(cls)
    return out


def structural_diff(era_dir):
    now_pc = load_prefab_components(DATA / "prefab_components.txt")
    era_pc = load_prefab_components(era_dir / "prefab_components.txt")
    now_ko = load_key_owners(DATA / "component_keys.txt")
    era_ko = load_key_owners(era_dir / "component_keys.txt")
    hand = load_hand_patterns(DATA / "component_key_patterns.txt")
    for key, classes in hand.items():
        now_ko.setdefault(key, set()).update(classes)

    def persisted(name):
        return PERSISTED_BY in era_pc.get(name, ()) or PERSISTED_BY in now_pc.get(name, ())

    print(f"\nprefabs: era={len(era_pc)} current={len(now_pc)}")

    gone = sorted(p for p in set(era_pc) - set(now_pc) if persisted(p))
    print(f"persisted prefabs that existed then and are gone now: {len(gone)}")
    for p in gone:
        print(f"   {p}  (-> UnknownPrefab, the whole ZDO is skipped)")

    lost = {p: era_pc[p] - now_pc[p] for p in set(era_pc) & set(now_pc) if era_pc[p] - now_pc[p]}
    persisted_lost = {p: c for p, c in lost.items() if persisted(p)}
    print(
        f"prefabs that lost a component: {len(lost)}"
        f" ({len(persisted_lost)} of them persisted)"
    )
    for p, comps in sorted(persisted_lost.items()):
        print(f"   {p}: -{sorted(comps)}")

    # The pairs that matter: legitimate under the era tables, Foreign under
    # the current ones, on a prefab that can actually be in a save.
    risky = {}
    for prefab, comps in lost.items():
        if not persisted(prefab):
            continue
        for comp in comps:
            for key, era_owners in era_ko.items():
                if comp in era_owners and not (now_ko.get(key, set()) & now_pc[prefab]):
                    risky.setdefault(prefab, set()).add(key)

    total = sum(len(v) for v in risky.values())
    print(f"\n{'FOUND' if total else 'none'}: (prefab, key) pairs legitimate then, Foreign now: {total}")
    for prefab, keys in sorted(risky.items()):
        print(f"   {prefab}: {sorted(keys)}")

    # The other way a key goes Foreign: the component that used it stopped
    # mentioning it (1.0 moved ItemDrop's per-key item data into one blob)
    # while some other component still does, so the old owner's prefabs now
    # fail the check. A key nobody owns any more is merely Unattributed.
    components_now = set().union(*now_pc.values()) if now_pc else set()
    components_era = set().union(*era_pc.values()) if era_pc else set()
    orphaned = {}
    for key, era_owners in era_ko.items():
        now_owners = now_ko.get(key, set())
        if not (now_owners & components_now):
            continue
        for cls in era_owners & components_era:
            if cls not in now_owners:
                orphaned.setdefault(cls, set()).add(key)
    lost_total = sum(len(v) for v in orphaned.values())
    print(
        f"{'FOUND' if lost_total else 'none'}: keys a component used then and"
        f" no longer owns now, while another component does: {lost_total}"
    )
    for cls, keys in sorted(orphaned.items()):
        print(f"   {cls}: {sorted(keys)}  (add them to component_key_patterns.txt if old saves carry them)")
    return total + lost_total


def removals(save, tables_note):
    """`by_prefab_key` from a dry run of the real cleaner."""
    print(f"  running the cleaner with {tables_note} tables ...", flush=True)
    proc = run(
        [
            "cargo", "run", "-q", "--release", "--",
            "world", "clean-netobj-corruption", str(save), "--dry-run", "--json",
        ],
        cwd=ROOT,
        stdout=subprocess.PIPE,
        text=True,
    )
    report = json.loads(proc.stdout)
    return {
        (prefab, key): count
        for prefab, keys in report["by_prefab_key"].items()
        for key, count in keys.items()
    }


def end_to_end(save, era_dir):
    backup = era_dir / "backup"
    backup.mkdir()
    for name in GENERATED:
        shutil.copy2(DATA / name, backup / name)
    # Plain copies, not copy2: the tables are compiled into the binary, and
    # cargo decides whether to rebuild by file time, so a copy that keeps an
    # old time can leave the previous tables in the binary.
    try:
        now = removals(save, "current")
        for name in GENERATED:
            shutil.copy(era_dir / name, DATA / name)
        era = removals(save, "era")
    finally:
        for name in GENERATED:
            shutil.copy(backup / name, DATA / name)
        restored = all(filecmp.cmp(DATA / n, backup / n, shallow=False) for n in GENERATED)
        print(f"  data/ restored: {restored}")
        if not restored:
            print("  !! restore data/ by hand: git checkout -- data/", file=sys.stderr)

    only_now = {k: v for k, v in now.items() if k not in era}
    only_era = {k: v for k, v in era.items() if k not in now}
    print(f"\nremoved pairs: current={len(now)} era={len(era)}")
    print(f"{'FOUND' if only_now else 'none'}: removed now but kept by the era tables: {len(only_now)}")
    for (prefab, key), count in sorted(only_now.items(), key=lambda kv: -kv[1]):
        print(f"   {count:>6}  {prefab}  {key}")
    print(f"(informational) removed by the era tables but kept now: {len(only_era)}")
    for (prefab, key), count in sorted(only_era.items(), key=lambda kv: -kv[1])[:20]:
        print(f"   {count:>6}  {prefab}  {key}")
    return len(only_now)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("reference_repo", type=Path, help="decompiled Valheim tree with git history")
    ap.add_argument("commit", help="commit-ish for the older game version to compare against")
    ap.add_argument("--save", type=Path, help="also diff real removals over this world .db")
    ap.add_argument("--keep-temp", action="store_true", help="leave the extracted tree in place")
    args = ap.parse_args()

    if args.save and not args.save.is_file():
        sys.exit(f"no such world: {args.save}")

    tmp = Path(tempfile.mkdtemp(prefix="fejdit-era-"))
    try:
        src = tmp / "src"
        src.mkdir()
        extract_era(args.reference_repo.expanduser(), args.commit, src)
        tables = tmp / "tables"
        run([sys.executable, str(HERE / "gen_tables.py"), str(src), str(tables)])

        findings = structural_diff(tables)
        if args.save:
            findings += end_to_end(args.save, tables)
    finally:
        if args.keep_temp:
            print(f"\ntemp tree left at {tmp}")
        else:
            shutil.rmtree(tmp, ignore_errors=True)

    print("\n" + ("FAIL: the current tables would strip data an older save may legitimately carry"
                  if findings else "OK: the current tables strip nothing an older save legitimately carries"))
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main())
