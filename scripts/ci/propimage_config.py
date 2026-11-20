#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The propimage bundle configuration: schema, validator and the app resolver (stdlib only).

STATUS: LIVE as schema, validator and resolver, 2026-11-19 (GPU-free; tests: test_propimage_config.py, mutants: propimage_config_mutants.py). Nothing
reads the entries at run time yet: the generic stage that iterates them and the Worker planner are the next steps of
docs/design/V3_PROPIMAGE_CONFIG.md. Rule: docs/OWNER_RULINGS.md section BA item 1 and the 2026-11-19 refinement (image division is arbitrary).

A BUNDLE is one stored image (one object, one Windows or Linux disk) and the applications installed on it. The code has no concept of "image A":
a bundle is a configuration entry, and an application list is turned into the bundles to fetch by `bundles_for`.

  {"schema": "kayfabe.propimage-bundles/1",
   "status": "free text",                                     optional
   "bundles": {ID: BUNDLE, ...}}
  BUNDLE = {"os": "windows" | "linux",
            "r2": "object/key/in/the/store.zip",              the object key of the image archive (an operator's store; S3-portable)
            "size_bytes": N,                                  what a box downloads (the bandwidth estimate reads it)
            "apps": ["app", ...],                             the applications on the image: the names an app list is resolved against
            "sha256": "64 hex",                               optional identity pin (enforced only by a strict run, section AW)
            "ref": "pins-name.zip",                           optional name of the object in a pins file
            "licence_note": "text",                           optional; never a key, account, e-mail or institution
            "basis": "text",                                  optional; how the entry was measured or why it is unverified
            "needs_gpu_families": ["ada", ...],               optional; the image has only been shown to work on these families
            "run": RUN}                                       optional: what a stage reads (absent: a fetch-only bundle)
  RUN (windows) = {"rows_file": "scripts/.../table.py",       the module whose CHECKS table holds the rows (timeout, lane, gpu, disabled)
                   "pins_file": "scripts/ci/..._pins.json",   the pins file (image, boot GOP, swtpm sources and, if `data_iso`, the data volume)
                   "rows": ["row", ...],                      optional: the rows of the stage in order (absent: the table's order)
                   "solo_rows": ["row", ...],                 optional: rows that always boot alone (a discovery-lane row always does)
                   "data_iso": true | false,                  the stage passes the open-asset data volume (`--data-iso`)
                   "guest_ram_mb": N, "audio": true|false,    per-run overrides of the sealed state (0 / false: none)
                   "harness_args": {"before_apps": [..], "after_apps": [..], "per_row": {"row": [..]}}}
                                                              argv words of the harness command: `--app-set X` belongs in before_apps; the order is
                                                              before_apps, `--data-iso PATH` (if data_iso), `--apps LIST`, after_apps, `--ram-mb N`,
                                                              `--audio`, then the per_row words of the rows on the boot, in row order
  RUN (linux)   = {"rows_file": "scripts/bench/linux/..._rows.py", "pins_file": "scripts/ci/..._pins.json", "isolation": "row" | "shared"}

Every refusal has a name (`ConfigError.reason`, upper case with an optional `:detail`). Unknown keys are refused: a typo is not a setting.
"""
import argparse
import json
import re
import sys
from itertools import combinations
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
DEFAULT_CONFIG = ROOT / "ci/vast-broker/propimage-bundles.json"
SCHEMA = "kayfabe.propimage-bundles/1"
OSES = ("windows", "linux")
FAMILIES = ("turing", "ampere", "ada", "blackwell")          # propimage_apps.FAMILY_BY_CC values (a test compares them)
ISOLATIONS = ("row", "shared")
BUNDLE_ID_RE = re.compile(r"[a-z0-9][a-z0-9_-]{0,31}")
APP_RE = re.compile(r"[a-z0-9][a-z0-9_]{0,31}")               # propimage_runspec.APP_RE
ROW_RE = APP_RE
KEY_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9._/-]{0,199}")      # ci/vast-broker/src/propimages.js KEY
REF_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,99}")
PATH_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9._/-]{0,199}")
WORD_RE = re.compile(r"[A-Za-z0-9._:=/-]{1,64}")
HEX64 = re.compile(r"[0-9a-f]{64}")
TEXT_RE = re.compile(r"[\x20-\x7e]{1,300}")
SECRETISH = re.compile(r"@|[A-Za-z0-9]{24,}")             # an e-mail address, or a key-sized token
MAX_BUNDLES = 16
MAX_APPS = 64
MAX_SIZE = 1 << 42
BUNDLE_KEYS = {"os", "r2", "size_bytes", "apps", "sha256", "ref", "licence_note", "basis", "needs_gpu_families", "run"}
BUNDLE_REQUIRED = ("os", "r2", "size_bytes", "apps")
WINDOWS_RUN_KEYS = {"rows_file", "pins_file", "rows", "solo_rows", "data_iso", "guest_ram_mb", "audio", "harness_args"}
WINDOWS_RUN_REQUIRED = ("rows_file", "pins_file")
LINUX_RUN_KEYS = {"rows_file", "pins_file", "isolation"}
LINUX_RUN_REQUIRED = ("rows_file", "pins_file", "isolation")
ARG_KEYS = {"before_apps", "after_apps", "per_row"}


class ConfigError(ValueError):
    def __init__(self, reason):
        super().__init__(reason)
        self.reason = reason


def refuse(condition, reason):
    if not condition:
        raise ConfigError(reason)


def _is_int(value):
    return type(value) is int


def _unique_strings(value, regex, reason, limit=MAX_APPS):
    refuse(type(value) is list and 0 < len(value) <= limit, reason)
    refuse(all(type(x) is str and regex.fullmatch(x) for x in value), reason)
    refuse(len(set(value)) == len(value), reason)


def _text(value, name, bid):
    refuse(type(value) is str and TEXT_RE.fullmatch(value), "%s:%s" % (name, bid))
    refuse(not SECRETISH.search(value), "%s_SECRETISH:%s" % (name, bid))


def _relative_path(value, reason):
    refuse(type(value) is str and PATH_RE.fullmatch(value), reason)
    refuse(".." not in value.split("/") and "//" not in value and not value.endswith("/"), reason)


def _words(value, bundle, where):
    refuse(type(value) is list and len(value) <= 16 and all(type(w) is str and WORD_RE.fullmatch(w) for w in value), "ARGS:%s:%s" % (bundle, where))


def check_windows_run(bid, run):
    refuse(set(run) <= WINDOWS_RUN_KEYS, "RUN_KEY:" + bid)
    refuse(all(k in run for k in WINDOWS_RUN_REQUIRED), "RUN_MISSING:" + bid)
    _relative_path(run["rows_file"], "RUN_ROWS_FILE:" + bid)
    _relative_path(run["pins_file"], "RUN_PINS_FILE:" + bid)
    rows = run.get("rows")
    if rows is not None:
        _unique_strings(rows, ROW_RE, "RUN_ROWS:" + bid, 128)
    if "solo_rows" in run:
        _unique_strings(run["solo_rows"], ROW_RE, "RUN_SOLO_ROWS:" + bid, 128)
        if rows is not None:
            refuse(set(run["solo_rows"]) <= set(rows), "RUN_SOLO_ROWS_UNKNOWN:" + bid)
    if "data_iso" in run:
        refuse(type(run["data_iso"]) is bool, "RUN_DATA_ISO:" + bid)
    if "guest_ram_mb" in run:
        value = run["guest_ram_mb"]
        refuse(_is_int(value) and (value == 0 or (1024 <= value <= 1 << 20 and value % 1024 == 0)), "RUN_GUEST_RAM:" + bid)
    if "audio" in run:
        refuse(type(run["audio"]) is bool, "RUN_AUDIO:" + bid)
    if "harness_args" in run:
        args = run["harness_args"]
        refuse(type(args) is dict and set(args) <= ARG_KEYS, "ARGS_KEY:" + bid)
        for where in ("before_apps", "after_apps"):
            if where in args:
                _words(args[where], bid, where)
        per_row = args.get("per_row", {})
        refuse(type(per_row) is dict, "ARGS_PER_ROW:" + bid)
        for row, words in per_row.items():
            refuse(type(row) is str and ROW_RE.fullmatch(row), "ARGS_PER_ROW_NAME:" + bid)
            if rows is not None:
                refuse(row in rows, "ARGS_PER_ROW_UNKNOWN:%s:%s" % (bid, row))
            _words(words, bid, "per_row:" + row)
        refuse("--data-iso" not in [w for where in ("before_apps", "after_apps") for w in args.get(where, [])]
               and not any("--data-iso" in words for words in per_row.values()), "ARGS_DATA_ISO_SET_BY_FIELD:" + bid)


def check_linux_run(bid, run):
    refuse(set(run) <= LINUX_RUN_KEYS, "RUN_KEY:" + bid)
    refuse(all(k in run for k in LINUX_RUN_REQUIRED), "RUN_MISSING:" + bid)
    _relative_path(run["rows_file"], "RUN_ROWS_FILE:" + bid)
    _relative_path(run["pins_file"], "RUN_PINS_FILE:" + bid)
    refuse(run["isolation"] in ISOLATIONS, "RUN_ISOLATION:" + bid)


def check_bundle(bid, b):
    refuse(type(bid) is str and BUNDLE_ID_RE.fullmatch(bid), "BUNDLE_ID:" + str(bid)[:32])
    refuse(type(b) is dict, "BUNDLE_SHAPE:" + bid)
    refuse(set(b) <= BUNDLE_KEYS, "BUNDLE_KEY:" + bid)
    refuse(all(k in b for k in BUNDLE_REQUIRED), "BUNDLE_MISSING:" + bid)
    refuse(b["os"] in OSES, "BUNDLE_OS:" + bid)
    refuse(type(b["r2"]) is str and KEY_RE.fullmatch(b["r2"]), "BUNDLE_R2:" + bid)
    refuse(".." not in b["r2"].split("/") and "//" not in b["r2"] and not b["r2"].endswith("/"), "BUNDLE_R2:" + bid)
    refuse(_is_int(b["size_bytes"]) and 0 < b["size_bytes"] < MAX_SIZE, "BUNDLE_SIZE:" + bid)
    _unique_strings(b["apps"], APP_RE, "BUNDLE_APPS:" + bid)
    if "sha256" in b:
        refuse(type(b["sha256"]) is str and HEX64.fullmatch(b["sha256"]), "BUNDLE_SHA256:" + bid)
    if "ref" in b:
        refuse(type(b["ref"]) is str and REF_RE.fullmatch(b["ref"]), "BUNDLE_REF:" + bid)
    for key, name in (("licence_note", "BUNDLE_LICENCE_NOTE"), ("basis", "BUNDLE_BASIS")):
        if key in b:
            _text(b[key], name, bid)
    if "needs_gpu_families" in b:
        fam = b["needs_gpu_families"]
        refuse(type(fam) is list and fam and all(f in FAMILIES for f in fam) and len(set(fam)) == len(fam), "BUNDLE_FAMILIES:" + bid)
    if "run" in b:
        refuse(type(b["run"]) is dict, "RUN_SHAPE:" + bid)
        if b["os"] == "windows":
            check_windows_run(bid, b["run"])
        else:
            check_linux_run(bid, b["run"])


def validate(doc):
    """The document itself, or ConfigError(reason). Strict: unknown keys, duplicate object keys across bundles and an empty bundle list are refused."""
    refuse(type(doc) is dict and doc.get("schema") == SCHEMA, "SCHEMA")
    refuse(set(doc) <= {"schema", "status", "bundles"} and "bundles" in doc, "TOP_KEY")
    if "status" in doc:
        refuse(type(doc["status"]) is str and len(doc["status"]) <= 4000, "STATUS")
    bundles = doc["bundles"]
    refuse(type(bundles) is dict and 0 < len(bundles) <= MAX_BUNDLES, "BUNDLES")
    for bid, b in bundles.items():
        check_bundle(bid, b)
    keys = [b["r2"] for b in bundles.values()]
    refuse(len(set(keys)) == len(keys), "DUPLICATE_R2")
    return doc


def _no_duplicates(pairs):
    out = {}
    for k, v in pairs:
        if k in out:
            raise ConfigError("DUPLICATE_KEY:" + str(k)[:32])
        out[k] = v
    return out


def parse(text):
    try:
        return validate(json.loads(text, object_pairs_hook=_no_duplicates))
    except json.JSONDecodeError:
        raise ConfigError("JSON") from None


def load(path=DEFAULT_CONFIG):
    return parse(Path(path).read_text())


# ---------------------------------------------------------------- the resolver
def bundles_for(config, apps, os=None):
    """The bundle ids to fetch so that every requested application is on one of them: the fewest bytes, then the fewest bundles, then the
    smallest id list (so the answer is a function of the configuration and the set of applications only). The result is sorted by id.

    `os` (windows or linux) restricts the search to that platform. Without it, an application that exists on both a Windows and a Linux bundle is
    AMBIGUOUS_APP: the caller says which one. An application that is on no bundle is UNKNOWN_APP:<name>; nothing is guessed or skipped."""
    bundles = validate(config)["bundles"]
    refuse(os is None or os in OSES, "BUNDLE_OS:" + str(os)[:16])
    refuse(type(apps) in (list, tuple, set, frozenset) and all(type(a) is str for a in apps), "APPS_SHAPE")
    wanted = sorted(set(apps))
    for name in wanted:
        refuse(APP_RE.fullmatch(name), "BAD_APP:" + name[:32])
    holders = {}
    for name in wanted:
        have = sorted(bid for bid, b in bundles.items() if name in b["apps"] and (os is None or b["os"] == os))
        refuse(have, "UNKNOWN_APP:" + name)
        refuse(len({bundles[bid]["os"] for bid in have}) == 1, "AMBIGUOUS_APP:" + name)
        holders[name] = have
    if not wanted:
        return []
    relevant = sorted({bid for have in holders.values() for bid in have})
    best = None
    for count in range(1, len(relevant) + 1):
        for pick in combinations(relevant, count):
            if all(any(bid in pick for bid in holders[name]) for name in wanted):
                key = (sum(bundles[bid]["size_bytes"] for bid in pick), count, pick)
                if best is None or key < best:
                    best = key
    return list(best[2])


def download_bytes(config, ids):
    """Total bytes of the named bundles (the number the bandwidth estimate wants); an unknown id is UNKNOWN_BUNDLE."""
    bundles = validate(config)["bundles"]
    refuse(all(i in bundles for i in ids), "UNKNOWN_BUNDLE")
    return sum(bundles[i]["size_bytes"] for i in ids)


def main(argv=None):
    p = argparse.ArgumentParser(prog="propimage_config.py", description=__doc__.split("\n\n")[0])
    p.add_argument("--config", default=str(DEFAULT_CONFIG), help="a kayfabe.propimage-bundles/1 file (default: the repository's own entries)")
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("validate")
    r = sub.add_parser("resolve")
    r.add_argument("--apps", required=True, help="comma list of application names")
    r.add_argument("--os", choices=OSES)
    a = p.parse_args(argv)
    try:
        doc = load(a.config)
        if a.cmd == "validate":
            print(json.dumps({"ok": True, "bundles": sorted(doc["bundles"])}))
            return 0
        ids = bundles_for(doc, [x for x in a.apps.split(",") if x], a.os)
        print(json.dumps({"bundles": ids, "bytes": download_bytes(doc, ids)}))
        return 0
    except ConfigError as exc:
        print("KF_PROPIMAGE_CONFIG_REFUSED " + exc.reason, flush=True)
        return 2
    except OSError as exc:
        print("KF_PROPIMAGE_CONFIG_UNREADABLE " + type(exc).__name__, flush=True)
        return 2


if __name__ == "__main__":
    sys.exit(main())
