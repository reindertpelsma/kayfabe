"""B1g read-only market watcher (controller 01:30). It never rents, arms timers or edits leases.

It reuses the operator's own offer rules by loading them from ops.py (AST slices of the same
source: imports and profile, the per-SKU search, and the filter/selection block), so no rule is
duplicated. One poll = one harness market check (the operator's 7 per-SKU searches, run
sequentially, plus one account-credit read). Cadence: first poll at start, then every
1200 s + uniform(0, 120) s, never faster. Exit 0 on the first qualifying offer, 3 after 8 h
without one, 4 after 3 consecutive API errors. Writes only into its output directory.

Usage: market_watch.py [output_dir]   (default: <workspace>/market_watch)
"""

import ast
import contextlib
import datetime as dt
import io
import json
from pathlib import Path
import random
import subprocess
import sys
import time

sys.dont_write_bytecode = True

HERE = Path(__file__).resolve().parent
INTERVAL = 1200
JITTER = 120
MAX_SECONDS = 8 * 3600
MAX_ERRORS = 3
ALLOWED = (("search", "offers"), ("show", "user"))
STAGES = {
    'x.get("verification")': "base_filters",
    "P.exclude_bad(": "known_bad",
    "geolocation": "denied_or_missing_country",
    "for candidate in offers:": "plan_stop_cap_reserve_or_90min",
}


class ReadOnlyViolation(RuntimeError):
    """A non-whitelisted (possibly mutating) provider call was attempted."""


class ApiError(RuntimeError):
    """A read-only provider query failed or returned non-JSON."""


def readonly_api(args, runner=subprocess.run):
    if tuple(args[:2]) not in ALLOWED:
        raise ReadOnlyViolation("market watcher is read-only: " + " ".join(map(str, args[:2])))
    r = runner(["vastai", *args, "--raw"], text=True, capture_output=True, timeout=60)
    if r.returncode != 0:
        raise ApiError("query rc=%d" % r.returncode)
    try:
        return json.loads(r.stdout)
    except json.JSONDecodeError as error:
        raise ApiError("query returned non-JSON") from error


class Rules:
    def __init__(self, namespace, skus, search, stages):
        self.namespace = namespace
        self.skus = skus
        self.search = search
        self.stages = stages


def load_rules(ops_path):
    source = Path(ops_path).read_text()
    tree = ast.parse(source)
    prelude = []
    for node in tree.body:
        if isinstance(node, ast.Assign) and ast.unparse(node.targets[0]) == "mode":
            break
        if isinstance(node, (ast.Import, ast.ImportFrom)):
            prelude.append(node)
        elif isinstance(node, ast.Assign) and ast.unparse(node.targets[0]) in ("D", "W", "floor"):
            prelude.append(node)
        elif isinstance(node, ast.Expr) and ast.unparse(node).startswith("sys.path.insert("):
            prelude.append(node)
        elif isinstance(node, ast.FunctionDef) and node.name == "now":
            prelude.append(node)
    branch = next(
        n
        for n in tree.body
        if isinstance(n, ast.If) and ast.unparse(n.test) == "mode in ('preflight', 'rent')"
    )
    skus = [
        n for n in branch.body if isinstance(n, ast.Assign) and ast.unparse(n.targets[0]) == "skus"
    ]
    search = [n for n in ast.walk(branch) if isinstance(n, ast.FunctionDef) and n.name == "search"]
    start = next(
        i
        for i, n in enumerate(branch.body)
        if ast.unparse(n).startswith("offers = [")
        and 'x.get("verification")' in ast.get_source_segment(source, n)
    )
    end = next(
        i for i, n in enumerate(branch.body) if ast.unparse(n).startswith("rate = float(offer[")
    )
    if len(skus) != 1 or len(search) != 1 or not start < end:
        raise ValueError("operator rule anchors changed; refusing to guess")
    namespace = {"__name__": "ops_rules", "__builtins__": __builtins__}
    exec(compile(ast.Module(prelude + skus, []), str(ops_path), "exec"), namespace)
    exec(compile(ast.Module(search, []), str(ops_path), "exec"), namespace)
    namespace["api"] = readonly_api
    stages = []
    for node in branch.body[start:end]:
        text = ast.get_source_segment(source, node)
        label = next((v for k, v in STAGES.items() if k in text), None)
        stages.append((label, compile(ast.Module([node], []), str(ops_path), "exec")))
    return Rules(namespace, dict(namespace["skus"]), namespace["search"], stages)


def decide(rules, offers, credit):
    ns = dict(rules.namespace, offers=list(offers), credit=float(credit))
    rejected = {label: 0 for label in STAGES.values()}
    selected = None
    eligible_count = 0
    with contextlib.redirect_stdout(io.StringIO()):
        try:
            for label, code in rules.stages:
                before = len(ns["offers"])
                exec(code, ns)
                if label == "plan_stop_cap_reserve_or_90min":
                    eligible_count = len(ns["eligible"])
                    rejected[label] += before - eligible_count
                elif label:
                    rejected[label] += before - len(ns["offers"])
            selected = ns.get("offer")
        except SystemExit as stop:
            if stop.code != 75:
                raise
    return {
        "selected": selected,
        "rejected": rejected,
        "eligible_count": eligible_count,
        "offers": len(offers),
    }


def poll_once(rules, api):
    rules.namespace["api"] = api
    try:
        credit = float(api(["show", "user"])["credit"])
        offers = []
        for item in rules.skus.items():
            offers.extend(rules.search(item))
    finally:
        rules.namespace["api"] = readonly_api
    return decide(rules, offers, credit)


def watch(
    out,
    poll,
    *,
    clock=time.monotonic,
    sleep=time.sleep,
    rng=random.Random(),
    max_seconds=MAX_SECONDS
):
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    log = (out / "market_watch.log").open("a")

    def say(text):
        stamp = dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds")
        log.write(stamp + " " + text + "\n")
        log.flush()

    start = clock()
    errors = polls = 0
    say("START interval=%d jitter=0..%d max_seconds=%d" % (INTERVAL, JITTER, max_seconds))
    while True:
        polls += 1
        try:
            result = poll()
            errors = 0
            say(
                "POLL %d offers=%d eligible=%d rejected=%s"
                % (
                    polls,
                    result["offers"],
                    result["eligible_count"],
                    json.dumps(result["rejected"], sort_keys=True),
                )
            )
        except (
            ApiError,
            ReadOnlyViolation,
            OSError,
            subprocess.SubprocessError,
            KeyError,
            ValueError,
            RuntimeError,
        ) as error:
            errors += 1
            result = None
            say("POLL %d ERROR %s n=%d" % (polls, type(error).__name__, errors))
            if errors >= MAX_ERRORS:
                say("EXIT 4 repeated API errors")
                return 4
        if result and result["selected"]:
            o = result["selected"]
            row = {
                k: o.get(k)
                for k in (
                    "id",
                    "machine_id",
                    "dph_total",
                    "candidate_die",
                    "gpu_name",
                    "geolocation",
                    "verification",
                    "cpu_cores_effective",
                )
            }
            row["utc"] = dt.datetime.now(dt.timezone.utc).isoformat()
            (out / "match.json").write_text(json.dumps(row, indent=2) + "\n")
            say(
                "MATCH id=%s machine=%s price=%s die=%s"
                % (o.get("id"), o.get("machine_id"), o.get("dph_total"), o.get("candidate_die"))
            )
            print("MATCH", json.dumps(row), flush=True)
            say("EXIT 0")
            return 0
        delay = INTERVAL + rng.uniform(0, JITTER)
        if clock() - start + delay >= max_seconds:
            say("EXIT 3 no qualifying offer within %d s" % max_seconds)
            return 3
        sleep(delay)


def main():
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else HERE / "market_watch"
    rules = load_rules(HERE / "ops.py")
    return watch(out, lambda: poll_once(rules, readonly_api))


if __name__ == "__main__":
    sys.exit(main())
