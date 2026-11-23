# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Run with PYTHONPATH=scripts/ci python3 -m adobe_census."""
import argparse
import json
from pathlib import Path
import sys

from . import APPS, compare, parse, render_table, validate


def read_text(path):
    data = path.read_bytes()
    return data.decode("utf-16" if data.startswith((b"\xff\xfe", b"\xfe\xff")) else "utf-8-sig")


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON key")
        result[key] = value
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    actions = parser.add_subparsers(dest="action", required=True)
    collect = actions.add_parser("parse", help="parse an evidence-input manifest into a census")
    collect.add_argument("manifest", type=Path)
    check = actions.add_parser("validate", help="validate census JSON")
    check.add_argument("census", type=Path)
    diff = actions.add_parser("diff", help="compare reference and kf3 censuses")
    diff.add_argument("reference", type=Path)
    diff.add_argument("kf3", type=Path)
    diff.add_argument("--format", choices=("table", "json"), default="table")
    args = parser.parse_args()

    def load(path):
        return json.loads(read_text(path), object_pairs_hook=unique_object)

    try:
        if args.action == "parse":
            manifest = load(args.manifest)
            if not isinstance(manifest, dict) or manifest.keys() - APPS.keys():
                raise ValueError("expected an object keyed by supported app names")
            records = []
            for app in APPS:
                entries = manifest.get(app, [])
                if not isinstance(entries, list):
                    raise ValueError("expected a list of input descriptors")
                inputs = []
                for entry in entries:
                    if (not isinstance(entry, dict) or entry.keys() != {"format", "path"}
                            or not all(isinstance(value, str) for value in entry.values())):
                        raise ValueError("invalid input descriptor")
                    path = args.manifest.parent / entry["path"]
                    inputs.append((entry["format"], path.name, read_text(path)))
                records.append(parse(app, inputs))
            print(json.dumps(records, indent=2))
        elif args.action == "validate":
            validate(load(args.census))
        else:
            result = compare(load(args.reference), load(args.kf3))
            if args.format == "json":
                print(json.dumps(result, indent=2))
            else:
                print(render_table(result), end="")
    except (OSError, ValueError) as error:
        # OS errors can contain the operator's local paths.
        parser.exit(2, f"adobe_census: {error if not isinstance(error, OSError) else error.strerror}\n")


if __name__ == "__main__":
    sys.exit(main())
