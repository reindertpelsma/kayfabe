"""Execute a validated, ordered assignment; flush progress before the next step."""

import os
import sys
import time
from .assignment import validate
from .step import Context, StepResult, RetryClass, Verdict


class Runner:
    def __init__(self, catalog, *, output=None, env=None, clock=time.monotonic):
        self.catalog = catalog
        self.output = output if output is not None else sys.stdout
        self.env = dict(os.environ if env is None else env)
        self.clock = clock

    def emit(self, line):
        self.output.write(line + "\n")
        self.output.flush()

    def run(self, assignment):
        plan = validate(assignment)
        wire = "runs" in plan
        if wire:
            entries = plan["runs"]
            steps = self.catalog.assigned(entries)
            seconds = plan["reservation_minutes"] * 60
        else:
            entries = plan["steps"]
            steps = self.catalog.ordered(
                [s["id"] for s in entries], plan.get("inputs", ())
            )
            seconds = plan["budget_seconds"] - plan.get("reserve_seconds", 0)
        end = self.clock() + seconds
        results = []
        final = StepResult(Verdict.PASS)
        self.emit("KFCI_START id=" + plan["id"])
        for step, entry in zip(steps, entries):
            start = self.clock()
            name = entry["id"]
            estimate = max(step.estimated_minutes, entry.get("minutes", 0))
            guarded = end - start < estimate * 60 or end <= start
            if guarded:
                final = StepResult.retry(RetryClass.TIME)
            else:
                deadline = min(end, start + step.timeout_seconds)
                ctx = Context(
                    dict(entry.get("parameters", {})),
                    dict(self.env),
                    self.output,
                    deadline,
                    self.clock,
                )
                ctx.assignment_id = plan["id"]
                ctx.instance_id = name
                ctx.drivers = dict(entry["drivers"]) if wire else None
                self.emit("KFCI_STEP_START id=" + name)
                try:
                    final = step.run(ctx)
                    if not isinstance(final, StepResult):
                        raise TypeError("step did not return StepResult")
                    if self.clock() > deadline and final.verdict is Verdict.PASS:
                        final = StepResult.retry(RetryClass.TIME)
                except TimeoutError:
                    final = StepResult.retry(RetryClass.TIME)
                except Exception:
                    final = StepResult.retry(RetryClass.OPERATOR)
            row = dict(
                id=name,
                verdict=final.verdict.value,
                rc=final.rc,
                retry_class=final.retry_class.value if final.retry_class else None,
                started=not guarded,
            )
            results.append(row)
            cls = " class=" + final.retry_class.value if final.retry_class else ""
            self.emit("KFCI_ROW id=%s result=%s%s" % (name, final.verdict.value, cls))
            if not guarded:
                self.emit("KFCI_STEP_EXIT id=%s rc=%d" % (name, final.rc))
            if final.verdict is not Verdict.PASS:
                break
        self.emit(
            "KFCI_SUMMARY result=%s steps=%d of=%d"
            % (final.verdict.value, len(results), len(steps))
        )
        self.emit("KFCI_EXIT rc=%d" % final.rc)
        return dict(
            id=plan["id"], verdict=final.verdict.value, rc=final.rc, steps=results
        )
