"""2026-11-15, stages S1 and S3 (OWNER_RULINGS section AR): the payload instruction file, GPU-free.

FALSIFIER (written before the code): a hostile or malformed instruction (unknown field, wrong type, oversize,
duplicate key, NaN, a traversing artifact path, another schema version) is accepted; the environment path and the
file path yield different objects for the same request or disagree on what is refused; `--dry-run` touches a
device, a directory or the result file; `--only/--skip` selects a stage that is not in the plan or an empty set;
the result file does not carry the sha256 of the instruction bytes that were read; or the payload reads the run
token or the broker URL.
"""

import hashlib
import importlib.util
import itertools
import json
import os
from pathlib import Path
import re
import tempfile
import unittest
from unittest.mock import patch

import payload_instruction as pi
import test_vast_payload_v7 as v7

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
PAYLOAD = HERE / "vast_payload.py"


def payload():
    spec = importlib.util.spec_from_file_location("payload_instruction_t", PAYLOAD)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def good(**over):
    doc = {
        "schema": pi.SCHEMA_ID,
        "tier": "small",
        "windows": False,
        "windows_runs": 1,
        "stages": [],
        "spec": None,
        "winapps": None,
        "linux_apps": None,
        "rehearsal": False,
    }
    doc.update(over)
    return doc


def raw(doc):
    return json.dumps(doc).encode()


def matrix_env():
    """A complete, valid proposed release matrix in its KF_* spelling (the Worker's, test_vast_release_matrix.py)."""
    import base64

    enc = lambda v: base64.urlsafe_b64encode(json.dumps(v).encode()).decode().rstrip("=")
    matrix = json.loads((ROOT / "ci/vast-broker/release-matrix.json").read_text())
    row = matrix["dies"][1]
    return {
        "KF_RELEASE_DRIVER_PINS": enc(matrix["driver_installers"]),
        "KF_HOST_DRIVER": row["drivers"]["host_driver"],
        "KF_LINUX_GUEST_DRIVER": row["drivers"]["linux_guest_driver"],
        "KF_WINDOWS_GUEST_DRIVER": row["drivers"]["windows_guest_driver"],
        "KF_WINGATE_GUEST_DRIVER": row["windows_guest_abi"],
        "KF_RELEASE_SUBRUNS": enc(dict(status="proposed", subruns=row["subruns"], cross_subrun=row["cross_subrun"])),
    }


class Hostile(unittest.TestCase):
    def refuses(self, data):
        with self.assertRaises(pi.InstructionError):
            pi.parse(data if isinstance(data, bytes) else raw(data))

    def test_a_good_document_parses(self):
        self.assertEqual(pi.parse(raw(good())).tier, "small")

    def test_unknown_top_level_field_is_refused(self):
        self.refuses(good(extra=1))
        self.refuses(good(KF_RUN_TOKEN="x"))

    def test_unknown_artifact_field_is_refused(self):
        self.refuses(good(artifacts=[{"name": "a", "path": "x", "sha256": "0" * 64, "mode": "0777"}]))

    def test_missing_required_field_is_refused(self):
        doc = good()
        del doc["tier"]
        self.refuses(doc)

    def test_wrong_types_are_refused(self):
        for key, value in [
            ("windows", "true"), ("windows", 1), ("windows_runs", "2"), ("windows_runs", True), ("windows_runs", 3),
            ("tier", 3), ("tier", "huge"), ("stages", "specviewperf"), ("stages", [1]), ("spec", 1),
            ("rehearsal", None), ("linux_apps", True), ("tree_sha", "A" * 40), ("tree_sha", "abc"),
            ("requirement_id", ""), ("requirement_id", "a b"), ("artifacts", {}),
        ]:
            with self.subTest(key=key, value=value):
                self.refuses(good(**{key: value}))

    def test_reserved_design_only_stage_names_are_refused(self):
        for name in ("two_guests", "trio", "llm_parity"):
            self.refuses(good(tier="large", windows=True, stages=[name]))

    def test_not_an_object_or_not_json_is_refused(self):
        for data in (b"[]", b'"x"', b"null", b"3", b"", b"{", b"\xff\xfe{}"):
            with self.subTest(data=data):
                self.refuses(data)

    def test_duplicate_keys_and_non_finite_numbers_are_refused(self):
        text = json.dumps(good())
        self.refuses(text[:-1].encode() + b', "tier": "large"}')
        self.refuses(text.replace('"windows_runs": 1', '"windows_runs": NaN').encode())
        self.refuses(text.replace('"windows_runs": 1', '"windows_runs": Infinity').encode())

    def test_deep_nesting_is_refused_not_crashed(self):
        self.refuses(b"[" * 100000 + b"]" * 100000)
        self.refuses(b'{"a":' * 5000 + b"1" + b"}" * 5000)

    def test_oversize_is_refused(self):
        self.refuses(raw(good(requirement_id="a" * 100)) + b" " * pi.MAX_BYTES)
        # exactly at the bound is still read; one byte more is not
        body = raw(good())
        self.assertEqual(pi.parse(body + b" " * (pi.MAX_BYTES - len(body))).tier, "small")
        self.refuses(body + b" " * (pi.MAX_BYTES - len(body) + 1))

    def test_oversize_collections_are_refused(self):
        art = [{"name": "n%d" % i, "path": "p%d" % i, "sha256": "0" * 64} for i in range(33)]
        self.refuses(good(artifacts=art))
        self.assertEqual(len(pi.parse(raw(good(artifacts=art[:32]))).artifacts), 32)
        self.refuses(good(tier="large", windows=True, stages=["specviewperf"] * 9, spec="both"))

    def test_path_traversal_in_artifact_paths_is_refused(self):
        for path in ("../x", "/etc/passwd", "a/../b", "a//b", "./a", "a/./b", "a/", "..", ".hidden", "a/.b", "a\\b",
                     "a\nb", "x" * 257, "", "a/..", "%2e%2e/x", "~/x"):
            with self.subTest(path=path):
                self.refuses(good(artifacts=[{"name": "a", "path": path, "sha256": "0" * 64}]))

    def test_safe_artifact_paths_are_accepted(self):
        for path in ("a", "dir/file.run", "a/b-c_d.e/f"):
            with self.subTest(path=path):
                self.assertEqual(
                    pi.parse(raw(good(artifacts=[{"name": "a", "path": path, "sha256": "0" * 64}]))).artifacts[0].path, path
                )

    def semantic_refuses(self, **over):
        i = pi.parse(raw(good()))
        with self.assertRaises(pi.InstructionError):
            pi.check_semantics(pi.Instruction(**{**i.__dict__, **over}))

    # defence in depth: even if the schema pattern were loosened, the object refuses a traversing path
    def test_semantic_traversal_check_stands_alone(self):
        for path in ("../x", "/x", "a/..", "a/../b"):
            with self.subTest(path=path):
                self.semantic_refuses(artifacts=(pi.Artifact("a", path, "0" * 64),))

    def test_semantic_empty_and_dot_segments_stand_alone(self):
        for path in ("a//b", "a/./b", "./a", "a/"):
            with self.subTest(path=path):
                self.semantic_refuses(artifacts=(pi.Artifact("a", path, "0" * 64),))

    def test_semantic_duplicate_names_stand_alone(self):
        self.semantic_refuses(artifacts=(pi.Artifact("a", "x", "0" * 64), pi.Artifact("a", "y", "0" * 64)))

    def test_semantic_reserved_stage_stands_alone(self):
        self.semantic_refuses(tier="large", windows=True, stages=("trio",), spec=None)

    def test_semantic_duplicate_stage_stands_alone(self):
        self.semantic_refuses(tier="large", windows=True, stages=("specviewperf", "specviewperf"), spec="both")

    def test_semantic_apps_profile_stands_alone(self):
        self.semantic_refuses(tier="small", linux_apps="middle")
        self.semantic_refuses(tier="middle", linux_apps="large")

    def test_duplicate_names_anywhere_in_the_list_are_refused(self):
        row = lambda n, p: {"name": n, "path": p, "sha256": "0" * 64}
        self.refuses(good(artifacts=[row("a", "x"), row("b", "y"), row("a", "z")]))

    def test_duplicate_nested_key_is_refused(self):
        text = json.dumps(good(artifacts=[{"name": "a", "path": "x", "sha256": "0" * 64}]))
        self.refuses(text.replace('"path": "x"', '"path": "x", "path": "y"').encode())

    def test_oversize_file_is_refused_by_load(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "big.json"
            path.write_bytes(raw(good()) + b" " * (2 * pi.MAX_BYTES))
            with self.assertRaises(pi.InstructionError):
                pi.load(path)

    def test_duplicate_artifact_names_and_bad_digests_are_refused(self):
        a = {"name": "a", "path": "x", "sha256": "0" * 64}
        self.refuses(good(artifacts=[a, {**a, "path": "y"}]))
        self.refuses(good(artifacts=[{**a, "sha256": "0" * 63}]))
        self.refuses(good(artifacts=[{**a, "sha256": "G" * 64}]))

    def test_schema_version_mismatch_is_refused(self):
        for version in ("kayfabe.payload-instruction/2", "kayfabe.payload-instruction/0", "x", 1, None):
            with self.subTest(version=version):
                self.refuses(good(schema=version))
        doc = good()
        del doc["schema"]
        self.refuses(doc)

    def test_cross_field_rules_are_refused(self):
        for over in (
            dict(windows_runs=2),
            dict(winapps="mid"),
            dict(rehearsal=True),
            dict(tier="middle", rehearsal=True),
            dict(tier="middle", windows=True, stages=["specviewperf"], spec="both"),
            dict(tier="large", stages=["specviewperf"], spec="both"),  # needs windows
            dict(tier="large", windows=True, stages=["specviewperf"]),  # slot without a lane
            dict(tier="large", windows=True, spec="both"),  # lane without the slot
            dict(tier="small", linux_apps="middle"),
            dict(tier="middle", linux_apps="large"),
            dict(tier="unknown", linux_apps="middle"),
        ):
            with self.subTest(**over):
                self.refuses(good(**over))

    def test_a_release_matrix_cannot_be_spelled_in_a_file(self):
        self.refuses(good(host_driver="580.159.04"))
        self.refuses(good(release_subruns="x"))

    def test_the_file_is_read_once_and_hashed_as_read(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "i.json"
            data = raw(good(tier="large", requirement_id="req-1"))
            path.write_bytes(data)
            inst, digest = pi.load(path)
            self.assertEqual(digest, hashlib.sha256(data).hexdigest())
            self.assertEqual(inst.requirement_id, "req-1")

    def test_unreadable_file_and_directory_are_refused(self):
        with tempfile.TemporaryDirectory() as d:
            for target in (Path(d) / "missing.json", Path(d)):
                with self.assertRaises(pi.InstructionError):
                    pi.load(target)

    def test_the_two_schema_copies_are_byte_identical(self):
        broker = ROOT / "ci/vast-broker/planner/schemas/instruction.schema.json"
        self.assertEqual(broker.read_bytes(), pi.SCHEMA_PATH.read_bytes())


def env_spelling(doc):
    """The KF_* spelling of an instruction document, or None when the environment cannot say it."""
    env = {"KF_MERGE_WINDOWS": "1" if doc["windows"] else "0"}
    if doc["tier"] != "unknown":
        env["KF_TIER"] = doc["tier"]
    if doc["windows_runs"] == 2:
        env["KF_WINDOWS_RUNS"] = "2"
    if doc["stages"]:
        env["KF_STAGES"] = ":".join(doc["stages"])
    if doc["spec"]:
        env["KF_SPEC"] = doc["spec"]
    if doc["winapps"]:
        env["KF_WINAPPS"] = doc["winapps"]
    if doc["rehearsal"]:
        env["KF_RELEASE_REHEARSAL"] = "1"
    if doc["linux_apps"] == "large":
        return None  # the owner switch APPS_STAGE_ENABLED, not the environment, decides the default large apps
    if doc["linux_apps"] == "middle":
        env["KF_LINUX_APPS"] = "1"
    return env


class EnvAndFileAgree(unittest.TestCase):
    def setUp(self):
        self.m = payload()

    def test_every_combination_is_accepted_or_refused_by_both_and_builds_the_same_object(self):
        tiers = ("small", "middle", "large", "adhoc", "unknown")
        grid = itertools.product(
            tiers, (False, True), (1, 2), ([], ["specviewperf"]), (None, "lanea", "both"), (None, "mid", "full"),
            (None, "middle"), (False, True),
        )
        accepted = refused = 0
        with patch.object(self.m, "APPS_STAGE_ENABLED", False):
            for tier, windows, runs, stages, spec, winapps, apps, rehearsal in grid:
                doc = good(tier=tier, windows=windows, windows_runs=runs, stages=stages, spec=spec, winapps=winapps,
                           linux_apps=apps, rehearsal=rehearsal)
                try:
                    from_file = pi.from_dict(doc)
                except pi.InstructionError:
                    from_file = None
                env = env_spelling(doc)
                try:
                    from_env = self.m.instruction_from_env(dict(env))
                except self.m.EvidenceError:
                    from_env = None
                with self.subTest(doc=doc):
                    self.assertEqual(from_file, from_env)
                accepted += from_file is not None
                refused += from_file is None
        self.assertGreater(accepted, 30)
        self.assertGreater(refused, 300)

    def test_the_default_large_apps_profile_is_the_large_list(self):
        with patch.object(self.m, "APPS_STAGE_ENABLED", True):
            large = self.m.instruction_from_env({"KF_TIER": "large", "KF_MERGE_WINDOWS": "0"})
            middle = self.m.instruction_from_env({"KF_TIER": "middle", "KF_MERGE_WINDOWS": "0"})
            explicit = self.m.instruction_from_env({"KF_TIER": "large", "KF_MERGE_WINDOWS": "0", "KF_LINUX_APPS": "1"})
        self.assertEqual((large.linux_apps, middle.linux_apps, explicit.linux_apps), ("large", "middle", "middle"))
        self.assertEqual(pi.from_dict(large.to_dict()), large)

    def test_a_missing_windows_statement_is_the_master_gate_with_source_default(self):
        inst = self.m.instruction_from_env({})
        self.assertTrue(inst.windows)
        self.assertEqual(inst.source, "default")

    def test_env_round_trips_through_a_written_file(self):
        env = {"KF_MERGE_WINDOWS": "1", "KF_TIER": "large", "KF_WINDOWS_RUNS": "2", "KF_STAGES": "specviewperf",
               "KF_SPEC": "laneb", "KF_WINAPPS": "mid", "KF_LINUX_APPS": "1", "KF_RELEASE_REHEARSAL": "1"}
        inst = self.m.instruction_from_env(dict(env))
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "i.json"
            digest = pi.write(inst, path)
            loaded, read_digest = pi.load(path)
        self.assertEqual(loaded, inst)
        self.assertEqual(digest, read_digest)

    def test_the_environment_adapter_still_refuses_what_it_refused(self):
        for env in (
            {"KF_MERGE_WINDOWS": "2"}, {"KF_TIER": "huge"}, {"KF_STAGES": "trio", "KF_TIER": "large"},
            {"KF_SPEC": "lanea"}, {"KF_RELEASE_REHEARSAL": "1", "KF_TIER": "middle"}, {"KF_WINAPPS": "mid", "KF_MERGE_WINDOWS": "0"},
            {"KF_HOST_DRIVER": "580.159.04"},
        ):
            with self.subTest(env=env):
                with self.assertRaises(self.m.EvidenceError):
                    self.m.instruction_from_env(dict(env))

    def test_a_complete_release_matrix_is_still_refused_as_proposed(self):
        env = {**matrix_env(), "KF_TIER": "small", "KF_MERGE_WINDOWS": "0"}
        with self.assertRaises(self.m.EvidenceError) as caught:
            self.m.instruction_from_env(env)
        self.assertEqual(caught.exception.name, "release_retry_matrix_proposed")

    def test_a_file_beside_instruction_variables_is_refused(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "i.json"
            path.write_bytes(raw(good()))
            for name in self.m.INSTRUCTION_ENV:
                with self.subTest(name=name):
                    with self.assertRaises(self.m.EvidenceError):
                        self.m.instruction_from_file(path, {name: "1"})
            inst, digest = self.m.instruction_from_file(path, {"KF_APPS_BUDGET_S": "9", "KF_APPS_PROVISION": "0"})
            self.assertEqual(inst.tier, "small")

    def test_instruction_names_list_every_variable_the_env_adapter_reads(self):
        source = PAYLOAD.read_text()
        start = source.index("def instruction_from_env")
        # every KF_* the legacy functions pop, other than the stage knobs, is covered by the conflict list
        popped = set(re.findall(r'environ\.pop\("(KF_[A-Z_]+)"', source))
        knobs = set(self.m.APPS_STAGE_KNOBS) | {"KF_APPS_PROVISION"}
        self.assertEqual(popped - knobs, set(self.m.INSTRUCTION_ENV) - set(self.m.RELEASE_DRIVER_FIELDS) - {
            "KF_WINGATE_GUEST_DRIVER", "KF_RELEASE_SUBRUNS", "KF_RELEASE_DRIVER_PINS"})
        self.assertGreater(start, 0)

    def test_runnable_stage_slots_match_the_payload(self):
        plumbing = tuple(n for n, kind in self.m.STAGE_SLOTS.items() if kind == "PLUMBING")
        self.assertEqual(plumbing, pi.RUNNABLE_EXTRA_STAGES)


class Plan(unittest.TestCase):
    def setUp(self):
        self.m = payload()

    def plan(self, **over):
        return self.m.plan_stage_names(pi.from_dict(good(**over)))

    def test_plan_order(self):
        self.assertEqual(self.plan(), ["provision", "cuda_host", "merge_bar", "cuda_guest"])
        full = self.plan(tier="large", windows=True, windows_runs=2, winapps="mid", linux_apps="middle",
                         stages=["specviewperf"], spec="both")
        self.assertEqual(full, ["provision", "cuda_host", "merge_bar", "cuda_guest", "apps", "windows_gate",
                                "windows_apps", "windows_gate_2", "specviewperf"])

    def test_select_only_and_skip(self):
        plan = self.plan(windows=True)
        self.assertEqual(self.m.select_stages(plan, [], []), plan)
        self.assertEqual(self.m.select_stages(plan, ["merge_bar"], []), ["merge_bar"])
        self.assertEqual(self.m.select_stages(plan, ["merge_bar", "provision"], []), ["provision", "merge_bar"])
        self.assertEqual(self.m.select_stages(plan, [], ["windows_gate", "provision"]), ["cuda_host", "merge_bar", "cuda_guest"])
        self.assertEqual(self.m.select_stages(plan, ["merge_bar", "cuda_host"], ["cuda_host"]), ["merge_bar"])

    def test_unknown_or_unplanned_names_and_empty_selection_are_refused(self):
        plan = self.plan()
        for only, skip in ((["nope"], []), ([], ["nope"]), (["windows_gate"], []), ([], ["apps"]),
                           (["merge_bar"], ["merge_bar"]), ([], list(plan))):
            with self.subTest(only=only, skip=skip):
                with self.assertRaises(self.m.EvidenceError):
                    self.m.select_stages(plan, only, skip)


class DryRun(unittest.TestCase):
    def run_main(self, argv, env=None):
        m = payload()
        out = []
        boom = AssertionError("dry run touched the box")
        with (
            patch("sys.argv", ["vast_payload.py", *argv]),
            patch.dict(os.environ, env or {}, clear=False),
            patch.object(m.subprocess, "run", side_effect=boom),
            patch.object(m.subprocess, "check_output", side_effect=boom),
            patch.object(m, "persist_result", side_effect=boom),
            patch.object(m, "execute_stages", side_effect=boom),
            patch.object(m, "hardware", side_effect=boom),
            patch.object(m, "prepare_checkout", side_effect=boom),
            patch.object(m.shutil, "disk_usage", side_effect=boom),
            patch.object(Path, "mkdir", side_effect=boom),
            patch("builtins.print", side_effect=lambda *a, **k: out.append(" ".join(map(str, a)))),
        ):
            for name in list(os.environ):
                if name.startswith("KF_") and name not in (env or {}):
                    os.environ.pop(name)
            rc = m.main()
        return rc, "\n".join(out)

    def write(self, d, **over):
        path = Path(d) / "i.json"
        path.write_bytes(raw(good(**over)))
        return str(path), hashlib.sha256(path.read_bytes()).hexdigest()

    def test_dry_run_from_a_file_prints_the_plan_and_the_hash_and_runs_nothing(self):
        with tempfile.TemporaryDirectory() as d:
            path, digest = self.write(d, tier="large", windows=True, winapps="small")
            rc, out = self.run_main(["--dry-run", "--instruction", path])
        self.assertEqual(rc, 0)
        self.assertIn("KF_PAYLOAD_TIER windows=1 source=instruction tier=large", out)
        self.assertIn("KF_PAYLOAD_INSTRUCTION sha256=" + digest, out)
        self.assertIn("KF_PAYLOAD_PLAN stages=provision:cuda_host:merge_bar:cuda_guest:windows_gate:windows_apps", out)

    def test_dry_run_from_the_environment_needs_no_sha_nor_deadline(self):
        rc, out = self.run_main(["--dry-run"], {"KF_TIER": "small", "KF_MERGE_WINDOWS": "0"})
        self.assertEqual(rc, 0)
        self.assertIn("source=worker tier=small", out)
        self.assertIn("KF_PAYLOAD_PLAN stages=provision:cuda_host:merge_bar:cuda_guest planned=", out)
        self.assertNotIn("KF_PAYLOAD_INSTRUCTION ", out)

    def test_dry_run_with_a_file_and_instruction_variables_is_refused(self):
        with tempfile.TemporaryDirectory() as d:
            path, _ = self.write(d)
            rc, out = self.run_main(["--dry-run", "--instruction", path], {"KF_TIER": "small"})
        self.assertEqual((rc, out), (1, "KF_PAYLOAD_FAIL suite=preflight"))

    def test_dry_run_of_a_release_matrix_is_refused(self):
        rc, out = self.run_main(["--dry-run"], {**matrix_env(), "KF_MERGE_WINDOWS": "0"})
        self.assertEqual((rc, out), (1, "KF_PAYLOAD_FAIL suite=release_retry_matrix_proposed"))

    def test_dry_run_hash_is_of_the_bytes_read(self):
        with tempfile.TemporaryDirectory() as d:
            path, digest = self.write(d, requirement_id="r-2")
            rc, out = self.run_main(["--dry-run", "--instruction", path])
        self.assertEqual(re.findall(r"^KF_PAYLOAD_INSTRUCTION sha256=([0-9a-f]{64})$", out, re.M), [digest])

    def test_dry_run_skipping_everything_is_refused(self):
        argv = ["--dry-run"]
        for name in ("provision", "cuda_host", "merge_bar", "cuda_guest"):
            argv += ["--skip", name]
        rc, out = self.run_main(argv, {"KF_MERGE_WINDOWS": "0"})
        self.assertEqual((rc, out), (1, "KF_PAYLOAD_FAIL suite=preflight"))

    def test_dry_run_applies_only_and_skip_and_names_the_full_plan(self):
        rc, out = self.run_main(["--dry-run", "--only", "merge_bar", "--only", "cuda_guest"], {"KF_MERGE_WINDOWS": "0"})
        self.assertEqual(rc, 0)
        self.assertIn("KF_PAYLOAD_PLAN stages=merge_bar:cuda_guest planned=provision:cuda_host:merge_bar:cuda_guest", out)

    def test_dry_run_refusals_are_rc1_and_say_nothing_else(self):
        rc, out = self.run_main(["--dry-run", "--only", "windows_gate"], {"KF_MERGE_WINDOWS": "0"})
        self.assertEqual((rc, out), (1, "KF_PAYLOAD_FAIL suite=preflight"))
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "bad.json"
            path.write_bytes(raw(good(extra=1)))
            rc, out = self.run_main(["--dry-run", "--instruction", str(path)])
        self.assertEqual(rc, 1)
        self.assertIn("KF_PAYLOAD_FAIL suite=preflight", out)
        self.assertNotIn("KF_PAYLOAD_PLAN", out)

    def test_the_payload_never_names_the_token_or_the_broker(self):
        text = PAYLOAD.read_text() + (HERE / "payload_instruction.py").read_text()
        for name in ("KF_RUN_TOKEN", "KF_BROKER_URL", "KF_RUN_ID", "KF_RUNNER_NONCE", "run-token", "/run/kf-ci"):
            self.assertNotIn(name, text.replace("KF_RUNNER_NONCE_UNUSED", ""), name)

    def test_emit_instruction_writes_the_env_object_and_runs_nothing(self):
        with tempfile.TemporaryDirectory() as d:
            target = Path(d) / "out.json"
            rc, out = self.run_main(["--emit-instruction", str(target)], {"KF_TIER": "middle", "KF_MERGE_WINDOWS": "0"})
            self.assertEqual(rc, 0)
            inst, digest = pi.load(target)
        self.assertEqual((inst.tier, inst.windows), ("middle", False))
        self.assertIn("KF_PAYLOAD_INSTRUCTION_WRITTEN sha256=" + digest, out)


class ThroughMain(unittest.TestCase):
    setUp = v7.V7Tests.setUp
    _main_fixture = v7.V7Tests._main_fixture

    def run_with(self, doc, extra_args=(), **kw):
        orig = self.m.parse_args
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "i.json"
            path.write_bytes(raw(doc))
            with patch.object(self.m, "parse_args", side_effect=lambda argv: orig([*argv, "--instruction", str(path), *extra_args])):
                rc, result, calls = self._main_fixture(windows=None, **kw)
            return rc, result, calls, hashlib.sha256(path.read_bytes()).hexdigest()

    def test_file_run_executes_the_plan_and_records_the_hash(self):
        rc, result, calls, digest = self.run_with(good())
        self.assertEqual(rc, 0, result)
        self.assertEqual(calls, ["provision", "cuda_host", "merge_bar", "cuda_guest"])
        self.assertEqual(result["instruction_sha256"], digest)
        self.assertNotIn("selection", result)

    def test_only_and_skip_select_and_are_recorded(self):
        rc, result, calls, digest = self.run_with(good(), ["--skip", "cuda_host", "--skip", "cuda_guest"])
        self.assertEqual(calls, ["provision", "merge_bar"])
        self.assertEqual(len(result["instruction_sha256"]), 64)
        self.assertEqual(result["selection"], {"only": [], "skip": ["cuda_host", "cuda_guest"]})

    def test_only_is_recorded_and_selects(self):
        rc, result, calls, _ = self.run_with(good(), ["--only", "merge_bar"])
        self.assertEqual(calls, ["merge_bar"])
        self.assertEqual(result["selection"], {"only": ["merge_bar"], "skip": []})

    def test_a_refused_instruction_is_a_preflight_fail_with_no_stage_run(self):
        rc, result, calls, _ = self.run_with(good(extra=1))
        self.assertEqual(rc, 1)
        self.assertEqual(calls, [])
        self.assertEqual([r["name"] for r in result["suites"] if r["fail"]], ["preflight"])
        self.assertNotIn("instruction_sha256", result)

    def test_a_stage_outside_the_plan_is_a_preflight_fail(self):
        rc, result, calls, _ = self.run_with(good(), ["--only", "windows_gate"])
        self.assertEqual((rc, calls), (1, []))
        self.assertEqual([r["name"] for r in result["suites"] if r["fail"]], ["preflight"])

    def test_env_run_has_no_instruction_hash(self):
        rc, result, calls = self._main_fixture()
        self.assertEqual(rc, 0)
        self.assertNotIn("instruction_sha256", result)

    def test_the_planned_names_equal_the_stages_main_builds(self):
        for doc in (good(), good(tier="large", windows=True, windows_runs=2, winapps="small", rehearsal=True)):
            with self.subTest(tier=doc["tier"]):
                seen = []
                real = self.m.execute_stages

                def recording(stages, soft, result, destination, runner=real.__defaults__[0]):
                    seen.append([x["name"] for x in stages])
                    return real(stages, soft, result, destination, runner)

                with patch.object(self.m, "execute_stages", recording):
                    rc, result, calls, _ = self.run_with(doc, soft_ms=900 * 60000)
                self.assertEqual(seen, [self.m.plan_stage_names(pi.from_dict(doc))])


class ReporterAcceptsProvenance(unittest.TestCase):
    def test_the_reporter_reads_the_new_keys_and_refuses_bad_ones(self):
        import vast_report

        base = {"verdict": "PASS", "suites": [{"name": "a", "pass": 1, "fail": 0, "skip": 0}]}
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "r.json"

            def read(extra):
                path.write_text(json.dumps({**base, **extra}))
                return vast_report.read_result_file(path)

            read({"instruction_sha256": "0" * 64, "selection": {"only": ["a"], "skip": []}})
            for extra in ({"instruction_sha256": "xyz"}, {"instruction_sha256": 3}, {"selection": []},
                          {"selection": {"only": [], "skip": [], "x": []}}, {"selection": {"only": "a", "skip": []}}):
                with self.subTest(extra=extra):
                    with self.assertRaises(vast_report.ResultError):
                        read(extra)


if __name__ == "__main__":
    unittest.main()
