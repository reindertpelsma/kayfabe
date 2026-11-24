"""The job layout of .github/workflows/ci.yml after `stable` was split (2026-11-17): nothing dropped, nothing doubled.

STATUS: LIVE, 2026-11-17.

FALSIFIER (written before the code): a command that `stable` ran before the split is in no job (a harness stopped
running), in two jobs (it runs twice and costs a second runner), or in a job that the gate does not wait for; the
unittest shards do not add up to the whole of `scripts/ci` (the matrix and `--count` disagree); a job split out of
`stable` has no time limit, so a hung harness holds a runner for the default six hours.

HARNESSES below is the pinned list of every command the old `stable` job ran by hand (its two long steps, the six
Rust mutation steps, the gates with a harness). Removing one from ci.yml fails this test; to remove one on purpose,
remove it here in the same commit and say why. Adding a harness needs no edit here.
"""

import functools
import re
import sys
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/ci.yml"

HARNESSES = [
    "npx --yes prettier@3.6.2 --check 'ci/vast-broker/**/*.js' 'ci/vast-broker/**/*.mjs'",
    "npx --yes prettier@3.6.2 --config ci/vast-broker/.prettierrc.json --check 'ci/vast-admin/**/*.js' 'ci/vast-admin/**/*.mjs'",
    'python3 -m pip install black==25.1.0',
    'python3 -m black --check --config ci/vast-broker/pyproject.toml ci/vast-broker',
    'node --test ci/vast-broker/test',
    'node ci/vast-broker/scripts/mutate-tiers.mjs',
    'node ci/vast-broker/scripts/check-bundle-pin.mjs',
    'node ci/vast-broker/scripts/bundle-pin-mutants.mjs',
    'python3 scripts/ci/oidc_retry_mutants.py',
    'python3 -B -I scripts/ci/test_propimage.py',
    'python3 -B -I scripts/ci/propimage_mutants.py',
    'python3 -B -I scripts/ci/test_propimage_linux.py',
    'python3 -B -I scripts/ci/propimage_linux_mutants.py',
    'python3 -B -I scripts/ci/clockrule_mutants.py',
    'node ci/vast-broker/release-matrix-mutants.mjs',
    'node ci/vast-broker/release-safety-mutants.mjs',
    'node ci/vast-broker/release-resilience-mutants.mjs',
    'node ci/vast-broker/release-vendor-mutants.mjs',
    'node ci/vast-broker/rv2-mutants.mjs',
    'node ci/vast-broker/driver-pins-mutants.mjs',
    'node ci/vast-broker/simulation-mutants.mjs',
    'python3 scripts/ci/profile_resources.py',
    'node ci/vast-broker/profile-resources-mutants.mjs',
    'node ci/vast-broker/evidence-mutants.mjs',
    'node ci/vast-broker/report-mutants.mjs',
    'node ci/vast-broker/provider-tolerance-mutants.mjs',
    'node ci/vast-broker/denied-countries-mutants.mjs',
    'node ci/vast-broker/propimages-mutants.mjs',
    'python3 -B -I scripts/ci/bandwidth_check.py',
    'python3 -B -I scripts/ci/bandwidth_mutants.py',
    'npm ci --prefix ci/vast-admin --ignore-scripts --no-audit --no-fund',
    'node --test ci/vast-admin/test/*.test.mjs',
    'bash scripts/ci/clippy.sh',
    'cargo fmt --all --check',
    'python3 scripts/ci/full_tree_tests.py run',
    'python3 scripts/ci/oracle_gate_mutants.py',
    'python3 scripts/ci/video_class_coverage_mutants.py',
    'python3 scripts/ci/hide_untabled_video_mutants.py',
    'python3 scripts/ci/va_apply_bounds_mutants.py',
    'python3 scripts/ci/guest_influence_mutants.py',
    'python3 scripts/ci/windows_profile_mutants.py',
    'python3 scripts/ci/va_profile_mutants.py',
    'python3 scripts/ci/dependencies.py',
    'python3 scripts/ci/debt.py frozen',
    'python3 scripts/ci/workflow_lint.py',
    'python3 scripts/ci/ignore_ladder.py check',
    'python3 scripts/ci/image_size_mutants.py',
    'python3 scripts/ci/kfci_mutants.py',
    'python3 scripts/ci/instruction_mutants.py',
    'python3 scripts/ci/kfroot_mutants.py --jobs 4',
    'python3 scripts/ci/box_downloader_mutants.py',
    'python3 scripts/ci/propimage_apps_mutants.py',
    'python3 scripts/ci/propimage_runner_mutants.py',
    'python3 scripts/ci/clocknet_assert_mutants.py',
    'python3 scripts/ci/stream_url_mutants.py',
    'python3 scripts/ci/doc_steps_mutants.py',
    'python3 scripts/ci/derive_fault_names_mutants.py',
    "python3 -m unittest discover -s scripts/evidence -p 'test_*.py'",
    'bash scripts/bench/qemu_identity_selftest.sh',
    'python3 -I scripts/bench/va_profile_table.py --self-test',
    'bash scripts/bench/box/birth_census.sh --selftest',
    'python3 scripts/ci/debt.py claims',
    'python3 scripts/ci/no_gpu_uuids.py',
    'python3 scripts/ci/gpu_uuid_mutants.py',
    'python3 scripts/evidence/gpu_uuid_scrub_mutants.py',
    'python3 scripts/ci/no_private_refs.py',
    'python3 scripts/ci/caller_contracts.py',
    'python3 scripts/ci/test_caller_contracts.py',
    'python3 -B -I scripts/ci/env_allowlist.py',
    'python3 -B -I scripts/ci/comment_hygiene.py',
    'python3 -B -I scripts/ci/env_allowlist_mutants.py',
    'python3 -B -I scripts/ci/comment_hygiene_mutants.py',
]

SPLIT = ["worker-tests", "worker-mutants-a", "worker-mutants-b", "middle-apps", "mutants-a", "mutants-b", "selftests", "rust-mutants-a", "rust-mutants-b"]
LONGEST_ALLOWED_MINUTES = 90


@functools.lru_cache(maxsize=1)
def jobs():
    return yaml.safe_load(WORKFLOW.read_text())["jobs"]


def logical_lines(run):
    out, current = [], ""
    for raw in str(run).split("\n"):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        current = (current + " " + line) if current else line
        if current.endswith("\\"):
            current = current[:-1].rstrip()
            continue
        out.append(" ".join(current.split()))
        current = ""
    return out


def commands_by_job():
    found = {}
    for name, job in jobs().items():
        for step in job.get("steps", []):
            for line in logical_lines(step.get("run", "")):
                found.setdefault(line, []).append(name)
    return found


class Layout(unittest.TestCase):
    def test_every_command_of_the_old_stable_job_runs_in_exactly_one_job(self):
        found = commands_by_job()
        for command in HARNESSES:
            where = found.get(" ".join(command.split()), [])
            self.assertEqual(len(where), 1, "%r runs in %s" % (command, where or "no job"))

    def test_the_split_jobs_are_all_in_the_workflow_and_stable_kept_its_name(self):
        for name in SPLIT + ["stable"]:
            self.assertIn(name, jobs())
        self.assertTrue(jobs()["stable"]["name"].startswith("stable"))

    def test_the_jobs_that_were_split_out_run_none_of_the_commands_that_stay_in_stable(self):
        found = commands_by_job()
        for command in ("bash scripts/ci/clippy.sh", "cargo fmt --all --check", "python3 scripts/ci/full_tree_tests.py run",
                        "python3 scripts/ci/workflow_lint.py", "python3 scripts/ci/no_gpu_uuids.py"):
            self.assertEqual(found[command], ["stable"], command)

    def test_every_split_job_has_a_time_limit_and_every_harness_job_is_in_the_gate(self):
        needs = set(jobs()["gate"]["needs"])
        for name in SPLIT:
            limit = jobs()[name].get("timeout-minutes")
            self.assertIsInstance(limit, int, name)
            self.assertLessEqual(limit, LONGEST_ALLOWED_MINUTES, name)
            self.assertIn(name, needs, name)

    def test_the_worker_dependency_is_installed_before_any_harness_that_may_import_it(self):
        for name in ("worker-mutants-a", "worker-mutants-b", "middle-apps", "mutants-a", "mutants-b", "selftests"):
            steps = jobs()[name]["steps"]
            first_run = next(s["run"] for s in steps if "run" in s)
            self.assertIn("npm ci --prefix ci/vast-broker --omit=dev", logical_lines(first_run)[0], name)

    def test_worker_tests_install_the_dev_dependencies_so_the_bundle_test_runs(self):
        # test/bundle.test.js skips without wrangler (a dev dependency); an --omit=dev install let a stale pin through.
        installs = [line for line in logical_lines("\n".join(s.get("run", "") for s in jobs()["worker-tests"]["steps"]))
                    if line.startswith("npm ci --prefix ci/vast-broker")]
        self.assertEqual(len(installs), 1, installs)
        self.assertNotIn("--omit=dev", installs[0])

    def test_stable_and_stripped_have_a_time_limit_too(self):
        for name in ("stable", "stripped"):
            self.assertIsInstance(jobs()[name].get("timeout-minutes"), int, name)

    def test_the_rust_jobs_share_the_cache_entry_stable_has(self):
        for name in ("rust-mutants-a", "rust-mutants-b"):
            uses = {s["uses"].split("@")[0]: s.get("with", {}) for s in jobs()[name]["steps"] if "uses" in s}
            self.assertEqual(uses["Swatinem/rust-cache"].get("shared-key"), "stable", name)

    def test_the_rust_jobs_use_the_pinned_toolchain_and_both_build_targets(self):
        for name in ("rust-mutants-a", "rust-mutants-b"):
            uses = {s["uses"].split("@")[0]: s.get("with", {}) for s in jobs()[name]["steps"] if "uses" in s}
            self.assertEqual(uses["dtolnay/rust-toolchain"]["toolchain"], "1.99.0", name)
            self.assertIn("x86_64-unknown-uefi", uses["dtolnay/rust-toolchain"]["targets"], name)
            self.assertIn("x86_64-unknown-linux-musl", uses["dtolnay/rust-toolchain"]["targets"], name)
            self.assertIn("Swatinem/rust-cache", uses, name)


class LocalRunner(unittest.TestCase):
    def test_ci_gates_sh_reads_exactly_the_jobs_that_were_split_out_of_stable(self):
        text = (ROOT / "scripts/ci_gates.sh").read_text()
        listed = re.search(r"SPLIT_JOBS = \((.*?)\)\n", text, re.S)
        self.assertIsNotNone(listed)
        self.assertEqual(re.findall(r'"([a-z0-9-]+)"', listed[1]), SPLIT)


class Shards(unittest.TestCase):
    def shard_step(self):
        steps = [s for s in jobs()["selftests"]["steps"] if "unittest_shard.py" in str(s.get("run", ""))]
        self.assertEqual(len(steps), 1)
        return steps[0]["run"]

    def test_the_matrix_is_the_partition_the_command_asks_for(self):
        matrix = jobs()["selftests"]["strategy"]["matrix"]["shard"]
        count = int(re.search(r"--count (\d+)", self.shard_step())[1])
        self.assertEqual(matrix, list(range(1, count + 1)))
        self.assertIn("--index ${{ matrix.shard }}", self.shard_step())

    def test_the_shards_run_the_same_discovery_the_plain_command_ran(self):
        run = self.shard_step()
        self.assertIn("-s scripts/ci -p 'test_*.py'", run)

    def test_no_shard_may_fail_fast_and_hide_another(self):
        self.assertIs(jobs()["selftests"]["strategy"]["fail-fast"], False)


class MiddleApps(unittest.TestCase):
    def command(self):
        lines = [l for l in logical_lines(jobs()["middle-apps"]["steps"][1]["run"]) if "middle_apps_mutants" in l]
        self.assertEqual(len(lines), 1, lines)
        return lines[0]

    def test_the_slices_are_every_mutant_once_and_the_matrix_is_as_long_as_the_command_says(self):
        count = int(re.fullmatch(r"python3 scripts/ci/middle_apps_mutants.py --shard \$\{\{ matrix.shard \}\}/(\d+)", self.command())[1])
        self.assertEqual(jobs()["middle-apps"]["strategy"]["matrix"]["shard"], list(range(1, count + 1)))
        self.assertIs(jobs()["middle-apps"]["strategy"]["fail-fast"], False)

    def test_the_middle_apps_harness_is_not_run_with_workers_on_one_runner(self):
        self.assertNotIn("--jobs", self.command())


if __name__ == "__main__":
    sys.exit(unittest.main())
