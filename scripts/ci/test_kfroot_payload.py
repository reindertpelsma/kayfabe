#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of the rootless payload run: the audit as data (scripts/ci/kfroot.py STEPS and STAGE_POLICY), the static check, the
record, the orchestration (`Rootless`), the payload hooks (scripts/ci/vast_payload.py --rootless) and the two harness changes that go with
them (scripts/lib/kfroot.sh KF_KFROOT_RECORD, scripts/bench/host_xid_watch.sh kf_dmesg).

STATUS: LIVE, 2026-11-20. docs/design/V3_KFROOT_AUDIT.md, docs/CI_V3.md "Rootless payload run". No box, no GPU, no sudo, no root: every effect
of `Rootless` goes through an injected `run`, so these tests read the argv a box would run.

    KF_KFROOT_TREE=<dir>   aim the tests at a mutated tree (scripts/ci/kfroot_mutants.py)
"""

import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import types
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
TREE = Path(os.environ.get("KF_KFROOT_TREE") or ROOT)
CI = TREE / "scripts/ci"
sys.path.insert(0, str(CI))


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


K = load("kfroot", CI / "kfroot.py")
P = load("vast_payload", CI / "vast_payload.py")
R = load("vast_report", CI / "vast_report.py")
LIB = TREE / "scripts/lib/kfroot.sh"
XID = TREE / "scripts/bench/host_xid_watch.sh"
AUDIT = TREE / "docs/design/V3_KFROOT_AUDIT.md"


def planned_stage_names():
    """Every stage vast_payload.plan_stage_names can plan (read from its source, so a new stage cannot be forgotten here)."""
    src = (CI / "vast_payload.py").read_text()
    body = src[src.index("def plan_stage_names"):src.index("def select_stages")]
    return set(re.findall(r'names(?:\.append\(|\s*=\s*\[)\s*"([a-z_0-9]+)"', body)) | {"provision", "cuda_host", "merge_bar", "cuda_guest"}


class Result:
    def __init__(self, rc=0, out=""):
        self.returncode, self.stdout, self.stderr = rc, out, ""


class FakeRun:
    def __init__(self, fail=()):
        self.calls, self.fail = [], tuple(fail)

    def __call__(self, argv, **kw):
        self.calls.append((list(argv), kw))
        return Result(1 if any(f in " ".join(argv) for f in self.fail) else 0)

    def argvs(self):
        return [" ".join(a) for a, _ in self.calls]


def fake_bootstrap(user, workspace, chown=(), run=None, only_needed=False, memlock=True, install_files=True, **kw):
    fake_bootstrap.calls.append(dict(user=user, chown=list(chown), only_needed=only_needed, memlock=memlock, install_files=install_files))
    return [{"name": "kfroot_files", "rc": 0}, {"name": "nvidia_node_0", "rc": 0}, {"name": "chown_tree", "rc": 0}]


fake_bootstrap.calls = []


def make_rootless(tmp, **kw):
    tmp = Path(tmp)
    (tmp / "checkout").mkdir(exist_ok=True)
    (tmp / "ws/bench").mkdir(parents=True, exist_ok=True)
    (tmp / "ws/prov").mkdir(parents=True, exist_ok=True)
    (tmp / "home").mkdir(exist_ok=True)
    (tmp / "rootdir/.cargo").mkdir(parents=True, exist_ok=True)
    fake_bootstrap.calls = []
    run = kw.pop("run", FakeRun())
    root = kw.pop("root", None)
    params = dict(run=run, environ={"PATH": "/root/.cargo/bin:/usr/bin:/bin"}, workspace=str(tmp / "ws"), home=str(tmp / "home"),
                  geteuid=lambda: 0, bootstrap_fn=fake_bootstrap, toolchain_src=str(tmp / "rootdir"), prov_dir=str(tmp / "ws/prov"),
                  kfroot_py="/x/kfroot.py", python="py")
    params.update(kw)
    return K.Rootless("ci", root or tmp / "checkout", **params), run


class TempCase(unittest.TestCase):
    def setUp(self):
        self._d = tempfile.TemporaryDirectory()
        self.addCleanup(self._d.cleanup)
        self.tmp = Path(self._d.name)


class AuditData(unittest.TestCase):
    def test_every_step_is_complete_and_in_a_known_class(self):
        for name, step in K.STEPS.items():
            self.assertRegex(name, K.NAME_RE.pattern)
            self.assertIn(step["class"], K.CLASSES, name)
            for key in ("where", "reason", "equivalent"):
                self.assertTrue(step[key].strip(), f"{name}.{key}")

    def test_every_stage_the_payload_can_plan_is_audited_and_nothing_else_is(self):
        self.assertEqual(set(K.STAGE_POLICY), planned_stage_names())

    def test_every_stage_has_a_known_kind_and_status(self):
        for name, p in K.STAGE_POLICY.items():
            self.assertIn(p["as"], ("setup", "user", "self_drop", "root_stage"), name)
            self.assertIn(p["status"], ("probed", "not_yet_run", "not_applicable"), name)
            if p["as"] in ("root_stage", "self_drop"):
                self.assertTrue(p["blocker"], f"{name}: a stage that runs as root says why")
            if p["as"] == "user":
                self.assertTrue(p["scripts"], f"{name}: a dropped stage names the scripts the static check reads")

    def test_a_probed_stage_cites_evidence_that_exists(self):
        for name, p in K.STAGE_POLICY.items():
            if p["status"] == "probed":
                self.assertTrue((TREE / p["evidence"]).is_file(), f"{name}: {p['evidence']}")

    def test_only_provision_is_setup_and_it_is_a_registered_step(self):
        self.assertEqual([n for n, p in K.STAGE_POLICY.items() if p["as"] == "setup"], ["provision"])
        self.assertEqual(K.STEPS["provision"]["class"], "SETUP-ONCE")

    def test_a_root_stage_is_a_vm_run_class_step(self):
        self.assertEqual(K.STEPS["root_stage"]["class"], "VM-RUN")

    def test_every_bootstrap_step_name_is_registered(self):
        names = {s.name for s in K.plan("ci", only_needed=False)} | {"nvidia_node_0", "kfroot_files"}
        for n in names:
            base = re.sub(r"_[0-9]+$", "", n)
            self.assertIn(base, K.STEPS, n)

    def test_every_kfroot_call_site_of_the_harness_is_an_audited_step(self):
        sites = 0
        for rel in ("scripts/bench/box/windows_gate.sh", "scripts/bench/box/windows_apps.sh", "scripts/bench/windows/win_vm.sh", "scripts/lib/kfroot.sh"):
            for n, line in enumerate((TREE / rel).read_text().splitlines(), 1):
                code = line.split(" # ")[0].strip()
                m = re.search(r'(?:^|\|\s*|;\s*)(?:[A-Za-z_]+=\S+\s+)*kfroot\s+(.+)$', code)
                if not m or code.startswith("#"):
                    continue
                text = m.group(1).replace('"${apt_run[@]}"', "python3 -I vast_setup.py apt").replace('"$local"', "/etc/apparmor.d/local/usr.bin.swtpm")
                if rel.endswith("kfroot.sh") and "kfroot dmesg" not in code:
                    continue
                sites += 1
                self.assertIsNotNone(K.classify_command(text), f"{rel}:{n}: {line.strip()}")
        self.assertGreaterEqual(sites, 4)

    def test_the_audit_document_names_every_step_and_every_stage(self):
        text = AUDIT.read_text()
        for name in (*K.STEPS, *K.STAGE_POLICY):
            self.assertIn(f"`{name}`", text, name)

    def test_the_audit_document_is_dated_and_says_what_has_not_run(self):
        head = AUDIT.read_text()[:600]
        self.assertRegex(head, r"STATUS: LIVE as an audit, 2026-11-20")
        self.assertIn("NOT YET RUN", AUDIT.read_text())

    def test_the_cli_prints_the_audit_as_json(self):
        out = subprocess.run([sys.executable, str(CI / "kfroot.py"), "steps"], capture_output=True, text=True, check=True).stdout
        doc = json.loads(out)
        self.assertEqual(set(doc["steps"]), set(K.STEPS))
        self.assertEqual(set(doc["stages"]), set(K.STAGE_POLICY))


class StaticCheck(TempCase):
    def scan(self, text, python=False):
        return K.unmarked_root_ops(text, python)

    def test_a_kfroot_line_is_not_a_finding(self):
        self.assertEqual(self.scan("    kfroot apparmor_parser -r x\n    kf_dmesg | head\n"), [])

    def test_a_comment_is_not_a_finding(self):
        self.assertEqual(self.scan("# apt-get install x\n    # modprobe nbd\nx=1 # sudo ls\n"), [])

    def test_each_root_operation_is_found(self):
        for op in ("apt-get install x", "modprobe nbd", "mount -o ro /a /b", "umount /b", "chown -R a /x", "sysctl -w a=1", "dmesg", "systemctl stop a",
                   "apparmor_parser -r p", "nvidia-modprobe -c0", "sudo ls", "useradd x", "qemu-nbd --connect=/dev/nbd0 f"):
            for form in (f"{op}", f"x=$({op})", f"a && {op}", f"if true; then {op}; fi", f"f() {{ {op}; }}", f"A=1 {op}"):
                self.assertTrue(self.scan(form), form)

    def test_a_blanket_root_guard_is_found(self):
        self.assertEqual(self.scan('[ "$(id -u)" = 0 ] || abort not root\n'), [(1, "root_guard")])
        self.assertEqual(self.scan("if os.geteuid() != 0:\n", True), [(1, "root_guard")])

    def test_a_python_list_literal_is_found(self):
        self.assertEqual(self.scan('steps = [["apt-get", "install", "-y"]]\n', True), [(1, "apt-get")])

    def test_a_word_that_merely_contains_an_operation_is_not_a_finding(self):
        self.assertEqual(self.scan("echo mounted; cp amount.txt x; file=dmesg.log; grep sysctl_note f\n"), [])

    def test_the_real_dropped_stages_are_clean(self):
        for name, p in K.STAGE_POLICY.items():
            if p["as"] == "user":
                self.assertEqual(K.lint_stage(name, TREE), [], name)

    def test_a_planted_root_operation_in_a_dropped_stage_is_refused_by_name(self):
        for rel in ("scripts/bench/box/windows_gate.sh", "scripts/bench/cuda_ladder.sh"):
            dst = self.tmp / rel
            dst.parent.mkdir(parents=True, exist_ok=True)
            dst.write_text((TREE / rel).read_text() + "\napt-get install -y something\n")
        for rel in K.STAGE_POLICY["windows_gate"]["scripts"] + K.STAGE_POLICY["cuda_host"]["scripts"]:
            if not (self.tmp / rel).exists():
                (self.tmp / rel).parent.mkdir(parents=True, exist_ok=True)
                (self.tmp / rel).write_text((TREE / rel).read_text())
        tokens = K.lint_stage("windows_gate", self.tmp)
        self.assertEqual(len(tokens), 1)
        self.assertRegex(tokens[0], r"^unmarked_root_op:windows_gate:scripts/bench/box/windows_gate\.sh:\d+:apt-get$")
        self.assertRegex(K.lint_stage("cuda_host", self.tmp)[0], r"^unmarked_root_op:cuda_host:scripts/bench/cuda_ladder\.sh:\d+:apt-get$")

    def plant(self, stage, rel, extra):
        for r in K.STAGE_POLICY[stage]["scripts"]:
            dst = self.tmp / r
            dst.parent.mkdir(parents=True, exist_ok=True)
            dst.write_text((TREE / r).read_text() + (extra if r == rel else ""))
        return K.lint_stage(stage, self.tmp)

    def test_a_raw_dmesg_planted_in_the_xid_wrapper_is_refused_for_every_dropped_stage(self):
        for stage in ("cuda_host", "windows_gate"):
            tokens = self.plant(stage, "scripts/bench/host_xid_watch.sh", "\ndmesg | tail\n")
            self.assertEqual(len(tokens), 1, (stage, tokens))
            self.assertRegex(tokens[0], rf"^unmarked_root_op:{stage}:scripts/bench/host_xid_watch\.sh:\d+:dmesg$")

    def test_a_blanket_root_guard_planted_in_a_dropped_stage_is_refused(self):
        tokens = self.plant("windows_gate", "scripts/bench/box/windows_gate.sh", '\n[ "$(id -u)" = 0 ] || refuse not-root\n')
        self.assertEqual(len(tokens), 1, tokens)
        self.assertTrue(tokens[0].endswith(":root_guard"), tokens)

    def test_a_planted_kfroot_line_is_not_refused(self):
        self.assertEqual(self.plant("windows_gate", "scripts/bench/box/windows_gate.sh", "\nkfroot apt-get install x\n"), [])

    def test_a_missing_entry_script_is_refused_not_skipped(self):
        self.assertTrue(all(t.startswith("missing_script:cuda_host:") for t in K.lint_stage("cuda_host", self.tmp)))
        self.assertTrue(K.lint_stage("cuda_host", self.tmp))

    def test_a_root_stage_is_not_read_by_the_check(self):
        self.assertEqual(K.lint_stage("merge_bar", self.tmp), [])

    def test_the_xid_wrapper_reads_the_kernel_log_through_kf_dmesg(self):
        self.assertEqual(self.scan(XID.read_text()), [])
        self.assertIn("kf_dmesg", XID.read_text())

    def test_the_lint_command_exits_non_zero_on_a_finding(self):
        p = subprocess.run([sys.executable, str(CI / "kfroot.py"), "lint-stage", "cuda_host", "--root", str(self.tmp)], capture_output=True, text=True)
        self.assertEqual(p.returncode, 1)
        self.assertIn("KFROOT_LINT_RESULT stage=cuda_host as=user findings=", p.stdout)
        p = subprocess.run([sys.executable, str(CI / "kfroot.py"), "lint-stage", "cuda_host", "--root", str(TREE)], capture_output=True, text=True)
        self.assertEqual((p.returncode, p.stdout.strip()), (0, "KFROOT_LINT_RESULT stage=cuda_host as=user findings=0"))


class StockPatchAudit(unittest.TestCase):
    """The stock-patch audit follows every literal script name; naming the stage scripts in kfroot.py must not make the Windows entry scripts
    (which source scripts/lib/kfroot.sh) reach an opt-in applicator (CI job 'Release check fixtures, mutants and stock patch policy')."""

    def test_the_real_tree_passes_the_audit(self):
        p = subprocess.run([sys.executable, str(ROOT / "scripts/ci/no_patch_check.py")], capture_output=True, text=True, cwd=ROOT)
        self.assertEqual(p.returncode, 0, p.stdout[:600] + p.stderr[:600])
        self.assertNotIn("stock path reaches opt-in applicator", p.stdout + p.stderr)


class RootStageBlockersAreStillThere(unittest.TestCase):
    """A root stage cites its blocker. When the blocker is gone the stage can be converted: this test then fails, and the audit is re-done."""

    def text(self, rel):
        return (TREE / rel).read_text()

    def test_merge_bar(self):
        t = self.text("scripts/fastguest/build_fast_guest.sh")
        for needle in ("modprobe nbd", "qemu-nbd --read-only --connect=/dev/nbd0", 'mount -o ro /dev/nbd0p1 "$ROOT/mnt"'):
            self.assertIn(needle, t)
        self.assertIn("build_fast_guest.sh", self.text("scripts/bench/box/merge_check.sh"))

    def test_the_swtpm_build_stages(self):
        self.assertIn('["apt-get", "install", "-y", "--no-install-recommends", *SWTPM_DEPS]', self.text("scripts/ci/propimage_apps.py"))
        self.assertIn("import propimage_apps as pa", self.text("scripts/ci/propimage_apps_b_stage.py"))
        self.assertIn("import propimage_apps as pa", self.text("scripts/ci/matlab_stage.py"))

    def test_specviewperf(self):
        self.assertIn('[ "$(id -u)" = 0 ] || abort OPERATOR "not root"', self.text("scripts/bench/box/specviewperf_stage.sh"))

    def test_apps(self):
        t = self.text("scripts/apps/apps_matrix.sh")
        self.assertIn("mkdir -p /opt/apps", t)
        self.assertIn("as root", t)

    def test_install_flow(self):
        self.assertIn("if os.geteuid() != 0:", self.text("scripts/ci/install_flow.py"))


class Refusals(unittest.TestCase):
    def tokens(self, stages, approved=()):
        return K.stage_refusals(stages, TREE, approved)

    def test_a_stage_the_audit_does_not_know_is_refused(self):
        self.assertEqual(self.tokens(["cuda_host", "brand_new"]), ["unclassified:brand_new"])

    def test_a_root_stage_nobody_approved_is_refused(self):
        self.assertEqual(self.tokens(["merge_bar"]), ["root_stage_not_approved:merge_bar"])

    def test_an_approved_root_stage_is_accepted(self):
        self.assertEqual(self.tokens(["merge_bar"], ["merge_bar"]), [])

    def test_approving_a_user_stage_as_root_is_refused(self):
        self.assertEqual(self.tokens(["cuda_host"], ["cuda_host"]), ["approval_not_a_root_stage:cuda_host"])

    def test_approving_provision_or_an_unknown_name_is_refused(self):
        self.assertEqual(self.tokens(["provision"], ["provision"]), ["approval_not_a_root_stage:provision"])
        self.assertEqual(self.tokens(["provision"], ["nope"]), ["approval_not_a_root_stage:nope"])

    def test_the_default_gate_plan_needs_exactly_one_approval(self):
        self.assertEqual(self.tokens(["provision", "cuda_host", "merge_bar", "cuda_guest"]), ["root_stage_not_approved:merge_bar"])

    def test_the_windows_stages_need_no_approval(self):
        self.assertEqual(self.tokens(["provision", "cuda_host", "cuda_guest", "windows_gate", "windows_apps", "windows_gate_2"]), [])

    def test_every_planned_stage_is_known_to_the_refusals(self):
        for name in planned_stage_names():
            self.assertNotIn(f"unclassified:{name}", self.tokens([name], [name] if K.STAGE_POLICY[name]["as"] == "root_stage" else []), name)

    def test_the_preflight_names_an_unknown_stage(self):
        with tempfile.TemporaryDirectory() as d:
            rl, _ = make_rootless(d, root=TREE)
            self.assertEqual(rl.preflight(["brand_new"], []), ["unclassified:brand_new"])

    def test_every_non_user_non_setup_stage_is_refused_without_approval(self):
        for name, p in K.STAGE_POLICY.items():
            if p["as"] == "root_stage":
                self.assertEqual(self.tokens([name]), [f"root_stage_not_approved:{name}"])


class RecordTests(unittest.TestCase):
    def rec(self):
        return K.Record("ci", False)

    def test_an_empty_run_is_rootless(self):
        self.assertEqual(self.rec().verdict(), "ROOTLESS")

    def test_an_approved_root_stage_is_not_a_pure_rootless_run(self):
        r = self.rec()
        r.stage("merge_bar", "root_stage", "not_yet_run")
        self.assertEqual(r.verdict(), "ROOT_STAGES_APPROVED")

    def test_an_unaudited_step_violates(self):
        r = self.rec()
        r.step("made_up", "x", 0)
        self.assertEqual((r.verdict(), r.to_json()["steps"][0]["step"], r.to_json()["unaudited"]), ("VIOLATED", "unaudited", ["made_up"]))

    def test_a_refused_kfroot_call_in_a_dropped_stage_violates(self):
        r = self.rec()
        r.refusals("a\n[xid-watch] KFROOT_REFUSED dmesg: no\nb\n", "cuda_host")
        self.assertEqual(r.verdict(), "VIOLATED")
        self.assertEqual(r.to_json()["refused"][0]["stage"], "cuda_host")

    def test_used_lines_become_audited_steps_by_their_command(self):
        r = self.rec()
        r.used("KFROOT_USED\troot\tpython3 -I /x/vast_setup.py apt -o DPkg install -y ovmf\n"
               "KFROOT_USED\troot\ttee -a /etc/apparmor.d/local/usr.bin.swtpm\n"
               "KFROOT_USED\troot\tapparmor_parser -r /etc/apparmor.d/usr.bin.swtpm\n"
               "KFROOT_USED\troot\tdmesg\nunrelated\n", "windows_gate")
        self.assertEqual([s["step"] for s in r.steps], ["apt_prereqs", "swtpm_apparmor_rule", "swtpm_apparmor_load", "dmesg_via_kfroot"])
        self.assertEqual(r.verdict(), "ROOTLESS")

    def test_a_used_line_for_a_command_the_audit_does_not_know_violates(self):
        r = self.rec()
        r.used("KFROOT_USED\troot\trm -rf /\n", "windows_gate")
        self.assertEqual((r.verdict(), r.to_json()["unaudited"]), ("VIOLATED", ["rm -rf /"]))

    def test_a_malformed_used_line_violates(self):
        r = self.rec()
        r.used("KFROOT_USED\tonly-two\n", "s")
        self.assertEqual(r.verdict(), "VIOLATED")

    def test_a_refused_use_records_rc_126(self):
        r = self.rec()
        r.used("KFROOT_USED\trefused\tdmesg\n", "s")
        self.assertEqual(r.steps[0]["rc"], 126)

    def test_the_record_is_bounded_and_printable(self):
        r = self.rec()
        for i in range(200):
            r.step("provision", "s\x00" * 50, 0, detail="d\n" * 200)
        doc = r.to_json()
        self.assertEqual(len(doc["steps"]), K.Record.LIMIT)
        self.assertTrue(all(re.fullmatch(r"[ -~]{0,160}", s["detail"]) and re.fullmatch(r"[ -~]{0,160}", s["stage"]) for s in doc["steps"]))
        R.validate_rootless(doc)

    def test_a_record_of_five_hundred_steps_stays_inside_the_reporters_bound(self):
        r = self.rec()
        for i in range(500):
            r.step("chown_tree", "s" * 40, 0, detail="x" * 500)
        self.assertLess(len(json.dumps(r.to_json(), separators=(",", ":"))), 16384)
        R.validate_rootless(r.to_json())

    def test_control_characters_in_a_command_never_reach_the_reporter(self):
        r = self.rec()
        r.used("KFROOT_USED\troot\tdmesg \x01\x02\x7f\n", "s")
        self.assertEqual(r.verdict(), "ROOTLESS")
        R.validate_rootless(r.to_json())
        self.assertRegex(r.steps[0]["detail"], r"^[ -~]*$")

    def test_a_forged_clean_verdict_in_a_result_file_is_refused_by_the_reader(self):
        r = self.rec()
        r.refusals("KFROOT_REFUSED x", "s")
        doc = r.to_json()
        doc["verdict"] = "ROOTLESS"
        path = Path(tempfile.mkdtemp()) / "r.json"
        path.write_text(json.dumps({"verdict": "PASS", "suites": [{"name": "x", "pass": 1, "fail": 0, "skip": 0}], "rootless": doc}))
        with self.assertRaises(Exception):
            R.read_result_file(path)

    def test_the_reporter_accepts_what_the_record_writes(self):
        r = self.rec()
        r.stage("provision", "setup", "not_applicable")
        r.stage("windows_gate", "user", "not_yet_run")
        r.step("provision", "provision", 0)
        R.validate_rootless(r.to_json())

    def test_the_reporter_rejects_a_forged_clean_verdict(self):
        r = self.rec()
        r.refusals("KFROOT_REFUSED x", "s")
        doc = r.to_json()
        doc["verdict"] = "ROOTLESS"
        with self.assertRaises(ValueError):
            R.validate_rootless(doc)

    def test_the_reporter_rejects_a_violated_verdict_with_nothing_wrong(self):
        doc = self.rec().to_json()
        doc["verdict"] = "VIOLATED"
        with self.assertRaises(ValueError):
            R.validate_rootless(doc)

    def test_the_reporter_rejects_extra_keys_and_bad_names(self):
        doc = self.rec().to_json()
        with self.assertRaises(ValueError):
            R.validate_rootless({**doc, "extra": 1})
        doc["steps"] = [{"step": "Bad Name", "stage": "s", "rc": 0, "source": "payload", "detail": ""}]
        with self.assertRaises(ValueError):
            R.validate_rootless(doc)

    def test_a_result_file_with_a_rootless_record_is_read(self):
        r = self.rec()
        d = tempfile.mkdtemp()
        path = Path(d) / "r.json"
        path.write_text(json.dumps({"verdict": "PASS", "suites": [{"name": "x", "pass": 1, "fail": 0, "skip": 0}], "rootless": r.to_json()}))
        R.read_result_file(path)
        path.write_text(json.dumps({"verdict": "PASS", "suites": [{"name": "x", "pass": 1, "fail": 0, "skip": 0}], "rootless": {"schema": 1}}))
        with self.assertRaises(Exception):
            R.read_result_file(path)


class PlanOnlyNeeded(TempCase):
    def tree(self, **present):
        dev, proc = self.tmp / "dev", self.tmp / "proc"
        (proc / "driver/nvidia/gpus/0000:01:00.0").mkdir(parents=True)
        dev.mkdir()
        for name in ("kvm", "nvidiactl", "nvidia0", "nvidia-uvm", "nvidia-uvm-tools", "nvidia-modeset"):
            (dev / name).write_text("")
            os.chmod(dev / name, 0o666)
        (self.tmp / "restrict").write_text("0\n")
        (self.tmp / "lock").write_text("")
        os.chmod(self.tmp / "lock", 0o666)
        return dev, proc

    def plan(self, dev, proc, **kw):
        ws = self.tmp / "ws"
        ws.mkdir(exist_ok=True)
        return [s.name for s in K.plan("ci", str(ws), str(dev), str(proc), only_needed=True, lock=str(self.tmp / "lock"),
                                       dmesg_restrict=str(self.tmp / "restrict"), **kw)]

    def test_a_box_that_is_already_set_up_needs_only_the_account_and_the_audit_log(self):
        dev, proc = self.tree()
        names = self.plan(dev, proc, memlock=False)
        for gone in ("nvidia_node_0", "nvidia_uvm", "nvidia_modeset", "nvidia_node_perms", "dmesg_readable", "memlock_limit", "gpu_lock"):
            self.assertNotIn(gone, names)
        self.assertIn("user", names)
        self.assertIn("sudo_log", names)

    def test_a_readable_kernel_log_is_not_a_step(self):
        dev, proc = self.tree()
        self.assertNotIn("dmesg_readable", self.plan(dev, proc, memlock=False))

    def test_world_writable_nodes_need_no_chmod_step(self):
        dev, proc = self.tree()
        self.assertNotIn("nvidia_node_perms", self.plan(dev, proc, memlock=False))

    def test_a_good_lock_file_is_not_a_step(self):
        dev, proc = self.tree()
        self.assertNotIn("gpu_lock", self.plan(dev, proc, memlock=False))

    def test_the_needed_check_is_what_the_executed_bootstrap_runs(self):
        dev, proc = self.tree()
        calls, saved = [], K.os.geteuid
        K.os.geteuid = lambda: 0
        try:
            out = K.bootstrap("ci", workspace=str(self.tmp / "ws"), run=lambda argv, **kw: calls.append(argv) or Result(), root=str(self.tmp / "r"),
                              dev=str(dev), proc=str(proc), only_needed=True, memlock=False, install_files=False)
        finally:
            K.os.geteuid = saved
        names = [r["name"] for r in out]
        self.assertNotIn("kfroot_files", names)
        self.assertNotIn("nvidia_node_perms", names)
        self.assertNotIn("dmesg_readable", names)
        self.assertFalse([a for a in calls if a[:1] == ["chmod"] and any("nvidia" in x for x in a)])

    def test_a_restricted_kernel_log_is_a_step(self):
        dev, proc = self.tree()
        (self.tmp / "restrict").write_text("1\n")
        self.assertIn("dmesg_readable", self.plan(dev, proc, memlock=False))

    def test_a_missing_node_is_a_step_only_when_the_helper_exists(self):
        dev, proc = self.tree()
        (dev / "nvidia-uvm").unlink()
        names = self.plan(dev, proc, memlock=False)
        self.assertEqual("nvidia_uvm" in names, K.shutil.which("nvidia-modprobe") is not None)

    def test_a_node_that_is_not_world_writable_is_a_step(self):
        dev, proc = self.tree()
        os.chmod(dev / "nvidiactl", 0o660)
        self.assertIn("nvidia_node_perms", self.plan(dev, proc, memlock=False))

    def test_a_lock_file_that_is_not_0666_is_a_step_and_a_missing_one_too(self):
        dev, proc = self.tree()
        os.chmod(self.tmp / "lock", 0o644)
        self.assertIn("gpu_lock", self.plan(dev, proc, memlock=False))
        (self.tmp / "lock").unlink()
        self.assertIn("gpu_lock", self.plan(dev, proc, memlock=False))

    def test_memlock_is_a_step_only_when_asked_for(self):
        dev, proc = self.tree()
        self.assertNotIn("memlock_limit", self.plan(dev, proc, memlock=False))
        self.assertIn("memlock_limit", self.plan(dev, proc, memlock=True))

    def test_the_legacy_plan_is_unchanged_by_default(self):
        dev, proc = self.tree()
        legacy = [s.name for s in K.plan("ci", "/workspace", str(dev), str(proc))]
        for name in ("group_kvm", "dev_kvm_group", "nvidia_node_perms", "dmesg_readable", "memlock_limit", "workspace", "gpu_lock", "sudo_log"):
            self.assertIn(name, legacy)

    def test_the_cli_takes_the_two_options(self):
        p = subprocess.run([sys.executable, str(CI / "kfroot.py"), "bootstrap", "--dry-run", "--only-needed", "--no-memlock"], capture_output=True, text=True)
        self.assertEqual(p.returncode, 0, p.stderr)
        steps = json.loads(p.stdout.split("KFROOT_BOOTSTRAP ", 1)[1])["steps"]
        self.assertNotIn("memlock_limit", [s["name"] for s in steps])


class Orchestration(TempCase):
    def test_provision_runs_as_root_and_is_one_recorded_setup_step(self):
        rl, run = make_rootless(self.tmp)
        cmd = ["bash", "provision_full.sh"]
        self.assertEqual(rl.start("provision", cmd), cmd)
        self.assertEqual(run.calls, [])
        self.assertEqual(rl.record.steps[0]["step"], "provision")
        self.assertEqual(rl.record.stages["provision"], {"as": "setup", "status": "not_applicable"})

    def test_a_user_stage_starts_dropped_through_kfroot_run(self):
        rl, _ = make_rootless(self.tmp)
        argv = rl.start("cuda_host", ["bash", "cuda_ladder.sh", "host"])
        i = argv.index("kfroot.py") if "kfroot.py" in argv else argv.index("/x/kfroot.py")
        self.assertEqual(argv[i:i + 5], ["/x/kfroot.py", "run", "--user", "ci", "--no-memlock"])
        self.assertEqual(argv[-3:], ["bash", "cuda_ladder.sh", "host"])
        self.assertEqual(argv[argv.index("--") + 1:], ["bash", "cuda_ladder.sh", "host"])
        self.assertEqual(argv[0], "env")
        self.assertNotEqual(argv[:1], ["bash"])

    def test_every_user_stage_is_dropped_and_no_user_stage_runs_as_root(self):
        for name, p in K.STAGE_POLICY.items():
            if p["as"] != "user":
                continue
            with tempfile.TemporaryDirectory() as d:
                rl, _ = make_rootless(d)
                command = ["bash", "stage.sh", name]
                argv = rl.start(name, command)
                self.assertIn("/x/kfroot.py", argv, name)
                self.assertIn("run", argv, name)
                self.assertEqual(argv[argv.index("--") + 1:], command, name)

    def test_the_dropped_stage_gets_the_users_home_and_toolchain_and_prov_dir(self):
        rl, _ = make_rootless(self.tmp)
        argv = rl.start("cuda_host", ["true"])
        self.assertIn(f"HOME={self.tmp / 'home'}", argv)
        self.assertIn("USER=ci", argv)
        path = [a for a in argv if a.startswith("PATH=")][0]
        self.assertTrue(path.startswith(f"PATH={self.tmp / 'home'}/.cargo/bin:"))
        self.assertNotIn("/root/", path)
        self.assertIn(f"KF_PROV_DIR={self.tmp / 'ws/prov'}", argv)
        self.assertIn("CARGO_HOME", argv)  # unset: the user's own, not root's

    def test_memlock_is_not_raised_unless_asked(self):
        rl, _ = make_rootless(self.tmp)
        self.assertIn("--no-memlock", rl.start("cuda_host", ["true"]))
        rl, _ = make_rootless(self.tmp, memlock=True)
        self.assertNotIn("--no-memlock", rl.start("cuda_host", ["true"]))
        self.assertTrue(fake_bootstrap.calls[-1]["memlock"])

    def test_the_common_setup_runs_once_with_only_the_needed_steps_and_the_trees(self):
        rl, _ = make_rootless(self.tmp)
        rl.start("cuda_host", ["true"])
        rl.start("cuda_guest", ["true"])
        self.assertEqual(len(fake_bootstrap.calls), 1)
        call = fake_bootstrap.calls[0]
        self.assertTrue(call["only_needed"])
        self.assertIn(str(self.tmp / "checkout"), call["chown"])
        self.assertIn(str(self.tmp / "ws/bench"), call["chown"])
        self.assertIn(str(self.tmp / "ws/prov"), call["chown"])
        self.assertEqual([s["step"] for s in rl.record.steps if s["source"] == "bootstrap"], ["kfroot_files", "nvidia_node", "chown_tree"])

    def test_the_common_setup_does_not_run_for_provision_or_a_root_stage(self):
        rl, _ = make_rootless(self.tmp)
        rl.start("provision", ["true"])
        rl.start("merge_bar", ["true"])
        self.assertEqual(fake_bootstrap.calls, [])

    def test_the_toolchain_is_copied_to_the_users_home_once(self):
        rl, run = make_rootless(self.tmp)
        rl.start("cuda_host", ["true"])
        cps = [a for a in run.argvs() if a.startswith("cp -a")]
        self.assertEqual(cps, [f"cp -a {self.tmp / 'rootdir/.cargo'} {self.tmp / 'home/.cargo'}"])
        self.assertIn(f"chown -R ci:ci {self.tmp / 'home/.cargo'}", run.argvs())
        self.assertEqual([s["step"] for s in rl.record.steps if s["source"] == "payload"], ["toolchain_for_user", "toolchain_for_user"])

    def test_an_existing_user_toolchain_is_left_alone(self):
        rl, run = make_rootless(self.tmp)
        (self.tmp / "home/.cargo").mkdir()
        rl.start("cuda_host", ["true"])
        self.assertEqual([a for a in run.argvs() if a.startswith("cp -a")], [])

    def test_a_failing_toolchain_copy_refuses_the_stage_by_name(self):
        rl, _ = make_rootless(self.tmp, run=FakeRun(fail=["cp -a"]))
        with self.assertRaises(K.RootlessRefused) as e:
            rl.start("cuda_host", ["true"])
        self.assertEqual(str(e.exception), "step_failed:toolchain_for_user")

    def test_a_windows_stage_runs_the_prepare_as_root_before_the_drop_and_only_once(self):
        rl, run = make_rootless(self.tmp)
        rl.start("windows_gate", ["true"], tag="t")
        rl.start("windows_apps", ["true"], tag="t")
        prepares = [(a, kw) for a, kw in run.calls if "--prepare" in a]
        self.assertEqual(len(prepares), 1)
        argv, kw = prepares[0]
        self.assertEqual(argv[:3], ["bash", str(self.tmp / "checkout/scripts/bench/box/windows_gate.sh"), "--prepare"])
        self.assertEqual(kw["env"]["KF_ROOTLESS"], "off")
        self.assertEqual(kw["env"]["KF_KFROOT_RECORD"], str(self.tmp / "ws/prov/kfroot_used.tsv"))

    def test_a_non_windows_stage_does_not_run_the_prepare(self):
        rl, run = make_rootless(self.tmp)
        rl.start("cuda_host", ["true"])
        self.assertEqual([a for a, _ in run.calls if "--prepare" in a], [])

    def test_the_prepare_kfroot_lines_become_recorded_steps(self):
        rl, run = make_rootless(self.tmp)
        mark = self.tmp / "ws/prov/kfroot_used.tsv"

        def run_with_lines(argv, **kw):
            if "--prepare" in argv:
                mark.write_text("KFROOT_USED\troot\tpython3 -I x/vast_setup.py apt install\nKFROOT_USED\troot\ttee -a /etc/apparmor.d/local/usr.bin.swtpm\n")
            return Result()

        rl.run = run_with_lines
        rl.start("windows_gate", ["true"], tag="t")
        self.assertEqual([s["step"] for s in rl.record.steps if s["source"] == "script"], ["apt_prereqs", "swtpm_apparmor_rule"])

    def test_a_failing_prepare_refuses_the_stage_by_name(self):
        rl, _ = make_rootless(self.tmp, run=FakeRun(fail=["--prepare"]))
        with self.assertRaises(K.RootlessRefused) as e:
            rl.start("windows_gate", ["true"], tag="t")
        self.assertEqual(str(e.exception), "step_failed:windows_prepare")

    def test_an_approved_root_stage_runs_as_root_with_the_checkout_marked_safe_and_is_recorded(self):
        rl, _ = make_rootless(self.tmp)
        argv = rl.start("merge_bar", ["bash", "merge_check.sh"], approved=["merge_bar"])
        self.assertEqual(argv[-2:], ["bash", "merge_check.sh"])
        self.assertIn("GIT_CONFIG_KEY_0=safe.directory", argv)
        self.assertNotIn("/x/kfroot.py", argv)
        self.assertEqual([s["step"] for s in rl.record.steps], ["root_stage"])
        self.assertEqual(rl.record.stages["merge_bar"]["as"], "root_stage")
        self.assertEqual(rl.record.verdict(), "ROOT_STAGES_APPROVED")

    def test_the_trees_go_back_to_the_user_after_a_root_stage_once_the_user_exists(self):
        rl, run = make_rootless(self.tmp)
        rl.start("merge_bar", ["true"], approved=["merge_bar"])
        rl.finish("merge_bar")
        self.assertEqual([a for a in run.argvs() if a.startswith("chown")], [])
        rl.start("cuda_host", ["true"])
        rl.finish("merge_bar")
        chowns = [a for a in run.argvs() if a.startswith("chown -R ci:ci") and "checkout" in a]
        self.assertEqual(len(chowns), 1)

    def test_the_trees_are_listed_only_when_they_exist(self):
        rl, _ = make_rootless(self.tmp)
        self.assertNotIn(str(self.tmp / "ws/apps"), rl.trees())
        (self.tmp / "ws/apps").mkdir()
        self.assertIn(str(self.tmp / "ws/apps"), rl.trees())

    def test_a_self_dropping_stage_runs_as_root_and_gets_its_trees_back(self):
        rl, run = make_rootless(self.tmp)
        rl.start("cuda_host", ["true"])
        argv = rl.start("install_flow", ["python3", "install_flow.py"])
        self.assertNotIn("/x/kfroot.py", argv)
        rl.finish("install_flow")
        self.assertTrue([a for a in run.argvs() if a.startswith("chown -R ci:ci") and "ws/bench" in a])

    def test_a_dropped_stage_that_tried_kfroot_is_recorded_as_refused(self):
        rl, _ = make_rootless(self.tmp)
        rl.start("cuda_host", ["true"])
        rl.finish("cuda_host", "x\nKFROOT_REFUSED NoNewPrivs=1: 'kfroot ls'\n")
        self.assertEqual(rl.record.verdict(), "VIOLATED")

    def test_a_root_stage_output_is_not_read_for_refusals(self):
        rl, _ = make_rootless(self.tmp)
        rl.start("merge_bar", ["true"], approved=["merge_bar"])
        rl.finish("merge_bar", "KFROOT_REFUSED x")
        self.assertEqual(rl.record.refused, [])

    def test_the_payload_must_be_root(self):
        rl, _ = make_rootless(self.tmp, geteuid=lambda: 1000, root=TREE)
        self.assertEqual(rl.preflight(["cuda_host"], []), ["payload_not_root"])
        rl, _ = make_rootless(self.tmp, root=TREE)
        self.assertEqual(rl.preflight(["cuda_host"], []), [])

    def test_the_preflight_refuses_a_stage_that_needs_root_without_a_kfroot_line(self):
        rl, _ = make_rootless(self.tmp)
        for rel in K.STAGE_POLICY["cuda_host"]["scripts"]:
            dst = self.tmp / "checkout" / rel
            dst.parent.mkdir(parents=True, exist_ok=True)
            dst.write_text((TREE / rel).read_text() + "\nmodprobe nbd\n")
        tokens = rl.preflight(["cuda_host"], [])
        self.assertTrue(tokens and all(t.startswith("unmarked_root_op:cuda_host:") for t in tokens), tokens)

    def test_a_non_root_payload_is_refused_even_with_an_approved_root_stage(self):
        rl, _ = make_rootless(self.tmp, geteuid=lambda: 1000, root=TREE)
        self.assertIn("payload_not_root", rl.preflight(["cuda_host", "merge_bar"], ["merge_bar"]))

    def test_the_environment_lists_the_users_home_first_and_no_roots_path(self):
        rl, _ = make_rootless(self.tmp)
        env = rl.environment()
        self.assertEqual(env[0], f"HOME={self.tmp / 'home'}")
        self.assertTrue(all("/root/" not in e for e in env), env)

    def test_an_unknown_kfroot_command_in_the_prepare_log_violates(self):
        rl, _ = make_rootless(self.tmp)
        mark = self.tmp / "ws/prov/kfroot_used.tsv"

        def run_with_unknown(argv, **kw):
            if "--prepare" in argv:
                mark.write_text("KFROOT_USED\troot\trm -rf /srv\n")
            return Result()

        rl.run = run_with_unknown
        rl.start("windows_gate", ["true"], tag="t")
        self.assertEqual(rl.record.verdict(), "VIOLATED")

    def test_the_prepare_environment_reaches_the_script_that_writes_the_record(self):
        rl, _ = make_rootless(self.tmp)
        seen = {}

        def run_env(argv, **kw):
            if "--prepare" in argv:
                seen.update(kw["env"])
                Path(kw["env"]["KF_KFROOT_RECORD"]).write_text("KFROOT_USED\troot\tdmesg\n")
            return Result()

        rl.run = run_env
        rl.start("windows_gate", ["true"], tag="t")
        self.assertEqual(seen["KF_ROOTLESS"], "off")
        self.assertEqual([s["step"] for s in rl.record.steps if s["source"] == "script"], ["dmesg_via_kfroot"])

    def late(self):
        (self.tmp / "ws/prov").mkdir(parents=True, exist_ok=True)
        run = FakeRun()
        rl = K.Rootless("ci", self.tmp / "checkout", run=run, geteuid=lambda: 0, bootstrap_fn=fake_bootstrap, home=str(self.tmp / "home"),
                        workspace=str(self.tmp / "ws"), toolchain_src=str(self.tmp / "none"), prov_dir=str(self.tmp / "ws/prov"), kfroot_py="/x/kfroot.py")
        return rl, run

    def test_the_path_is_read_when_the_stage_starts_not_when_the_object_is_made(self):
        rl, _ = self.late()
        with patch.dict(os.environ, {"PATH": "/root/.cargo/bin:/usr/bin"}):
            path = [e for e in rl.environment() if e.startswith("PATH=")][0]
        self.assertEqual(path, f"PATH={self.tmp / 'home'}/.cargo/bin:/usr/bin")

    def test_the_prepare_sees_what_the_payload_exported_after_the_object_was_made(self):
        rl, run = self.late()
        with patch.dict(os.environ, {"BASH_ENV": "/x/hooks.sh", "KF_PAYLOAD_ROOT": "/x"}):
            rl.start("windows_gate", ["true"], tag="t")
        env = [kw["env"] for a, kw in run.calls if "--prepare" in a][0]
        self.assertEqual((env["BASH_ENV"], env["KF_PAYLOAD_ROOT"]), ("/x/hooks.sh", "/x"))

    def test_the_account_must_not_be_root(self):
        with self.assertRaises(K.KfrootError):
            K.Rootless("root", self.tmp)


class PayloadHooks(TempCase):
    def stages(self, names):
        return [dict(name=n, command=["bash", "stage", n], output=self.tmp / f"{n}.out", progress=[], detail=[],
                     grade=lambda rc, n=n: [P.row(n, passed=1)] if rc == 0 else (_ for _ in ()).throw(P.EvidenceError(n))) for n in names]

    def run_stages(self, names, rootless, approved=(), runner=None):
        calls = []

        def default_runner(command, soft, output, progress):
            calls.append(list(command))
            Path(output).write_text("")
            return 0

        result = {"verdict": "FAIL", "suites": [], "hardware": {"gpu_name": "RTX 3060", "compute_cap": 860, "driver_version": "580.159.04"},
                  "host_reference": "MISSING"}
        P.STAGE_RECORDS.clear()
        with contextlib.redirect_stdout(io.StringIO()):
            rc = P.execute_stages(self.stages(names), 2**62, result, self.tmp / "result.json", runner or default_runner,
                                  rootless=rootless, approved=approved, tag="t")
        return rc, result, calls

    def test_without_rootless_the_commands_and_the_result_are_unchanged(self):
        rc, result, calls = self.run_stages(["cuda_host"], None)
        self.assertEqual((rc, calls), (0, [["bash", "stage", "cuda_host"]]))
        self.assertNotIn("rootless", result)

    def test_with_rootless_the_runner_gets_the_dropped_command(self):
        rl, _ = make_rootless(self.tmp)
        rc, result, calls = self.run_stages(["provision", "cuda_host"], rl)
        self.assertEqual(rc, 0)
        self.assertEqual(calls[0], ["bash", "stage", "provision"])
        self.assertIn("/x/kfroot.py", calls[1])
        self.assertEqual(calls[1][-3:], ["bash", "stage", "cuda_host"])

    def test_the_dropped_command_carries_the_users_home_and_no_roots_toolchain(self):
        rl, _ = make_rootless(self.tmp)
        _, _, calls = self.run_stages(["cuda_host"], rl)
        self.assertIn(f"HOME={self.tmp / 'home'}", calls[0])
        path = [a for a in calls[0] if a.startswith("PATH=")][0]
        self.assertNotIn("/root/", path)

    def test_the_common_setup_runs_once_and_with_only_the_needed_steps_through_the_payload(self):
        rl, _ = make_rootless(self.tmp)
        self.run_stages(["cuda_host", "cuda_guest", "windows_gate"], rl)
        self.assertEqual(len(fake_bootstrap.calls), 1)
        self.assertTrue(fake_bootstrap.calls[0]["only_needed"])

    def test_the_payload_flow_asks_for_no_launcher_through_the_executor(self):
        rl, _ = make_rootless(self.tmp)
        self.run_stages(["cuda_host"], rl)
        self.assertIs(fake_bootstrap.calls[0]["install_files"], False)

    def test_a_bootstrap_that_fails_fails_the_stage_by_name_and_does_not_crash_the_run(self):
        def failing(*a, **kw):
            raise K.KfrootError("visudo_missing")

        rl, _ = make_rootless(self.tmp, bootstrap_fn=failing)
        rc, result, calls = self.run_stages(["cuda_host"], rl)
        self.assertEqual((rc, calls), (1, []))
        self.assertIn({"name": "rootless_setup", "pass": 0, "fail": 1, "skip": 0}, result["suites"])

    def test_a_failed_toolchain_copy_fails_the_stage_by_name(self):
        rl, _ = make_rootless(self.tmp, run=FakeRun(fail=["cp -a"]))
        rc, result, calls = self.run_stages(["cuda_host"], rl)
        self.assertEqual((rc, calls), (1, []))
        self.assertIn({"name": "rootless_setup", "pass": 0, "fail": 1, "skip": 0}, result["suites"])

    def test_a_refused_stage_is_recorded_as_not_run_with_its_reason(self):
        rl, _ = make_rootless(self.tmp, run=FakeRun(fail=["--prepare"]))
        self.run_stages(["windows_gate"], rl)
        record = P.STAGE_RECORDS[0]
        self.assertEqual((record["rc"], record["reason"]), (125, "rootless_setup"))

    def test_an_approved_root_stage_command_marks_the_checkout_safe_for_git(self):
        rl, _ = make_rootless(self.tmp)
        _, _, calls = self.run_stages(["merge_bar"], rl, approved=["merge_bar"])
        self.assertIn("GIT_CONFIG_KEY_0=safe.directory", calls[0])
        self.assertIn("GIT_CONFIG_VALUE_0=*", calls[0])

    def test_a_user_stage_never_reaches_the_runner_as_a_bare_command(self):
        rl, _ = make_rootless(self.tmp)
        _, _, calls = self.run_stages(["cuda_host", "cuda_guest", "windows_gate"], rl)
        for command in calls:
            self.assertNotEqual(command[:2], ["bash", "stage"])
            self.assertEqual(command[0], "env")

    def test_the_result_carries_the_record(self):
        rl, _ = make_rootless(self.tmp)
        rc, result, _ = self.run_stages(["provision", "cuda_host"], rl)
        doc = result["rootless"]
        R.validate_rootless(doc)
        self.assertEqual(doc["verdict"], "ROOTLESS")
        self.assertEqual(doc["stages"]["provision"]["as"], "setup")
        self.assertEqual(doc["stages"]["cuda_host"], {"as": "user", "status": "probed"})
        self.assertIn("provision", [s["step"] for s in doc["steps"]])
        saved = json.loads((self.tmp / "result.json").read_text())
        self.assertEqual(saved["rootless"]["verdict"], "ROOTLESS")
        self.assertEqual(R.read_result_file(self.tmp / "result.json")["rootless"]["verdict"], "ROOTLESS")

    def test_a_failed_root_step_fails_the_stage_by_name_and_starts_nothing(self):
        rl, _ = make_rootless(self.tmp, run=FakeRun(fail=["--prepare"]))
        rc, result, calls = self.run_stages(["windows_gate"], rl)
        self.assertEqual((rc, calls), (1, []))
        self.assertEqual(result["verdict"], "FAIL")
        self.assertIn({"name": "rootless_setup", "pass": 0, "fail": 1, "skip": 0}, result["suites"])

    def test_a_dropped_stage_that_asked_for_kfroot_makes_the_record_violated(self):
        rl, _ = make_rootless(self.tmp)

        def runner(command, soft, output, progress):
            Path(output).write_text("[xid-watch] KFROOT_REFUSED dmesg: this tree is NoNewPrivs\n")
            return 0

        rc, result, _ = self.run_stages(["cuda_host"], rl, runner=runner)
        self.assertEqual(result["rootless"]["verdict"], "VIOLATED")
        R.validate_rootless(result["rootless"])

    def test_an_approved_root_stage_runs_as_root_and_the_record_says_so(self):
        rl, _ = make_rootless(self.tmp)
        rc, result, calls = self.run_stages(["merge_bar"], rl, approved=["merge_bar"])
        self.assertEqual(result["rootless"]["verdict"], "ROOT_STAGES_APPROVED")
        self.assertNotIn("/x/kfroot.py", calls[0])
        self.assertIn("root_stage", [s["step"] for s in result["rootless"]["steps"]])

    def test_a_stage_too_short_of_time_is_not_started_and_not_a_rootless_event(self):
        rl, _ = make_rootless(self.tmp)
        stages = self.stages(["cuda_host"])
        stages[0].update(min_remaining_ms=10**15, short_row="cuda_host_retry_time")
        result = {"verdict": "FAIL", "suites": [], "hardware": {}, "host_reference": "MISSING"}
        P.STAGE_RECORDS.clear()
        with contextlib.redirect_stdout(io.StringIO()):
            P.execute_stages(stages, 0, result, self.tmp / "r.json", lambda *a: 0, rootless=rl, tag="t")
        self.assertEqual(fake_bootstrap.calls, [])


class Flags(unittest.TestCase):
    def setUp(self):
        quiet = contextlib.redirect_stdout(io.StringIO())
        quiet.__enter__()
        self.addCleanup(quiet.__exit__, None, None, None)

    def args(self, *argv):
        return P.parse_args(list(argv))

    def test_off_is_the_default_and_the_variable_alone_turns_it_on(self):
        self.assertIsNone(P.rootless_request(self.args(), {}))
        self.assertEqual(P.rootless_request(self.args(), {"KF_PAYLOAD_ROOTLESS": "require"}),
                         {"user": "ci", "approved": [], "memlock": False})

    def test_the_flag_beats_the_variable(self):
        self.assertIsNone(P.rootless_request(self.args("--rootless", "off"), {"KF_PAYLOAD_ROOTLESS": "require"}))
        self.assertIsNotNone(P.rootless_request(self.args("--rootless", "require"), {"KF_PAYLOAD_ROOTLESS": "off"}))

    def test_a_bad_mode_is_refused_never_read_as_off(self):
        with self.assertRaises(P.EvidenceError):
            P.rootless_request(self.args(), {"KF_PAYLOAD_ROOTLESS": "record"})

    def test_a_rootless_option_without_require_is_refused(self):
        for argv in (["--rootless-root-stage", "merge_bar"], ["--rootless-memlock"], ["--rootless-user", "x"]):
            with self.assertRaises(P.EvidenceError):
                P.rootless_request(self.args(*argv), {})

    def test_the_options_and_their_variables(self):
        r = P.rootless_request(self.args("--rootless", "require", "--rootless-user", "kf", "--rootless-root-stage", "merge_bar", "--rootless-memlock"),
                               {"KF_PAYLOAD_ROOTLESS_ROOT_STAGES": "apps,merge_bar"})
        self.assertEqual(r, {"user": "kf", "approved": ["merge_bar", "apps"], "memlock": True})
        r = P.rootless_request(self.args("--rootless", "require"), {"KF_PAYLOAD_ROOTLESS_MEMLOCK": "1"})
        self.assertTrue(r["memlock"])

    def test_root_as_the_account_is_refused(self):
        with self.assertRaises(P.EvidenceError):
            P.rootless_request(self.args("--rootless", "require", "--rootless-user", "root"), {})

    def test_the_kfml_prefetch_is_a_root_process_and_a_rootless_run_leaves_it_out(self):
        self.assertTrue(P.kfml_prefetch_wanted(["kfml"], None))
        self.assertFalse(P.kfml_prefetch_wanted(["kfml"], object()))
        self.assertFalse(P.kfml_prefetch_wanted(["cuda_host"], None))

    def test_the_rootless_variables_are_not_instruction_fields(self):
        for name in P.ROOTLESS_ENV:
            self.assertNotIn(name, P.INSTRUCTION_ENV)

    def test_the_worker_does_not_send_them(self):
        spec = {n for n, _, _ in R.SPEC}
        for name in P.ROOTLESS_ENV:
            self.assertNotIn(name, spec)


class DryRun(unittest.TestCase):
    def run_payload(self, *argv, env=None):
        e = {k: v for k, v in os.environ.items() if not k.startswith("KF_")}
        e.update(env or {})
        p = subprocess.run([sys.executable, str(CI / "vast_payload.py"), "--dry-run", *argv], capture_output=True, text=True, env=e, cwd=str(TREE))
        return p.returncode, p.stdout

    def test_the_default_gate_plan_is_refused_by_name_without_an_approval(self):
        rc, out = self.run_payload("--rootless", "require")
        self.assertEqual(rc, 1)
        self.assertIn("KF_PAYLOAD_ROOTLESS_REFUSED root_stage_not_approved:merge_bar", out)
        self.assertIn("KF_PAYLOAD_FAIL suite=preflight", out)

    def test_an_approval_makes_the_plan_pass_and_the_plan_line_names_it(self):
        rc, out = self.run_payload("--rootless", "require", "--rootless-root-stage", "merge_bar", env={"KF_MERGE_WINDOWS": "1"})
        self.assertEqual(rc, 0, out)
        self.assertIn("KF_PAYLOAD_ROOTLESS mode=require user=ci memlock=0 root_stages=merge_bar stages=provision:cuda_host:merge_bar:cuda_guest:windows_gate", out)

    def test_a_dry_run_without_the_flag_prints_no_rootless_line(self):
        rc, out = self.run_payload()
        self.assertEqual(rc, 0)
        self.assertNotIn("KF_PAYLOAD_ROOTLESS", out)

    def test_the_variable_works_like_the_flag(self):
        rc, out = self.run_payload(env={"KF_PAYLOAD_ROOTLESS": "require", "KF_PAYLOAD_ROOTLESS_ROOT_STAGES": "merge_bar"})
        self.assertEqual(rc, 0, out)
        self.assertIn("KF_PAYLOAD_ROOTLESS mode=require", out)

    def test_a_bad_mode_in_the_variable_is_refused_by_name(self):
        rc, out = self.run_payload(env={"KF_PAYLOAD_ROOTLESS": "bogus"})
        self.assertEqual(rc, 1)
        self.assertIn("KF_PAYLOAD_ROOTLESS_REFUSED bad_mode", out)

    def test_the_flag_beats_the_variable_in_the_plan(self):
        rc, out = self.run_payload("--rootless", "off", env={"KF_PAYLOAD_ROOTLESS": "require"})
        self.assertEqual(rc, 0, out)
        self.assertNotIn("KF_PAYLOAD_ROOTLESS", out)

    def test_a_rootless_option_alone_is_refused(self):
        rc, out = self.run_payload("--rootless-root-stage", "merge_bar")
        self.assertEqual(rc, 1)
        self.assertIn("KF_PAYLOAD_ROOTLESS_REFUSED rootless_option_without_require", out)

    def test_root_as_the_account_is_refused_in_the_plan(self):
        rc, out = self.run_payload("--rootless", "require", "--rootless-user", "root", "--rootless-root-stage", "merge_bar")
        self.assertEqual(rc, 1)
        self.assertIn("KF_PAYLOAD_ROOTLESS_REFUSED bad_user", out)

    def test_approving_a_user_stage_is_refused_in_the_plan(self):
        rc, out = self.run_payload("--rootless", "require", "--rootless-root-stage", "windows_gate", "--rootless-root-stage", "merge_bar", env={"KF_MERGE_WINDOWS": "1"})
        self.assertEqual(rc, 1)
        self.assertIn("KF_PAYLOAD_ROOTLESS_REFUSED approval_not_a_root_stage:windows_gate", out)

    def test_the_stage_selection_is_what_is_checked(self):
        rc, out = self.run_payload("--rootless", "require", "--only", "cuda_host")
        self.assertEqual(rc, 0, out)


class ShellLibrary(TempCase):
    def sh(self, script, env=None):
        e = {"PATH": os.environ["PATH"], "HOME": str(self.tmp)}
        e.update(env or {})
        return subprocess.run(["bash", "-c", f". {LIB}\n{script}"], capture_output=True, text=True, env=e)

    def test_a_kfroot_call_as_root_is_recorded_and_runs(self):
        rec = self.tmp / "rec.tsv"
        p = self.sh("kfroot echo hi there", {"KFROOT_EUID": "0", "KF_KFROOT_RECORD": str(rec)})
        self.assertEqual((p.returncode, p.stdout), (0, "hi there\n"))
        self.assertEqual(rec.read_text(), "KFROOT_USED\troot\techo hi there\n")

    def test_a_refused_call_in_a_dropped_tree_is_recorded_as_refused(self):
        rec, status = self.tmp / "rec.tsv", self.tmp / "status"
        status.write_text("Name:\tx\nNoNewPrivs:\t1\n")
        p = self.sh("kfroot dmesg", {"KFROOT_EUID": "1000", "KFROOT_SELF_STATUS": str(status), "KF_KFROOT_RECORD": str(rec)})
        self.assertEqual(p.returncode, 126)
        self.assertEqual(rec.read_text(), "KFROOT_USED\trefused\tdmesg\n")
        self.assertIn("KFROOT_REFUSED", p.stderr)

    def test_without_a_record_file_nothing_is_written_and_nothing_fails(self):
        p = self.sh("kfroot true", {"KFROOT_EUID": "0"})
        self.assertEqual(p.returncode, 0)
        self.assertEqual(list(self.tmp.iterdir()), [])

    def test_what_the_library_writes_the_record_reads(self):
        rec, status = self.tmp / "rec.tsv", self.tmp / "status"
        (self.tmp / "bin").mkdir()
        (self.tmp / "bin/dmesg").write_text("#!/bin/sh\necho x\n")
        (self.tmp / "bin/dmesg").chmod(0o755)
        path = f"{self.tmp / 'bin'}:{os.environ['PATH']}"
        self.sh("kfroot dmesg", {"KFROOT_EUID": "0", "KF_KFROOT_RECORD": str(rec), "PATH": path})
        status.write_text("NoNewPrivs:\t1\n")
        self.sh("kfroot dmesg", {"KFROOT_EUID": "1000", "KFROOT_SELF_STATUS": str(status), "KF_KFROOT_RECORD": str(rec), "PATH": path})
        record = K.Record("ci", False)
        record.used(rec.read_text(), "windows_gate")
        self.assertEqual([(s["step"], s["rc"]) for s in record.steps], [("dmesg_via_kfroot", 0), ("dmesg_via_kfroot", 126)])

    def test_an_unwritable_record_file_never_fails_the_step(self):
        p = self.sh("kfroot true", {"KFROOT_EUID": "0", "KF_KFROOT_RECORD": "/proc/nope/x"})
        self.assertEqual(p.returncode, 0)


class XidWatch(TempCase):
    def run_watch(self, euid=None, restrict="0", nnp=False, dmesg_lines=5):
        bin_dir = self.tmp / "bin"
        bin_dir.mkdir(exist_ok=True)
        fake = bin_dir / "dmesg"
        # like the real one: refused while kernel.dmesg_restrict=1 and the caller is not root
        fake.write_text('#!/bin/bash\nif [ "$(cat "$KFROOT_DMESG_RESTRICT")" = 1 ] && [ "${KFROOT_EUID:-0}" != 0 ]; then echo "dmesg: read kernel buffer failed: Operation not permitted" >&2; exit 1; fi\n'
                        + "".join(f"echo line{i}\n" for i in range(dmesg_lines)))
        fake.chmod(0o755)
        (self.tmp / "restrict").write_text(restrict + "\n")
        (self.tmp / "status").write_text("Name:\tx\nNoNewPrivs:\t" + ("1" if nnp else "0") + "\n")
        env = {"PATH": f"{bin_dir}:{os.environ['PATH']}", "XID_OUTDIR": str(self.tmp / "out"), "HOME": str(self.tmp),
               "KFROOT_DMESG_RESTRICT": str(self.tmp / "restrict"), "KFROOT_SELF_STATUS": str(self.tmp / "status")}
        if euid is not None:
            env["KFROOT_EUID"] = str(euid)
        return subprocess.run(["bash", str(XID), "tag", "--", "true"], capture_output=True, text=True, env=env)

    def test_as_root_it_is_the_watcher_it_always_was(self):
        p = self.run_watch(euid=0)
        self.assertEqual(p.returncode, 0, p.stdout)
        self.assertIn("host_dmesg_lines=5", p.stdout)

    def test_a_readable_log_in_a_dropped_tree_is_watched(self):
        p = self.run_watch(euid=1000, restrict="0", nnp=True)
        self.assertEqual(p.returncode, 0, p.stdout)
        self.assertIn("host_dmesg_lines=5", p.stdout)

    def test_a_restricted_log_in_a_dropped_tree_is_unwatched_never_clean_and_names_the_refusal(self):
        p = self.run_watch(euid=1000, restrict="1", nnp=True)
        self.assertEqual(p.returncode, 7, p.stdout)
        self.assertIn("UNWATCHED", p.stdout)
        self.assertIn("KFROOT_REFUSED dmesg", p.stdout)
        self.assertNotIn("HOST Xid CLEAN", p.stdout)

    def test_the_refusal_line_of_the_wrapper_is_what_the_record_reads(self):
        p = self.run_watch(euid=1000, restrict="1", nnp=True)
        record = K.Record("ci", False)
        record.refusals(p.stdout, "cuda_host")
        self.assertEqual(record.verdict(), "VIOLATED")

    def test_a_copy_of_the_script_alone_reads_the_log_as_before(self):
        lone = self.tmp / "lone2" / "scripts/bench"
        lone.mkdir(parents=True)
        (lone / "host_xid_watch.sh").write_text(XID.read_text())
        bin_dir = self.tmp / "bin2"
        bin_dir.mkdir()
        (bin_dir / "dmesg").write_text("#!/bin/bash\necho only-one-line\n")
        (bin_dir / "dmesg").chmod(0o755)
        p = subprocess.run(["bash", str(lone / "host_xid_watch.sh"), "t", "--", "true"], capture_output=True, text=True,
                           env={"PATH": f"{bin_dir}:{os.environ['PATH']}", "XID_OUTDIR": str(self.tmp / "o2"), "HOME": str(self.tmp)})
        self.assertEqual(p.returncode, 7)
        self.assertIn("rc=0 with 1 line(s)", p.stdout)

    def test_a_dmesg_on_the_path_is_honoured_by_a_non_root_caller_with_a_restricted_log(self):
        # the CI runner case (test_vast_native_framing): the log source is a `dmesg` earlier on PATH, whatever the caller's uid and the sysctl say
        bin_dir = self.tmp / "binx"
        bin_dir.mkdir()
        (bin_dir / "dmesg").write_text("#!/bin/bash\necho a\necho b\necho c\n")
        (bin_dir / "dmesg").chmod(0o755)
        (self.tmp / "restrict").write_text("1\n")
        (self.tmp / "status").write_text("NoNewPrivs:\t0\n")
        p = subprocess.run(["bash", str(XID), "tag", "--", "true"], capture_output=True, text=True,
                           env={"PATH": f"{bin_dir}:{os.environ['PATH']}", "XID_OUTDIR": str(self.tmp / "out"), "HOME": str(self.tmp),
                                "KFROOT_EUID": "1000", "KFROOT_DMESG_RESTRICT": str(self.tmp / "restrict"), "KFROOT_SELF_STATUS": str(self.tmp / "status")})
        self.assertEqual(p.returncode, 0, p.stdout)
        self.assertIn("host_dmesg_lines=3", p.stdout)

    def test_the_new_xid_is_seen_by_a_non_root_caller_with_a_restricted_log(self):
        bin_dir = self.tmp / "biny"
        bin_dir.mkdir()
        (bin_dir / "dmesg").write_text("#!/bin/bash\necho boot\necho ok\n[ -e " + str(self.tmp / "go") + " ] && echo 'NVRM: Xid (PCI:0000:01:00): 31, fault'\ntrue\n")
        (bin_dir / "dmesg").chmod(0o755)
        (self.tmp / "restrict").write_text("1\n")
        p = subprocess.run(["bash", str(XID), "tag", "--", "touch", str(self.tmp / "go")], capture_output=True, text=True,
                           env={"PATH": f"{bin_dir}:{os.environ['PATH']}", "XID_OUTDIR": str(self.tmp / "out"), "HOME": str(self.tmp),
                                "KFROOT_EUID": "1000", "KFROOT_DMESG_RESTRICT": str(self.tmp / "restrict")})
        self.assertEqual(p.returncode, 6, p.stdout)
        self.assertIn("NEW HOST Xid ACROSS THE COMMAND", p.stdout)

    def lone_copy(self, body, name):
        lone = self.tmp / name / "scripts/bench"
        lone.mkdir(parents=True)
        (lone / "host_xid_watch.sh").write_text(XID.read_text())
        bin_dir = self.tmp / (name + "bin")
        bin_dir.mkdir()
        (bin_dir / "dmesg").write_text(body)
        (bin_dir / "dmesg").chmod(0o755)
        return subprocess.run(["bash", str(lone / "host_xid_watch.sh"), "t", "--", "true"], capture_output=True, text=True,
                              env={"PATH": f"{bin_dir}:{os.environ['PATH']}", "XID_OUTDIR": str(self.tmp / (name + "o")), "HOME": str(self.tmp)})

    def test_a_copy_of_the_script_alone_with_an_unreadable_log_is_unwatched_by_the_failure_of_dmesg_itself(self):
        p = self.lone_copy("#!/bin/bash\necho denied >&2\nexit 1\n", "lone3")
        self.assertEqual(p.returncode, 7, p.stdout)
        self.assertIn("returned rc=1 with", p.stdout)

    def test_a_copy_of_the_script_alone_retries_the_plain_dmesg_as_its_fallback(self):
        flag = self.tmp / "flag"
        p = self.lone_copy(f'#!/bin/bash\nif [ ! -e {flag} ]; then touch {flag}; exit 1; fi\necho a\necho b\necho c\n', "lone4")
        self.assertEqual(p.returncode, 0, p.stdout)
        self.assertIn("host_dmesg_lines=3", p.stdout)

    def test_a_copy_of_the_script_alone_still_works(self):
        lone = self.tmp / "lone" / "scripts/bench"
        lone.mkdir(parents=True)
        (lone / "host_xid_watch.sh").write_text(XID.read_text())
        bin_dir = self.tmp / "bin"
        bin_dir.mkdir()
        (bin_dir / "dmesg").write_text("#!/bin/bash\necho a\necho b\necho c\n")
        (bin_dir / "dmesg").chmod(0o755)
        p = subprocess.run(["bash", str(lone / "host_xid_watch.sh"), "t", "--", "true"], capture_output=True, text=True,
                           env={"PATH": f"{bin_dir}:{os.environ['PATH']}", "XID_OUTDIR": str(self.tmp / "o"), "HOME": str(self.tmp)})
        self.assertEqual(p.returncode, 0, p.stdout)


class HeadTail(TempCase):
    def test_a_log_that_is_not_utf8_or_not_there_never_raises(self):
        (self.tmp / "bad").write_bytes(b"\xff\xfe KFROOT_REFUSED x\n")
        self.assertIn("KFROOT_REFUSED", P.head_tail_text(self.tmp / "bad"))
        self.assertEqual(P.head_tail_text(self.tmp / "missing"), "")

    def test_a_huge_log_keeps_its_start_and_its_end(self):
        with open(self.tmp / "big", "wb") as f:
            f.write(b"FIRST_LINE\n")
            f.write(b"x" * (6 * 1024 * 1024))
            f.write(b"\nKFROOT_REFUSED at the end\n")
        text = P.head_tail_text(self.tmp / "big")
        self.assertIn("FIRST_LINE", text)
        self.assertIn("KFROOT_REFUSED at the end", text)
        self.assertLess(len(text), 2 * 1024 * 1024)

    def test_a_stage_output_over_four_megabytes_does_not_stop_the_run(self):
        rl, _ = make_rootless(self.tmp)

        def runner(command, soft, output, progress):
            Path(output).write_bytes(b"\xff" * (5 * 1024 * 1024) + b"\nKFROOT_REFUSED dmesg\n")
            return 0

        stages = [dict(name="cuda_host", command=["x"], output=self.tmp / "o.out", progress=[], detail=[], grade=lambda rc: [P.row("cuda_host", passed=1)])]
        result = {"verdict": "FAIL", "suites": [], "hardware": {}, "host_reference": "MISSING"}
        P.STAGE_RECORDS.clear()
        with contextlib.redirect_stdout(io.StringIO()):
            rc = P.execute_stages(stages, 2**62, result, self.tmp / "r.json", runner, rootless=rl, tag="t")
        self.assertEqual(rc, 0)
        self.assertEqual(result["rootless"]["verdict"], "VIOLATED")


class BootstrapForThePayload(TempCase):
    def test_the_payload_flow_installs_no_launcher_and_no_sudoers_file(self):
        calls = []
        saved = K.os.geteuid
        K.os.geteuid = lambda: 0
        try:
            out = K.bootstrap("ci", workspace=str(self.tmp / "ws"), run=lambda argv, **kw: calls.append(argv) or Result(), root=str(self.tmp / "r"),
                              dev=str(self.tmp / "dev"), proc=str(self.tmp / "proc"), only_needed=True, memlock=False, install_files=False)
        finally:
            K.os.geteuid = saved
        names = [r["name"] for r in out]
        self.assertNotIn("kfroot_files", names)
        self.assertIn("sudo_log", names)
        self.assertFalse((self.tmp / "r/etc/sudoers.d/kfroot").exists())
        self.assertFalse((self.tmp / "r/usr/local/bin/kfroot").exists())

    def test_the_default_bootstrap_still_installs(self):
        saved, calls = K.os.geteuid, []
        K.os.geteuid = lambda: 0
        try:
            out = K.bootstrap("ci", workspace=str(self.tmp / "ws"), run=lambda argv, **kw: calls.append(argv) or Result(), root=str(self.tmp / "r"),
                              dev=str(self.tmp / "dev"), proc=str(self.tmp / "proc"), visudo="true", memlock=False)
        except K.KfrootError:
            out = [{"name": "kfroot_files"}]
        finally:
            K.os.geteuid = saved
        self.assertIn("kfroot_files", [r["name"] for r in out])

    def test_the_orchestration_asks_for_no_install(self):
        rl, _ = make_rootless(self.tmp)
        rl.start("cuda_host", ["true"])
        self.assertIs(fake_bootstrap.calls[0]["install_files"], False)

    def test_a_bootstrap_that_cannot_run_refuses_the_stage_by_name(self):
        def failing(*a, **kw):
            raise K.KfrootError("bootstrap_step_failed user rc=1")

        rl, _ = make_rootless(self.tmp, bootstrap_fn=failing)
        with self.assertRaises(K.RootlessRefused) as e:
            rl.start("cuda_host", ["true"])
        self.assertEqual(str(e.exception), "bootstrap:bootstrap_step_failed_user_rc=1")


class PayloadMain(TempCase):
    """main() end to end with the V7 fixture (a fake checkout and a fake box), execute_stages recorded and the root steps faked: nothing here
    touches the host, whatever a mutant does."""

    def fixture(self, env=None, windows="0", euid=0):
        import test_vast_payload_v7 as v7

        case = v7.V7Tests("test_hardware_converts_smi_units")
        case.setUp()
        captured, tmp = [], self.tmp

        class Safe(K.Rootless):
            def __init__(self, user, root, **kw):
                kw.update(run=FakeRun(), bootstrap_fn=fake_bootstrap, geteuid=lambda: euid, toolchain_src=str(tmp / "none"), home=str(tmp / "home"),
                          prov_dir=str(tmp / "prov"))
                super().__init__(user, root, **kw)

            def preflight(self, stages, approved):
                # the fixture's checkout holds no scripts: only that finding is set aside
                return [x for x in super().preflight(stages, approved) if not x.startswith("missing_script")]

        shim = types.SimpleNamespace(**{n: getattr(K, n) for n in dir(K) if not n.startswith("__")})
        shim.Rootless = Safe

        def recorder(stages, soft, result, destination, runner=None, **kw):
            captured.append(dict(stages=stages, kw=kw))
            case.m.persist_result(destination, result)
            return 0

        with (patch.object(case.m, "execute_stages", recorder), patch.object(case.m, "load_kfroot", return_value=shim),
              patch.object(case.m, "start_kfml_prefetch") as prefetch, contextlib.redirect_stdout(io.StringIO()) as out):
            rc, result, _ = case._main_fixture(windows=windows, extra_env=env)
        return rc, result, captured, prefetch, out.getvalue(), Safe

    def test_the_default_plan_is_refused_before_any_stage_when_a_root_stage_is_not_approved(self):
        rc, result, captured, _, out, _ = self.fixture({"KF_PAYLOAD_ROOTLESS": "require"})
        self.assertEqual((rc, captured), (1, []))
        self.assertIn("KF_PAYLOAD_ROOTLESS_REFUSED root_stage_not_approved:merge_bar", out)
        self.assertIn({"name": "preflight", "pass": 0, "fail": 1, "skip": 0}, result["suites"])

    def test_an_approved_plan_reaches_the_executor_with_the_rootless_object_and_the_approvals(self):
        rc, result, captured, _, out, Safe = self.fixture({"KF_PAYLOAD_ROOTLESS": "require", "KF_PAYLOAD_ROOTLESS_ROOT_STAGES": "merge_bar"})
        self.assertEqual(rc, 0, out)
        kw = captured[0]["kw"]
        self.assertIsInstance(kw["rootless"], Safe)
        self.assertEqual(kw["approved"], ["merge_bar"])
        self.assertEqual(kw["tag"], "ci" + "a" * 16)
        self.assertEqual([s["name"] for s in captured[0]["stages"]], ["provision", "cuda_host", "merge_bar", "cuda_guest"])
        self.assertIn("KF_PAYLOAD_ROOTLESS mode=require user=ci memlock=0 root_stages=merge_bar", out)

    def test_a_non_root_payload_is_refused_before_any_stage(self):
        rc, result, captured, _, out, _ = self.fixture({"KF_PAYLOAD_ROOTLESS": "require", "KF_PAYLOAD_ROOTLESS_ROOT_STAGES": "merge_bar"}, euid=1000)
        self.assertEqual((rc, captured), (1, []))
        self.assertIn("KF_PAYLOAD_ROOTLESS_REFUSED payload_not_root", out)

    def test_a_rootless_run_hands_the_later_stages_the_workspace_provision_directory(self):
        rc, _, captured, _, out, Safe = self.fixture({"KF_PAYLOAD_ROOTLESS": "require", "KF_PAYLOAD_ROOTLESS_ROOT_STAGES": "merge_bar"}, windows="1")
        self.assertEqual(rc, 0, out)
        gate = [s for s in captured[0]["stages"] if s["name"] == "windows_gate"][0]
        self.assertEqual(gate["command"][-1], "/workspace/prov")
        self.assertIsInstance(captured[0]["kw"]["rootless"], Safe)
        self.assertEqual(captured[0]["kw"]["approved"], ["merge_bar"])

    def test_the_windows_apps_stage_gets_the_workspace_provision_directory_too(self):
        rc, _, captured, _, out, _ = self.fixture({"KF_PAYLOAD_ROOTLESS": "require", "KF_PAYLOAD_ROOTLESS_ROOT_STAGES": "merge_bar", "KF_WINAPPS": "small"}, windows="1")
        self.assertEqual(rc, 0, out)
        apps = [s for s in captured[0]["stages"] if s["name"] == "windows_apps"][0]
        self.assertEqual(apps["command"][-1], "/workspace/prov")
        self.assertTrue(str(apps["detail"][0]).endswith("workspace/prov/" + "ci" + "a" * 16 + "_winapps.log"), apps["detail"])

    def test_without_rootless_the_provision_directory_is_roots_and_nothing_is_passed(self):
        rc, _, captured, _, out, _ = self.fixture(None, windows="1")
        self.assertEqual(rc, 0, out)
        gate = [s for s in captured[0]["stages"] if s["name"] == "windows_gate"][0]
        self.assertEqual(gate["command"][-1], "/root/prov")
        self.assertEqual(captured[0]["kw"], {})   # the call of every run before --rootless existed

    def test_a_rootless_run_leaves_the_root_prefetch_out_and_a_normal_run_starts_it(self):
        env = {"KF_TIER": "middle"}
        rc, _, captured, prefetch, out, _ = self.fixture({**env, "KF_PAYLOAD_ROOTLESS": "require", "KF_PAYLOAD_ROOTLESS_ROOT_STAGES": "merge_bar"})
        self.assertEqual(rc, 0, out)
        self.assertIn("kfml", [s["name"] for s in captured[0]["stages"]])
        prefetch.assert_not_called()
        rc, _, captured, prefetch, out, _ = self.fixture(env)
        self.assertEqual(rc, 0, out)
        prefetch.assert_called_once()


if __name__ == "__main__":
    unittest.main()
