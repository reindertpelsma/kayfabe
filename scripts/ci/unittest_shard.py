#!/usr/bin/env python3
"""Run one shard of `python3 -m unittest discover -s DIR -p PATTERN`, so the shards run in parallel jobs.

STATUS: LIVE, 2026-11-17 (docs/CI_V3.md, "Job layout, concurrency and caches").

The shards are a partition of the test MODULES that discovery finds: every module is in exactly one shard, the
shards together run exactly the tests a plain `unittest discover` runs, and the exit status is the plain
runner's (non-zero for a failure, an error, or an import error of a test module). A shard that holds no test at
all fails too: an empty shard means the partition is wrong, not that there was nothing to check.

    python3 scripts/ci/unittest_shard.py -s scripts/ci -p 'test_*.py' --index 1 --count 3   # run shard 1 of 3 (1-based)
    python3 scripts/ci/unittest_shard.py -s scripts/ci -p 'test_*.py' --count 3 --list      # print the partition

The partition is longest-processing-time-first over the size of each module's source file (a stable, cheap proxy
for its run time), ties broken by name. It depends only on the file names and sizes, so every shard job computes
the same partition from the same checkout and nothing is shared between jobs. Adding a test file needs no edit
anywhere: it lands in the lightest shard. `scripts/ci/test_unittest_shard.py` proves the partition and the exit
status; `scripts/ci/unittest_shard_mutants.py` needs two named killers for each way of getting either wrong.
"""

from __future__ import annotations

import argparse
import sys
import unittest
from pathlib import Path


def flatten(suite):
    for item in suite:
        if isinstance(item, unittest.TestSuite):
            yield from flatten(item)
        else:
            yield item


def module_of(test) -> str:
    # A module that cannot be imported is a `_FailedTest` whose method name is the module's name.
    if type(test).__name__ == "_FailedTest":
        return test._testMethodName
    return type(test).__module__


def group_by_module(suite) -> dict[str, list]:
    modules: dict[str, list] = {}
    for test in flatten(suite):
        modules.setdefault(module_of(test), []).append(test)
    return modules


def weight(start_dir: Path, module: str) -> int:
    path = start_dir / (module.replace(".", "/") + ".py")
    try:
        return path.stat().st_size
    except OSError:
        return 1


def partition(modules: dict[str, list], count: int, start_dir: Path) -> list[list[str]]:
    """`count` lists of module names: every module once, lightest-shard-first, deterministic."""
    shards: list[list[str]] = [[] for _ in range(count)]
    load = [0] * count
    for name in sorted(modules, key=lambda m: (-weight(start_dir, m), m)):
        target = min(range(count), key=lambda i: (load[i], i))
        shards[target].append(name)
        load[target] += weight(start_dir, name)
    return [sorted(shard) for shard in shards]


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("-s", "--start-directory", default="scripts/ci")
    ap.add_argument("-p", "--pattern", default="test_*.py")
    ap.add_argument("--count", type=int, required=True, help="number of shards")
    ap.add_argument("--index", type=int, help="the shard to run, 1..count")
    ap.add_argument("--list", action="store_true", help="print the partition and exit")
    args = ap.parse_args(argv)
    if args.count < 1:
        ap.error("--count must be >= 1")
    if not args.list and not (args.index and 1 <= args.index <= args.count):
        ap.error("--index must be in 1..--count")

    start_dir = Path(args.start_directory)
    suite = unittest.TestLoader().discover(args.start_directory, pattern=args.pattern)
    modules = group_by_module(suite)
    shards = partition(modules, args.count, start_dir)
    if args.list:
        for number, names in enumerate(shards, 1):
            tests = sum(len(modules[n]) for n in names)
            print(f"shard {number}/{args.count}: {len(names)} modules, {tests} tests")
            for name in names:
                print(f"  {name}")
        return 0

    chosen = shards[args.index - 1]
    mine = unittest.TestSuite()
    for name in chosen:
        mine.addTests(modules[name])
    print(
        f"unittest_shard: shard {args.index}/{args.count} runs {len(chosen)} of {len(modules)} modules "
        f"({mine.countTestCases()} of {suite.countTestCases()} tests)",
        flush=True,
    )
    result = unittest.TextTestRunner(verbosity=1).run(mine)
    if result.testsRun == 0:
        print("unittest_shard: this shard ran no test; the partition is wrong", file=sys.stderr)
        return 1
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    sys.exit(main())
