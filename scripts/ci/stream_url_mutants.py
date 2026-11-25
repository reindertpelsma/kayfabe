#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation checks of the URL-file reader of scripts/bench/specviewperf/stream_url.py (2026-11-16, standalone release run):
every mutant is one exact-once text replacement in a private copy; scripts/ci/test_spec_pc_knobs.py runs against the copy and the
failing tests are the killers. The unmutated copy must be green first and each mutant needs at least two killers (stdlib, GPU-free)."""
import json
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SRC_REL = "scripts/bench/specviewperf/stream_url.py"
SRC = (ROOT / SRC_REL).read_text()
T = "PcKnobs."
CASES = [
    ("schema-not-recognised", 'doc.get("schema") == URLS_SCHEMA', "False",
     "test_stream_reads_the_standalone_url_file_by_basename_of_the_key", "test_stream_reads_the_standalone_url_file_by_the_full_key_too"),
    ("any-schema-accepted", 'doc.get("schema") == URLS_SCHEMA', 'doc.get("schema") is not None',
     "test_a_document_with_another_schema_is_not_read_as_a_map", "test_a_document_with_another_schema_is_not_read_as_a_map"),
    ("basename-fallback-dropped", 'return urls[key] if key in urls else urls[key.rsplit("/", 1)[-1]]', "return urls[key]",
     "test_stream_reads_the_standalone_url_file_by_basename_of_the_key", "test_stream_still_checks_the_pin_with_the_standalone_file"),
    ("full-key-preferred-lost", 'return urls[key] if key in urls else urls[key.rsplit("/", 1)[-1]]', 'return urls[key.rsplit("/", 1)[-1]]',
     "test_stream_reads_the_standalone_url_file_by_the_full_key_too", "test_stream_reads_the_standalone_url_file_by_the_full_key_too"),
    ("missing-object-guessed", 'return urls[key] if key in urls else urls[key.rsplit("/", 1)[-1]]', 'return next(iter(urls.values()))',
     "test_stream_refuses_a_standalone_file_without_the_object", "test_stream_refuses_a_standalone_file_without_the_object"),
    ("flat-map-lost", "    return doc[key]\n", "    return doc['urls'][key]\n",
     "test_stream_match", "test_stream_mismatch"),
    ("noted-pin-is-enforced", "ok = a.sha256 is None or h.hexdigest() == a.sha256", "ok = h.hexdigest() == (a.sha256 or a.note_pin or h.hexdigest())",
     "test_a_noted_pin_that_differs_is_recorded_and_never_a_refusal", "test_a_noted_pin_that_differs_still_delivers_every_byte_and_the_hash_of_what_was_delivered"),
    ("no-pin-is-a-refusal", "ok = a.sha256 is None or h.hexdigest() == a.sha256", "ok = h.hexdigest() == a.sha256",
     "test_no_pin_at_all_records_the_computed_hash_and_exits_zero", "test_a_noted_pin_that_matches_is_recorded_as_a_match"),
    ("enforced-pin-ignored", "ok = a.sha256 is None or h.hexdigest() == a.sha256", "ok = True",
     "test_an_enforced_pin_still_refuses_beside_a_noted_one", "test_stream_mismatch"),
    ("noted-pin-never-differs", 'else ("match" if h.hexdigest() == a.note_pin else "differs")', 'else "match"',
     "test_a_noted_pin_that_differs_is_recorded_and_never_a_refusal", "test_an_enforced_pin_that_matches_passes_beside_a_wrong_noted_one"),
    ("computed-hash-not-recorded", 'time.time() - t0, h.hexdigest(), noted))', 'time.time() - t0, "-", noted))',
     "test_no_pin_at_all_records_the_computed_hash_and_exits_zero", "test_a_noted_pin_that_differs_is_recorded_and_never_a_refusal"),
]


def run(tree):
    p = subprocess.run([sys.executable, "-I", "-B", "scripts/ci/test_spec_pc_knobs.py"], cwd=tree, text=True,
                       capture_output=True, timeout=600)
    out = p.stdout + p.stderr
    return p.returncode, sorted(set(re.findall(r"^(?:FAIL|ERROR): (\S+)", out, re.M))), out


def main():
    weak = 0
    with tempfile.TemporaryDirectory(prefix="stream-url-mutants-") as tmp:
        tree = Path(tmp)
        for rel in ("scripts/ci/test_spec_pc_knobs.py", "scripts/bench/specviewperf/lanes.json", "scripts/bench/specviewperf/lanes_pc.json"):
            (tree / rel).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(ROOT / rel, tree / rel)
        for d in ("scripts", "scripts/ci"):
            (tree / d / "__init__.py").write_text("")
        target = tree / SRC_REL
        target.write_text(SRC)
        code, _, out = run(tree)
        if code:
            print("FAIL unmutated baseline\n" + out[-3000:])
            return 2
        print("PASS baseline", flush=True)
        for name, old, new, *tests in CASES:
            if SRC.count(old) != 1:
                print("BAD mutant %s: anchor found %d times" % (name, SRC.count(old)))
                return 2
            target.write_text(SRC.replace(old, new))
            try:
                code, killers, _ = run(tree)
            finally:
                target.write_text(SRC)
            names = [k.split(".")[-1] if "." in k else k for k in killers]
            ok = code != 0 and len(killers) >= 2 and bool(set(tests) & set(k.split("(")[0] for k in names))
            print("%s %s: %d killers %s" % ("PASS" if ok else "WEAK", name, len(killers), names[:4]), flush=True)
            weak += not ok
    print("%d/%d mutants killed by at least two tests" % (len(CASES) - weak, len(CASES)))
    return 1 if weak else 0


if __name__ == "__main__":
    sys.exit(main())
