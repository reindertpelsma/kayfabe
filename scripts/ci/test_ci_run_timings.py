"""scripts/ci/ci_run_timings.py: the numbers it prints are the ones in the job records.

STATUS: LIVE, 2026-11-17.

FALSIFIER (written before the code): queueing (created to started) is added to a job's run time; the run's wall time
ignores the queue wait of the last job; the gate's lag is measured from the wrong job; a skipped job counts as work.
"""

import io
import unittest
from contextlib import redirect_stdout

import ci_run_timings as T


def job(name, created, started, done, conclusion="success", steps=()):
    return dict(name=name, status="completed", conclusion=conclusion, created_at=created, started_at=started,
                completed_at=done, steps=[dict(name=n, started_at=a, completed_at=b, conclusion="success") for n, a, b in steps])


JOBS = [
    job("stable", "2026-11-17T10:00:00Z", "2026-11-17T10:30:00Z", "2026-11-17T12:30:00Z",
        steps=[("slow step", "2026-11-17T10:31:00Z", "2026-11-17T12:01:00Z"), ("quick step", "2026-11-17T12:01:00Z", "2026-11-17T12:02:00Z")]),
    job("stripped", "2026-11-17T10:00:00Z", "2026-11-17T10:10:00Z", "2026-11-17T10:40:00Z"),
    job("slow", "2026-11-17T10:00:00Z", "2026-11-17T10:00:00Z", "2026-11-17T10:00:00Z", conclusion="skipped"),
    job("CI", "2026-11-17T12:30:00Z", "2026-11-17T14:30:00Z", "2026-11-17T14:30:05Z"),
]


class Timings(unittest.TestCase):
    def test_queue_and_run_time_are_kept_apart(self):
        rows = {r["name"]: r for r in T.summarize(JOBS)["rows"]}
        self.assertEqual((rows["stable"]["queued"], rows["stable"]["ran"]), (30.0, 120.0))
        self.assertEqual((rows["stripped"]["queued"], rows["stripped"]["ran"]), (10.0, 30.0))

    def test_wall_time_includes_the_gates_wait_for_a_runner(self):
        s = T.summarize(JOBS)
        self.assertEqual(s["wall"], 270.1)

    def test_gate_lag_is_measured_from_the_last_other_job(self):
        self.assertEqual(T.summarize(JOBS)["gate_lag"], 120.0)

    def test_a_skipped_job_is_not_work_and_the_longest_job_is_the_longest_run(self):
        s = T.summarize(JOBS)
        self.assertEqual(s["runner_minutes"], 150.0)
        self.assertEqual(s["longest"]["name"], "stable")

    def test_slowest_steps_are_sorted_and_cut(self):
        steps = T.slowest_steps(T.summarize(JOBS)["rows"], 1)
        self.assertEqual(steps, [(90.0, "stable", "slow step")])

    def test_report_prints_without_error(self):
        out = io.StringIO()
        with redirect_stdout(out):
            T.report("fixture", JOBS, 2)
        self.assertIn("wall 270.1 min", out.getvalue())
        self.assertIn("slow step", out.getvalue())


if __name__ == "__main__":
    unittest.main()
