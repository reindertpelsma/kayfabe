"""Three transports into one Runner, plus the compatible stage-wrapper entry."""

import argparse
import io
import json
import os
import signal
import sys
import time
from .assignment import AssignmentError, MAX_BYTES, load
from .apps_gate import AppsGate
from .catalog import Catalog, CatalogError
from .runner import Runner
from .step import Cancelled, Context


def catalog():
    result = Catalog()
    result.register(AppsGate())
    result.register(AppsGate("full"))
    result.register(AppsGate("middle"))
    return result


def execute(plan, *, output=None, env=None, local=False):
    steps = catalog()
    if local and "runs" in plan:
        raise AssignmentError("local resolution applies only to the local steps format")
    if local:
        # Only direct local runs may ask for prerequisites to be added. The
        # Worker's already ordered assignment is validated, never rearranged.
        resolved = steps.resolve(
            [s["id"] for s in plan["steps"]], plan.get("inputs", ())
        )
        parameters = {s["id"]: s.get("parameters", {}) for s in plan["steps"]}
        plan = dict(
            plan, steps=[dict(id=n, parameters=parameters.get(n, {})) for n in resolved]
        )
    return Runner(steps, output=output, env=env).run(plan)


def _main(argv=None):
    argv = list(sys.argv[1:] if argv is None else argv)
    if argv and argv[0] == "apps-gate":
        args = argv[1:]
        params = (
            dict(tag=args[0], prov=args[1], which=args[2] if len(args) == 3 else "full")
            if len(args) in (2, 3)
            else {}
        )
        step = AppsGate()
        ctx = Context(
            params,
            dict(os.environ),
            sys.stdout,
            time.monotonic() + step.timeout_seconds,
        )
        return step.run(ctx).rc
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_subparsers(dest="mode", required=True)
    direct = modes.add_parser("direct")
    direct.add_argument("assignment")
    direct.add_argument("--resolve", action="store_true")
    modes.add_parser("worker", help="one assignment JSON on stdin; no network calls")
    modes.add_parser(
        "daemon",
        help="serial JSONL assignments on stdin until EOF; supervisor owns transport",
    )
    args = parser.parse_args(argv)
    try:
        if args.mode == "daemon":
            # A non-Vast supervisor can feed this persistent executor over a local
            # pipe. It acquires assignments; this process neither polls nor plans.
            while True:
                raw = sys.stdin.readline(MAX_BYTES + 1)
                if not raw:
                    return 0
                if len(raw.encode("utf-8")) > MAX_BYTES:
                    raise AssignmentError("assignment exceeds 1 MiB")
                summary = execute(load(io.StringIO(raw)))
                print("KFCI_RESULT " + json.dumps(summary, sort_keys=True), flush=True)
        plan = load(args.assignment if args.mode == "direct" else sys.stdin)
        return execute(plan, local=getattr(args, "resolve", False))["rc"]
    except (AssignmentError, CatalogError) as error:
        print("KFCI_ERROR " + str(error).replace("\n", " ")[:200], flush=True)
        print("KFCI_RETRY class=USAGE\nKFCI_EXIT rc=2", flush=True)
        return 2
    except (OSError, UnicodeError):
        print("KFCI_RETRY class=USAGE\nKFCI_EXIT rc=2", flush=True)
        return 2


def main(argv=None):
    """Unwind owned process groups on supervisor cancellation, then write an exit."""

    def interrupted(*_):
        raise Cancelled("executor cancelled")

    previous = {sig: signal.getsignal(sig) for sig in (signal.SIGTERM, signal.SIGINT)}
    try:
        for sig in previous:
            signal.signal(sig, interrupted)
        return _main(argv)
    except Cancelled:
        print("KFCI_RETRY class=OPERATOR\nKFCI_EXIT rc=3", flush=True)
        return 3
    finally:
        for sig, handler in previous.items():
            signal.signal(sig, handler)
