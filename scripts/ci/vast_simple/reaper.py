#!/usr/bin/env python3
"""Backstop for the simple Vast runner: destroy kf-ci-simple-* instances past their deadline.

Usage: VAST_API_KEY=... python3 -I reaper.py [--dry-run] [--max-age-minutes N]
Run from a cron (a scheduled private workflow). Exit 0 clean, 2 the instance list is malformed
(fail closed: nothing is deleted), 3 something was left (undeletable, or a kf-ci-simple- label
this tool cannot age). Only labels that start with the runner's prefix are ever touched, and the
age comes from the epoch the runner put in the label, never from the provider's clock fields.
"""

import argparse
import os
import re
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import offers as offer_rules  # noqa: E402
import vast_api  # noqa: E402
from vast_api import PREFIX, ApiError  # noqa: E402

CLEAN, MALFORMED, LEFT = 0, 2, 3
LABEL = re.compile(re.escape(PREFIX) + r"(\d{10})-[0-9a-f]{8}")
MARGIN_MINUTES = 30
VERIFY_TRIES, VERIFY_SECONDS = 12, 10


@dataclass
class Deps:
    clock: object = time.time
    sleep: object = time.sleep
    stdout: object = field(default_factory=lambda: sys.stdout)


def main(argv=None, deps=None, env=None):
    env = os.environ if env is None else env
    deps = deps or Deps()
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("--defaults", default=str(offer_rules.DEFAULTS_PATH))
    p.add_argument("--api-base", default=vast_api.CONSOLE)
    p.add_argument("--max-age-minutes", type=int)
    p.add_argument("--dry-run", action="store_true")
    try:
        args = p.parse_args(argv)
    except SystemExit as exit_:
        return int(exit_.code or 2)
    red = vast_api.Redactor(env.get("VAST_API_KEY"))

    def say(text):
        print("[reaper] " + red.clean(text), file=deps.stdout, flush=True)

    try:
        hard = offer_rules.load_defaults(args.defaults)["hardMinutes"]
        api = vast_api.VastApi(env.get("VAST_API_KEY"), args.api_base, redactor=red)
        rows = api.list_instances()
    except (ApiError, OSError, ValueError, KeyError) as error:
        say("fail closed, nothing deleted: %s" % error)
        return MALFORMED
    limit = (args.max_age_minutes or hard + MARGIN_MINUTES) * 60
    now, status = deps.clock(), CLEAN
    for row in rows:
        label = row.get("label")
        if not isinstance(label, str) or not label.startswith(PREFIX):
            continue
        match = LABEL.fullmatch(label)
        if not match:
            say("instance %d: label cannot be aged, left alone" % row["id"])
            status = LEFT
            continue
        age = now - int(match.group(1))
        if age <= limit:
            continue
        say("instance %d is %d minutes old: destroy" % (row["id"], age // 60))
        if args.dry_run:
            continue
        try:
            api.destroy(row["id"], label)
            for _ in range(VERIFY_TRIES):
                if all(r["id"] != row["id"] for r in api.list_instances(label)):
                    break
                deps.sleep(VERIFY_SECONDS)
            else:
                raise ApiError("still listed after delete")
        except ApiError as error:
            say("instance %d not removed: %s" % (row["id"], error))
            status = LEFT
    return status


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
