"""The `CI` gate job of .github/workflows/ci.yml: what its script decides, and which jobs it must see.

STATUS: LIVE, 2026-11-17.

FALSIFIER (written before the code): the gate passes with a failed, cancelled or skipped job among the ones that carry
the gates (`stable`, `stripped` and the jobs split out of `stable`); a job that is in `needs` but in neither the strict
list nor the list of jobs that may skip by their own `if:` goes unnoticed (a new shard would then be allowed to skip);
a cancelled run's gate passes; or a job that may skip by its own `if:` fails the gate for doing so.

The script is run as GitHub runs it: `bash -e` with the `env:` of the step, the `${{ needs.X.result }}` expressions
replaced by the result the test gives that job.
"""

import functools
import re
import subprocess
import sys
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/ci.yml"

# Jobs that existed before the split and are accepted when skipped by their own `if:` (none has one today).
# A new job in `needs` that is in neither this set nor the gate's strict list fails the test below, on purpose.
LAX = {"aarch64", "firmware", "broker", "platform", "ptx"}


@functools.lru_cache(maxsize=1)
def workflow():
    return yaml.safe_load(WORKFLOW.read_text())


def gate():
    return workflow()["jobs"]["gate"]


def expand(template, results):
    return re.sub(r"\$\{\{\s*needs\.([A-Za-z0-9_-]+)\.result\s*\}\}", lambda m: results[m.group(1)], " ".join(template.split()))


def verdict(results):
    """Exit status of the gate step when each needed job ends with the given result."""
    job = gate()
    step = job["steps"][0]
    env = {k: expand(v, results) if k != "RESULTS" else " ".join(results[n] for n in job["needs"]) for k, v in step["env"].items()}
    run = subprocess.run(["bash", "-e", "-c", step["run"]], env={"PATH": "/usr/bin:/bin", **env}, capture_output=True, text=True)
    return run.returncode, run.stdout


def all_success():
    return {name: "success" for name in gate()["needs"]}


class GateVerdict(unittest.TestCase):
    def test_everything_green_passes(self):
        self.assertEqual(verdict(all_success())[0], 0)

    def test_the_gate_runs_after_a_cancel_and_the_run_it_is_in_is_not_green(self):
        self.assertEqual(gate()["if"], "always()")
        for name in gate()["needs"]:
            results = all_success()
            results[name] = "cancelled"
            code, out = verdict(results)
            self.assertEqual(code, 1, name)
            self.assertNotIn("passed", out.lower().replace("not passed", ""), name)

    def test_any_failure_fails(self):
        for name in gate()["needs"]:
            results = all_success()
            results[name] = "failure"
            self.assertEqual(verdict(results)[0], 1, name)

    def test_a_skipped_gate_carrying_job_fails_and_so_does_a_skipped_stripped(self):
        for name in self.strict():
            results = all_success()
            results[name] = "skipped"
            self.assertEqual(verdict(results)[0], 1, name)

    def test_a_job_that_may_skip_by_its_own_if_does_not_fail_the_gate(self):
        for name in LAX:
            results = all_success()
            results[name] = "skipped"
            self.assertEqual(verdict(results)[0], 0, name)

    def strict(self):
        env = gate()["steps"][0]["env"]
        named = set(re.findall(r"needs\.([A-Za-z0-9_-]+)\.result", env["MUST_RUN"] + " " + env["STRIPPED"]))
        return named


class GateCoverage(unittest.TestCase):
    def test_every_needed_job_is_strict_or_deliberately_lax(self):
        env = gate()["steps"][0]["env"]
        strict = set(re.findall(r"needs\.([A-Za-z0-9_-]+)\.result", env["MUST_RUN"] + " " + env["STRIPPED"]))
        needs = set(gate()["needs"])
        self.assertEqual(needs - strict - LAX, set(), "a job in `needs` is neither strict nor in LAX: decide which")
        self.assertEqual(strict - needs, set(), "the gate reads the result of a job it does not need")
        self.assertIn("stable", strict)
        self.assertIn("stripped", strict)

    def test_the_gate_needs_every_job_of_the_workflow_except_the_nightly_one(self):
        jobs = set(workflow()["jobs"])
        self.assertEqual(jobs - set(gate()["needs"]) - {"gate", "slow"}, set())

    def test_the_gate_job_is_the_required_check_name(self):
        self.assertEqual(gate()["name"], "CI")
        names = [j.get("name") for j in workflow()["jobs"].values()]
        self.assertEqual(names.count("CI"), 1, "ci_passed() in gate_reuse.py and site_policy.py wants exactly one `CI` job")


if __name__ == "__main__":
    sys.exit(unittest.main())
