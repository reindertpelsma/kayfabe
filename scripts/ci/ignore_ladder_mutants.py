#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation harness for scripts/ci/ignore_ladder.py (PROPOSAL, 2026-11-05).

Each mutant must be killed by at least two independently named failing tests of
scripts/ci/test_ignore_ladder.py, like the other *_mutants.py harnesses in this directory.
Run: python3 scripts/ci/ignore_ladder_mutants.py [--jobs N] [name-substring ...]  (exit 0 only if every mutant has >= 2 killers)
"""

import concurrent.futures
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
SOURCE = open(os.path.join(HERE, "ignore_ladder.py")).read()


def changed(old, new):
    assert SOURCE.count(old) == 1, old
    return SOURCE.replace(old, new)


CASES = [
    ("docker-star-crosses-slash", changed('out += "[^/]*"', 'out += ".*"')),
    (
        "parent-directory-match-dropped",
        changed(
            "for k in range(1, len(comps) + 1))\n\n\n@functools.lru_cache(maxsize=None)\ndef _dir_matches",
            "for k in range(len(comps), len(comps) + 1))\n\n\n@functools.lru_cache(maxsize=None)\ndef _dir_matches",
        ),
    ),
    (
        "doublestar-needs-one-directory",
        (
            changed(
                "for k in range(len(comps) + 1))\n    return bool",
                "for k in range(1, len(comps) + 1))\n    return bool",
            )
            if False
            else changed(
                "any(_match_segs(rest, comps[k:]) for k in range(len(comps) + 1))",
                "any(_match_segs(rest, comps[k:]) for k in range(1, len(comps) + 1))",
            )
        ),
    ),
    ("negation-message-lost", changed('"negation is not allowed"', '"not allowed"')),
    (
        "bare-name-accepted",
        changed('if not (line.startswith("/") or line.startswith("**/")):', "if False:"),
    ),
    ("trailing-slash-accepted", changed('if line.endswith("/"):', "if False:")),
    ("trailing-doublestar-accepted", changed("if i == len(segs) - 1:", "if False:")),
    ("ladder-check-inverted", changed("if line not in hi:", "if line in hi:")),
    (
        "self-ignore-never-checked",
        changed("for name, need in LADDER_SELF.items():", "for name, need in {}.items():"),
    ),
    (
        "hash-drops-mode",
        changed(
            'h.update(p + b"\\0" + m.encode() + b"\\0" + s.encode() + b"\\n")\n    return h.hexdigest(), len(keep)',
            'h.update(p + b"\\0" + s.encode() + b"\\n")\n    return h.hexdigest(), len(keep)',
        ),
    ),
    ("hash-unsorted", changed("    keep.sort()\n", "")),
    ("hash-keeps-ignored", changed("if not matches(patterns, p)]", "if matches(patterns, p)]")),
    (
        "hash-drops-blob-id",
        changed(
            'h.update(p + b"\\0" + m.encode() + b"\\0" + s.encode() + b"\\n")\n    return h.hexdigest(), len(keep)',
            'h.update(p + b"\\0" + m.encode() + b"\\n")\n    return h.hexdigest(), len(keep)',
        ),
    ),
    ("docker-doublestar-needs-directory", changed('out += "(.*/)?"', 'out += "(.*/)"')),
    ("fail-closed-exit-code-lost", changed("return 3", "return 0")),
    ("own-ignore-flag-ignored", changed("if not a.own_ignore:", "if True:")),
    ("git-comparison-skipped", changed("if got != gi:", "if False:")),
    # ---- the strip step, the floor and the tree key (2026-11-09) ----
    (
        "floor-check-skipped",
        changed(
            "    bad = floor_violations([e[2] for e in entries], patterns)\n    if bad:",
            "    bad = []\n    if bad:",
        ),
    ),
    (
        "floor-covers-no-directory",
        changed(
            'return any(path == d or path.startswith(d + "/") for d in KEEP_DIRS)', "return False"
        ),
    ),
    (
        "floor-forgets-docker-names",
        changed("if path in KEEP_FILES or KEEP_NAME.match(path):", "if path in KEEP_FILES:"),
    ),
    (
        "floor-protects-prose",
        changed(
            'if path.endswith(".md") or path.rsplit("/", 1)[-1] == ".gitignore":\n        return False',
            "if False:\n        return False",
        ),
    ),
    (
        "nothing-left-allowed",
        changed(
            'if not kept:\n        raise FailClosed("nothing would be left after stripping")',
            "if False:\n        raise FailClosed('x')",
        ),
    ),
    (
        "strip-reads-the-worktree-ignore-file",
        changed(
            '    p = git("show", "%s:.workerignore" % rev, cwd=root, check=False)\n',
            '    return open(os.path.join(root, ".workerignore")).read()\n    p = git("show", "%s:.workerignore" % rev, cwd=root, check=False)\n',
        ),
    ),
    (
        "missing-ignore-file-allowed",
        changed(
            '    if p.returncode != 0:\n        raise FailClosed("%s has no .workerignore in its own tree" % rev)',
            "    if False:\n        raise FailClosed('x')",
        ),
    ),
    (
        "delete-guard-dropped",
        changed(
            'if ".git" in parts or path.startswith("/") or ".." in parts or "" in parts:',
            "if False:",
        ),
    ),
    ("containment-check-dropped", changed("if not _inside(root, full):", "if False:")),
    (
        "containment-only-checks-the-root",
        changed(
            "    return os.path.realpath(os.path.dirname(full)) == want\n",
            "    return os.path.commonpath([r, os.path.realpath(os.path.dirname(full))]) == r\n",
        ),
    ),
    ("symlinks-followed", changed("st = os.lstat(full)", "st = os.stat(full)")),
    (
        "emptied-directories-kept",
        changed("            os.rmdir(os.path.join(root, d))", "            pass"),
    ),
    (
        "fail-closed-only-for-syntax",
        changed(
            "        return run(a, root, pats_for, load)\n    except FailClosed as e:",
            "        return run(a, root, pats_for, load)\n    except SyntaxProblem as e:",
        ),
    ),
    ("export-into-non-empty-dir-allowed", changed("    if os.listdir(dest):", "    if False:")),
    (
        "export-keeps-ignored-files",
        changed(
            "            if matches(pats, name):\n                continue",
            "            if False:\n                continue",
        ),
    ),
    (
        "strip-removes-the-kept-set",
        changed(
            "    removed = [e for e in entries if matches(patterns, e[2])]",
            "    removed = [e for e in entries if not matches(patterns, e[2])]",
        ),
    ),
    ("key-version-changed", changed('b"kf-tree-v2\\nignore "', 'b"kf-tree-v1\\nignore "')),
    (
        "key-ignore-digest-dropped",
        changed(
            'h.update(b"kf-tree-v2\\nignore " + digest.encode() + b"\\n")',
            'h.update(b"kf-tree-v2\\nignore " + b"\\n")',
        ),
    ),
    (
        "key-includes-ignored-files",
        changed(
            "for m, s, p in entries if not matches(tuple(patterns), p)",
            "for m, s, p in entries if matches(tuple(patterns), p)",
        ),
    ),
    (
        "key-unsorted",
        changed("    keep = sorted((os.fsencode(p)", "    keep = list((os.fsencode(p)"),
    ),
    (
        "key-drops-mode",
        changed(
            'h.update(p + b"\\0" + m.encode() + b"\\0" + s.encode() + b"\\n")\n    return h.hexdigest()\n\n\ndef _inside',
            'h.update(p + b"\\0" + s.encode() + b"\\n")\n    return h.hexdigest()\n\n\ndef _inside',
        ),
    ),
    (
        "keycheck-ignores-a-different-key",
        changed("if p.returncode != 0 or mine != theirs:", "if p.returncode != 0:"),
    ),
    (
        "keycheck-without-worker-tree-allowed",
        changed("if not os.path.exists(helper):", "if False:"),
    ),
    (
        "lint-ignores-what-the-strip-refuses",
        changed(
            '        strip_plan(ents, pats[".workerignore"])\n    except FailClosed as e:',
            "        pass\n    except FailClosed as e:",
        ),
    ),
    (
        "export-extraction-errors-uncaught",
        changed("except (tarfile.TarError, OSError) as e:", "except ZeroDivisionError as e:"),
    ),
    ("strip-needs-exactly-one-target-lost", changed("if a.in_place == bool(a.dest):", "if False:")),
]
COLLECT = """import io,json,unittest
s=unittest.defaultTestLoader.discover('.',pattern='test_ignore_ladder.py')
r=unittest.TextTestRunner(stream=io.StringIO()).run(s)
print(json.dumps({'tests':r.testsRun,'skipped':len(r.skipped),'killers':[t.id() for t,_ in r.failures+r.errors]}))
"""


def run(source):
    with tempfile.TemporaryDirectory() as d:
        shutil.copy(os.path.join(HERE, "test_ignore_ladder.py"), d)
        with open(os.path.join(d, "ignore_ladder.py"), "w") as f:
            f.write(source)
        p = subprocess.run(
            [sys.executable, "-c", COLLECT],
            cwd=d,
            capture_output=True,
            text=True,
            timeout=600,
            env={**os.environ, "KF_LADDER_REPO": os.path.dirname(os.path.dirname(HERE))},
        )
        assert p.returncode == 0, p.stderr[-300:]
        return json.loads(p.stdout.splitlines()[-1])


def main():
    jobs = int(sys.argv[sys.argv.index("--jobs") + 1]) if "--jobs" in sys.argv else 1
    only = [a for a in sys.argv[1:] if not a.startswith("--") and not a.isdigit()]
    cases = [c for c in CASES if not only or any(o in c[0] for o in only)]
    base = run(SOURCE)
    assert base["killers"] == [], base
    print("baseline: %d tests, %d skipped, 0 failing" % (base["tests"], base["skipped"]))
    weak = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=jobs) as pool:
        results = list(pool.map(lambda case: run(case[1])["killers"], cases))
    for (name, _src), killers in zip(cases, results):
        print(
            "%-36s killers=%d %s" % (name, len(killers), "" if len(killers) >= 2 else "  <-- WEAK")
        )
        if len(killers) < 2:
            weak.append(name)
    print("MUTANTS total=%d weak=%d" % (len(cases), len(weak)))
    return 1 if weak else 0


if __name__ == "__main__":
    sys.exit(main())
