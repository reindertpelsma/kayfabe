"""2026-11-15 (OWNER_RULINGS section AS): the documented commands are run from the docs themselves, GPU-free.

FALSIFIER (written before the code): a marker the extractor does not know is accepted or ignored; a marked block that is
missing, or a marker above something that is not a fence, is skipped without an error; a marker shown inside a fence is
run; a nested fence ends a block early; CRLF changes what runs; a block or a substitution reaches outside the run
prefix; a block that never reaches its last line passes; the real docs under docs/howto have a marker nobody plans, or a
planned step nobody marked; the stage cannot be selected with --only install_flow or is in a default tier; the result file
does not carry the doc hashes and the root steps.
"""

import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import doc_steps as ds
import install_flow as flow
import payload_instruction as pi
import test_vast_payload_v7 as v7

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
SHA = "a" * 40


def parse(text, doc="t.md"):
    return ds.parse(text.encode(), doc)


def block(marker, body="echo hi", fence="```sh"):
    return f"{marker}\n{fence}\n{body}\n```\n"


class Extractor(unittest.TestCase):
    def test_a_marked_block_is_extracted_with_its_options(self):
        p = parse(block("<!-- kf-run: build user=root timeout=90 -->", "make\nmake install"))
        (s,) = p.steps
        self.assertEqual((s.name, s.kind, s.user, s.timeout, s.text), ("build", "run", "root", 90, "make\nmake install\n"))
        self.assertEqual(p.fences, 1)
        self.assertEqual(p.sha256, hashlib.sha256(block("<!-- kf-run: build user=root timeout=90 -->", "make\nmake install").encode()).hexdigest())

    def test_defaults_and_order(self):
        p = parse(block("<!-- kf-run: one -->") + "text\n" + block("<!-- kf-args: two -->", "-m 1", "```"))
        self.assertEqual([(s.name, s.kind, s.user, s.timeout) for s in p.steps], [("one", "run", "unpriv", 3600), ("two", "args", "unpriv", 3600)])

    def test_an_unmarked_fence_is_counted_not_run(self):
        p = parse("```sh\nrm -rf /\n```\n")
        self.assertEqual((p.steps, p.fences), ((), 1))

    def refused(self, text):
        with self.assertRaises(ds.DocStepError) as caught:
            parse(text)
        self.assertNotIn("rm -rf", str(caught.exception))
        return str(caught.exception)

    def test_unknown_marker_kinds_and_keys_are_refused(self):
        for marker in (
            "<!-- kf-exec: a -->", "<!-- kf-run: A -->", "<!-- kf-run: a b -->", "<!-- kf-run: a user=admin -->",
            "<!-- kf-run: a color=red -->", "<!-- kf-args: a user=root -->", "<!-- kf-run: a user=root user=root -->",
            "<!-- kf-run: a timeout=0 -->", "<!-- kf-run: a timeout=14401 -->", "<!-- kf-run: a timeout=-5 -->",
            "<!--kf-run: a -->", "<!-- kf-run: ../a -->", "<!-- kf-run: a -- -->", "<!-- kf-run: a --> trailing",
            " <!-- kf-run: ../x -->", "<!-- kf- -->",
        ):
            with self.subTest(marker=marker):
                self.refused(block(marker, "rm -rf /"))

    def test_a_marker_with_no_block_is_refused(self):
        self.assertIn(":1:", self.refused("<!-- kf-run: a -->\n"))
        self.refused("<!-- kf-run: a -->\n\n```sh\necho\n```\n")  # a blank line is not adjacency
        self.refused("<!-- kf-run: a -->\ntext\n```sh\necho\n```\n")
        self.refused("<!-- kf-run: a -->\n<!-- kf-run: b -->\n```sh\necho\n```\n")

    def test_a_marker_above_an_indented_or_tilde_fence_is_refused(self):
        self.refused("<!-- kf-run: a -->\n   ```sh\necho\n   ```\n")
        self.refused("<!-- kf-run: a -->\n~~~sh\necho\n~~~\n")
        self.refused("<!-- kf-run: a -->\n```sh `x`\necho\n```\n")

    def test_duplicate_names_and_empty_blocks_and_unterminated_fences_are_refused(self):
        self.refused(block("<!-- kf-run: a -->") + block("<!-- kf-run: a -->"))
        self.refused(block("<!-- kf-run: a -->", ""))
        self.refused("<!-- kf-run: a -->\n```sh\necho hi\n")
        self.refused("```sh\necho hi\n")

    def test_sudo_needs_a_root_marker(self):
        self.refused(block("<!-- kf-run: a -->", "sudo apt-get install -y x"))
        self.refused(block("<!-- kf-run: a -->", "echo; sudo true"))
        self.assertEqual(parse(block("<!-- kf-run: a user=root -->", "sudo apt-get install -y x")).steps[0].user, "root")
        parse(block("<!-- kf-run: a -->", "echo sudoku"))  # a word that merely contains it

    def test_a_marker_shown_inside_a_fence_is_text(self):
        p = parse("````md\n<!-- kf-run: shown -->\n```sh\nrm -rf /\n```\n````\n")
        self.assertEqual((p.steps, p.fences), ((), 1))
        p = parse("```text\n<!-- kf-exec: nonsense -->\n```\n")
        self.assertEqual(p.steps, ())

    def test_nested_fences_do_not_end_a_marked_block_early(self):
        # a ~~~ line inside a ``` block is content; the first ``` closes it and "after" is prose
        (s,) = parse("<!-- kf-run: outer -->\n```sh\ncat <<'X'\n~~~\nX\n```\nafter\n").steps
        self.assertEqual(s.text, "cat <<'X'\n~~~\nX\n")
        # a longer fence is not closed by a shorter one, and the closer must have no info string
        q = parse("````sh\nA\n```\n```sh\nB\n```\nC\n````\nprose\n")
        self.assertEqual((q.steps, q.fences), ((), 1))

    def test_a_marked_four_backtick_fence_keeps_inner_three_backtick_lines_as_content(self):
        p = parse("<!-- kf-run: a -->\n````sh\necho hi\n```\nstill inside\n````\n")
        self.assertEqual(p.steps[0].text, "echo hi\n```\nstill inside\n")

    def test_crlf_reads_the_same_and_a_bare_cr_or_nul_is_refused(self):
        lf = block("<!-- kf-run: a -->", "echo one\necho two")
        crlf = lf.replace("\n", "\r\n")
        a, b = parse(lf), parse(crlf)
        self.assertEqual(a.steps, b.steps)
        self.assertNotEqual(a.sha256, b.sha256)  # the hash is of the bytes read
        self.refused(lf.replace("echo one", "echo\rone"))
        self.refused(lf.replace("echo one", "echo \0one"))
        with self.assertRaises(ds.DocStepError):
            ds.parse(b"\xff\xfe", "t.md")
        with self.assertRaises(ds.DocStepError):
            ds.parse(b"x" * (ds.MAX_DOC_BYTES + 1), "t.md")

    def test_args_words_follow_shell_quoting_and_drop_comments(self):
        p = parse(block("<!-- kf-args: a -->", "-object x,id=r\n-name 'two words'  # why\n", "```"))
        self.assertEqual(ds.args_words(p.steps[0]), ["-object", "x,id=r", "-name", "two words"])


class Substitution(unittest.TestCase):
    def step(self, body="git clone https://example.org/r.git\ncd r"):
        return parse(block("<!-- kf-run: clone -->", body)).steps[0]

    def test_exactly_once(self):
        with tempfile.TemporaryDirectory() as d:
            s, used = ds.substitute(self.step(), [ds.Substitution("clone", "https://example.org/r.git", "file://" + d + "/m.git")], d)
            self.assertIn("file://" + d + "/m.git", s.text)
            self.assertEqual(used, ["https://example.org/r.git"])
            for literal in ("https://nowhere/", "", "git"):  # absent, empty, absent-or-twice
                with self.subTest(literal=literal):
                    with self.assertRaises(ds.DocStepError):
                        ds.substitute(self.step("git git"), [ds.Substitution("clone", literal, "x")], d)
            # a substitution for another step does not apply
            same, used = ds.substitute(self.step(), [ds.Substitution("other", "git", "x")], d)
            self.assertEqual((same.text, used), (self.step().text, []))

    def test_a_value_that_is_not_one_clean_line_or_names_a_path_outside_the_prefix_is_refused(self):
        with tempfile.TemporaryDirectory() as d, tempfile.TemporaryDirectory() as other:
            link = Path(d) / "link"
            link.symlink_to(other)
            for value in ("a\nb", "a\0b", "", "file:///etc/passwd", "/etc/x", d + "/../x", str(link) + "/x", "file://" + other + "/m"):
                with self.subTest(value=value):
                    with self.assertRaises(ds.DocStepError):
                        ds.substitute(self.step(), [ds.Substitution("clone", "https://example.org/r.git", value)], d)
            ds.substitute(self.step(), [ds.Substitution("clone", "https://example.org/r.git", "file://" + d + "/ok.git")], d)

    def test_inside_follows_symlinks(self):
        with tempfile.TemporaryDirectory() as d, tempfile.TemporaryDirectory() as other:
            (Path(d) / "l").symlink_to(other)
            self.assertTrue(ds.inside(d, d + "/a/b"))
            self.assertFalse(ds.inside(d, d + "/l/x"))
            self.assertFalse(ds.inside(d, d + "-sibling/x"))
            self.assertFalse(ds.inside(d, "/"))


class Running(unittest.TestCase):
    def runner(self, **kw):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.lines = []
        return ds.Runner(self.tmp.name, log=self.lines.append, **kw)

    def run_block(self, runner, body, user="unpriv", timeout=30, name="s"):
        s = parse(block(f"<!-- kf-run: {name} user={user} timeout={timeout} -->", body)).steps[0]
        return runner.run(s)

    def test_cwd_and_path_carry_but_variables_do_not(self):
        r = self.runner()
        self.run_block(r, "mkdir proj\ncd proj\nPATH=/opt/x:$PATH\nexport FOO=bar", name="a")
        self.run_block(r, 'test "$(basename "$PWD")" = proj\ncase "$PATH" in /opt/x:*) ;; *) exit 9;; esac\ntest -z "${FOO:-}"', name="b")
        self.assertEqual([x["rc"] for x in r.records], [0, 0])
        self.assertEqual(self.lines[0], "KF_DOCSTEP_START doc=t.md step=a user=unpriv timeout=30")
        self.assertRegex(self.lines[1], r"^KF_DOCSTEP_END doc=t.md step=a rc=0 ms=\d+$")

    def test_the_callers_environment_is_not_inherited(self):
        r = self.runner()
        with patch.dict(os.environ, {"KF_SECRET_TOKEN": "x", "BASH_ENV": "/nonexistent", "CARGO_TARGET_DIR": "/elsewhere"}):
            self.run_block(r, 'test -z "${KF_SECRET_TOKEN:-}${BASH_ENV:-}${CARGO_TARGET_DIR:-}"\ncase "$HOME" in ' + self.tmp.name + '/*) ;; *) exit 9;; esac')
        self.assertEqual(r.records[0]["rc"], 0)

    def test_a_failing_block_stops_with_its_rc_and_set_e_pipefail_apply(self):
        for body, rc in (("false\necho not reached", 1), ("exit 7", 7), ("false | true", 1), ("echo $UNSET_VAR", 1)):
            with self.subTest(body=body):
                r = self.runner()
                with self.assertRaises(ds.DocStepError):
                    self.run_block(r, body)
                self.assertEqual(r.records[0]["rc"], rc)

    def test_a_block_that_does_not_reach_its_end_fails_even_with_rc_zero(self):
        for body in ("exec true", "exec >/dev/null\nexit 0", "cat <<'X'\nnever closed"):
            with self.subTest(body=body):
                r = self.runner()
                with self.assertRaises(ds.DocStepError):
                    self.run_block(r, body)
                self.assertEqual(r.records[0]["rc"], 125)
                self.assertIn("did not reach", r.records[0]["reason"])

    def test_a_block_that_leaves_the_prefix_is_refused_and_does_not_carry_its_cwd(self):
        r = self.runner()
        with self.assertRaises(ds.DocStepError):
            self.run_block(r, "cd /")
        self.assertEqual(r.records[0]["rc"], 126)
        self.assertEqual((r.state / "cwd").read_text(), str(r.work))
        r2 = self.runner()
        with tempfile.TemporaryDirectory() as outside:
            body = f"ln -s {outside} esc\ncd esc"
            with self.assertRaises(ds.DocStepError):
                self.run_block(r2, body)
            self.assertEqual(r2.records[0]["rc"], 126)

    def test_a_timeout_is_124(self):
        r = self.runner()
        with self.assertRaises(ds.DocStepError):
            self.run_block(r, "sleep 20", timeout=1)
        self.assertEqual(r.records[0]["rc"], 124)

    def test_a_root_step_gets_a_sudo_that_changes_nothing_and_is_recorded(self):
        r = self.runner()
        with patch.dict(os.environ, {"PATH": "/nonexistent"}):
            self.run_block(r, "sudo echo hi", user="root")
        self.assertEqual(r.root_steps(), ["s"])
        self.assertIn("hi", (r.logs / "01-s.log").read_text())

    def test_an_unprivileged_step_may_not_hold_root_or_capabilities_when_that_is_required(self):
        r = self.runner(require_unpriv=True)
        if os.geteuid() == 0:
            with self.assertRaises(ds.DocStepError):
                self.run_block(r, "true")
            self.assertEqual(r.records[0]["rc"], 127)
        else:
            self.run_block(r, "true")
            self.assertGreater(r.records[0]["uid"], 0)
            self.assertEqual(set(r.records[0]["cap_eff"]), {"0"})

    def test_the_drop_prefix_is_applied_to_unprivileged_steps_only(self):
        r = self.runner(drop=["/usr/bin/env", "KF_DROPPED=1"])
        self.run_block(r, 'test "${KF_DROPPED:-}" = 1', name="u")
        # env replaced the environment? `env A=1 cmd` adds to it, so the dropped step sees it
        r2 = self.runner(drop=["/usr/bin/env", "KF_DROPPED=1"])
        self.run_block(r2, 'test -z "${KF_DROPPED:-}"', user="root", name="r")
        self.assertEqual([x["rc"] for x in r.records + r2.records], [0, 0])

    def test_an_unsafe_prefix_is_refused(self):
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(ds.DocStepError):
                ds.Runner(Path(d) / "a b")

    def test_a_non_run_step_cannot_be_run(self):
        r = self.runner()
        with self.assertRaises(ds.DocStepError):
            r.run(parse(block("<!-- kf-args: a -->", "-m 1", "```")).steps[0])


class RealDocs(unittest.TestCase):
    def docs(self):
        return flow.load_docs(ROOT)

    def test_the_documented_steps_are_exactly_the_planned_ones(self):
        docs = self.docs()
        flow.check_plan(docs)
        got = [(d, s.name) for d, p in docs.items() for s in p.steps]
        self.assertEqual(sorted(got), sorted((d, s) for k, d, s in flow.PLAN if k == "doc"))
        self.assertEqual(
            [s.name for s in docs["building.md"].steps],
            ["clone", "packages_list", "packages_install", "rust_toolchain", "workspace_build", "qemu_build", "raw_client"],
        )
        self.assertEqual([s.name for s in docs["attach-to-existing-vm.md"].steps], ["qemu_args"])

    def test_the_root_steps_of_the_docs_are_exactly_one(self):
        steps = [s for p in self.docs().values() for s in p.steps if s.user == "root"]
        self.assertEqual([s.name for s in steps], ["packages_install"])

    def test_the_plan_order_follows_each_documents_own_order(self):
        docs = self.docs()
        for name, parsed in docs.items():
            planned = [s for k, d, s in flow.PLAN if k == "doc" and d == name]
            self.assertEqual(planned, [s.name for s in parsed.steps], name)

    def test_a_checkout_with_absent_objects_is_not_cloned_from(self):
        self.assertTrue(flow.has_missing_objects("aaaa\n?bbbb\ncccc path\n"))
        self.assertFalse(flow.has_missing_objects("aaaa\ncccc path\n"))
        with tempfile.TemporaryDirectory() as d:
            env = {**os.environ, "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t", "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@t"}
            run = lambda *a: subprocess.run(["git", "-C", d, *a], env=env, check=True, capture_output=True, text=True).stdout.strip()
            run("init", "-q")
            Path(d, "f").write_text("x\n")
            run("add", "f")
            run("commit", "-q", "-m", "c")
            sha = run("rev-parse", "HEAD")
            self.assertEqual(flow.mirror_source(d, sha), d)
            blob = run("rev-parse", "HEAD:f")
            os.remove(os.path.join(d, ".git/objects", blob[:2], blob[2:]))
            self.assertEqual(flow.mirror_source(d, sha), flow.REPO_URL)

    def test_the_mirror_clones_into_the_directory_the_docs_cd_into(self):
        with tempfile.TemporaryDirectory() as d:
            env = {**os.environ, "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t", "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@t"}
            src, prefix = Path(d, "src"), Path(d, "prefix")
            src.mkdir()
            prefix.mkdir()
            run = lambda *a: subprocess.run(["git", "-C", str(src), *a], env=env, check=True, capture_output=True, text=True).stdout.strip()
            run("init", "-q")
            (src / "f").write_text("x\n")
            run("add", "f")
            run("commit", "-q", "-m", "c")
            mirror = flow.make_mirror(src, prefix, run("rev-parse", "HEAD"), 1, 1)
            out = subprocess.run(["git", "clone", "-q", "file://" + str(mirror)], cwd=d, capture_output=True, text=True, env=env)
            self.assertEqual(out.returncode, 0, out.stderr)
            self.assertTrue(Path(d, "kayfabe", "f").exists(), "the documented `cd kayfabe` must find the clone")

    def test_the_cup8_log_lands_beside_the_step_logs(self):
        with tempfile.TemporaryDirectory() as d:
            path = flow.cup8_log(d)
            path.write_text("x")
            self.assertEqual(path, Path(d, "run", "logs", "cup8.out"))

    def test_the_documented_substitutions_apply_to_the_real_blocks(self):
        docs = self.docs()
        with tempfile.TemporaryDirectory() as d:
            clone = next(s for s in docs["building.md"].steps if s.name == "clone")
            s, used = ds.substitute(clone, [ds.Substitution("clone", flow.REPO_URL, "file://" + d + "/kayfabe.git")], d)
            self.assertIn("git clone file://" + d + "/kayfabe.git\n", s.text)
            args = next(x for x in docs["attach-to-existing-vm.md"].steps)
            subs = [ds.Substitution("qemu_args", flow.UUID_PLACEHOLDER, flow.UUID), ds.Substitution("qemu_args", flow.FB_PLACEHOLDER, "fb-mb=6144")]
            replaced, _ = ds.substitute(args, subs, d)
            words = ds.args_words(replaced)
            self.assertIn(flow.UUID, words)
            self.assertIn("kf3-gpu,id=kf0,fb-mb=6144,bar1-size=134217728,bar2-size=33554432", words)
            self.assertNotIn("<", " ".join(words))

    def test_the_real_blocks_run_through_the_runner_when_the_commands_are_stubbed(self):
        """The qemu_build block as written, against stub python3/tar on PATH: the runner's own plumbing, GPU-free.

        The block fetches the tarball with `python3 -I scripts/ci/qemu_source.py fetch .` (PR #422), which is not reachable
        from the runner's temp home and would hit the network, so python3 is stubbed. The stub accepts exactly one call, the
        verifying fetch with no flag (no --from-file, nothing that skips the pin check), and refuses every other argument
        list with rc 2; the real helper must exist at the path the block names. tar refuses to run unless the fetch has
        put the pinned file in place first, so the order is checked too.
        """
        import qemu_source

        docs = self.docs()
        name = qemu_source.read_pin()[0]
        self.assertTrue((ROOT / "scripts/ci/qemu_source.py").is_file())
        with tempfile.TemporaryDirectory() as d:
            stubs = Path(d) / "stubs"
            stubs.mkdir()
            (stubs / "python3").write_text(
                "#!/bin/sh\n"
                'echo "STUB python3 $*" >> "$HOME/calls"\n'
                'if [ "$#" -ne 4 ] || [ "$1" != -I ] || [ "$2" != scripts/ci/qemu_source.py ] || [ "$3" != fetch ] || [ "$4" != . ]; then\n'
                '  echo "stub python3: refused: $*" >&2; exit 2\n'
                "fi\n"
                ': > "./' + name + '"\n'
                'echo "./' + name + '"\n'
            )
            (stubs / "tar").write_text(
                "#!/bin/sh\n"
                'echo "STUB tar $*" >> "$HOME/calls"\n'
                '[ "$1" = -xf ] && [ -f "$2" ] || { echo "stub tar: no fetched $*" >&2; exit 2; }\n'
            )
            for n in ("python3", "tar"):
                (stubs / n).chmod(0o755)
            r = ds.Runner(Path(d) / "run", base_env={"PATH": str(stubs) + ":" + ds.DEFAULT_PATH}, log=lambda *_: None)
            step = next(s for s in docs["building.md"].steps if s.name == "qemu_build")
            tail = step.text.splitlines()[-1]
            self.assertTrue(tail.startswith("bash scripts/bench/build_kf3.sh "))
            body = "\n".join(step.text.splitlines()[:-1]) + "\n"
            import dataclasses

            r.run(dataclasses.replace(step, text=body))
            calls = (r.home / "calls").read_text().splitlines()
            self.assertEqual(calls, ["STUB python3 -I scripts/ci/qemu_source.py fetch .", "STUB tar -xf " + name])
            self.assertNotIn("curl", body)


class Stage(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location("vast_payload_if", HERE / "vast_payload.py")
        self.m = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.m)

    def inst(self, **over):
        doc = dict(tier="large", windows=False, windows_runs=1, stages=(), spec=None, winapps=None, linux_apps=None, rehearsal=False)
        doc.update(over)
        return pi.Instruction(**doc)

    def test_it_is_selectable_in_the_large_tier_only_and_not_in_a_default_plan(self):
        m = self.m
        self.assertFalse(m.INSTALL_FLOW_IN_LARGE)
        for tier in ("small", "middle", "large", "adhoc"):
            self.assertNotIn("install_flow", m.plan_stage_names(self.inst(tier=tier)))
            self.assertNotIn("install_flow", m.plan_stage_names(self.inst(tier=tier, linux_apps="middle" if tier in ("middle", "large") else None, windows=tier == "large")))
        self.assertEqual(m.plan_stage_names(self.inst(), ("install_flow",))[-1], "install_flow")
        for tier in ("small", "middle", "adhoc"):
            self.assertNotIn("install_flow", m.plan_stage_names(self.inst(tier=tier), ("install_flow",)))
        # it runs after the Linux guests and before the Windows gate, which owns the GPU last
        names = [n for n in m.plan_stage_names(self.inst(windows=True, linux_apps="middle"), ("install_flow",)) if n != "kfml"]  # kfml is a default stage since 2026-11-17
        self.assertEqual(names[names.index("install_flow") - 1], "apps")
        self.assertEqual(names[names.index("install_flow") + 1], "windows_gate")
        with patch.object(m, "INSTALL_FLOW_IN_LARGE", True):
            self.assertIn("install_flow", m.plan_stage_names(self.inst()))
            self.assertNotIn("install_flow", m.plan_stage_names(self.inst(tier="middle")))

    def test_select_stages_accepts_only_what_is_planned(self):
        m = self.m
        plan = m.plan_stage_names(self.inst(), ("install_flow",))
        self.assertEqual(m.select_stages(plan, ["install_flow"], []), ["install_flow"])
        with self.assertRaises(m.EvidenceError):
            m.select_stages(m.plan_stage_names(self.inst()), ["install_flow"], [])

    def test_dry_run_prints_the_plan_and_touches_nothing(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "i.json"
            pi.write(self.inst(), str(path))
            env = {k: v for k, v in os.environ.items() if not k.startswith("KF_")}
            cmd = [sys.executable, str(HERE / "vast_payload.py"), "--dry-run", "--instruction", str(path)]
            ok = subprocess.run([*cmd, "--only", "install_flow"], env=env, capture_output=True, text=True, cwd=d)
            self.assertEqual(ok.returncode, 0, ok.stderr)
            self.assertIn("KF_PAYLOAD_PLAN stages=install_flow planned=provision:cuda_host:merge_bar:cuda_guest:kfml:install_flow", ok.stdout)
            plain = subprocess.run(cmd, env=env, capture_output=True, text=True, cwd=d)
            self.assertIn("planned=provision:cuda_host:merge_bar:cuda_guest:kfml\n", plain.stdout)
            pi.write(self.inst(tier="small"), str(path))
            bad = subprocess.run([*cmd, "--only", "install_flow"], env=env, capture_output=True, text=True, cwd=d)
            self.assertEqual(bad.returncode, 1)
            self.assertIn("KF_PAYLOAD_FAIL suite=preflight", bad.stdout)
            self.assertEqual(sorted(os.listdir(d)), ["i.json"])

    def result_line(self, **over):
        doc = {
            "schema": 1, "verdict": "PASS", "sha": SHA, "docs": {"building.md": "1" * 64},
            "steps": [{"doc": "building.md", "step": "clone", "user": "unpriv", "rc": 0, "ms": 5}], "root_steps": ["root_setup", "packages_install"],
        }
        doc.update(over)
        return "noise\nKF_INSTALL_FLOW_RESULT " + json.dumps(doc) + "\nKF_INSTALL_FLOW_DONE\n", doc

    def test_the_rows_keep_the_result_in_the_result_file_even_when_red(self):
        m = self.m
        text, doc = self.result_line()
        result = {}
        self.assertEqual(m.install_flow_rows(text, 0, SHA, result), [m.row("install_flow", 1)])
        self.assertEqual(result["install_flow"], doc)
        for text, rc, sha in (
            (self.result_line(verdict="FAIL")[0], 0, SHA), (self.result_line()[0], 1, SHA), (self.result_line()[0], 0, "b" * 40),
            ("no result line\n", 0, SHA), (self.result_line()[0] * 2, 0, SHA), ("KF_INSTALL_FLOW_RESULT {not json}\n", 0, SHA),
        ):
            with self.subTest(rc=rc, sha=sha[:3]):
                result = {}
                with self.assertRaises(m.EvidenceError) as caught:
                    m.install_flow_rows(text, rc, sha, result)
                self.assertEqual(caught.exception.name, "install_flow")
        red, doc = self.result_line(verdict="FAIL", reason="step_failed")
        result = {}
        with self.assertRaises(m.EvidenceError):
            m.install_flow_rows(red, 1, SHA, result)
        self.assertEqual(result["install_flow"]["reason"], "step_failed")

    def test_the_reporter_reads_the_record_and_refuses_a_bad_one(self):
        import vast_report

        _, doc = self.result_line()
        base = {"verdict": "PASS", "suites": [{"name": "a", "pass": 1, "fail": 0, "skip": 0}]}
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "r.json"

            def read(value):
                path.write_text(json.dumps({**base, "install_flow": value}))
                return vast_report.read_result_file(path)

            self.assertEqual(read(doc)["install_flow"], doc)
            bad = [
                {**doc, "extra": 1}, {**doc, "verdict": "MAYBE"}, {**doc, "sha": "x"}, {**doc, "docs": {}}, {**doc, "docs": {"../x": "1" * 64}},
                {**doc, "docs": {"a.md": "zz"}}, {**doc, "steps": [{"step": "A", "rc": 0}]}, {**doc, "steps": [{"step": "a", "rc": "0"}]},
                {**doc, "root_steps": ["a"] * 40}, {**doc, "root_steps": [3]}, {**doc, "schema": 2},
                {**doc, "guest": {"pad": "x" * 9000}}, "x", [],
            ]
            for value in bad:
                with self.subTest(value=str(value)[:40]):
                    with self.assertRaises(vast_report.ResultError):
                        read(value)


def docs_with(testcase, mutate):
    """A temporary copy of docs/howto with `mutate(name, text) -> text` applied to every file; returns the tree root."""
    import shutil

    tmp = tempfile.TemporaryDirectory()
    testcase.addCleanup(tmp.cleanup)
    base = Path(tmp.name) / "docs/howto"
    shutil.copytree(ROOT / "docs/howto", base)
    for name in flow.DOCS:
        (base / name).write_bytes(mutate(name, (base / name).read_bytes()))
    return tmp.name


def append_to(doc, extra):
    return lambda name, raw: raw + extra if name == doc else raw


class SecondAngle(unittest.TestCase):
    """The same rules reached through the real docs, the runner or the stage, so each is pinned by two different tests."""

    def refuses_in_docs(self, extra, doc="building.md"):
        root = docs_with(self, append_to(doc, extra))
        with self.assertRaises(ds.DocStepError):
            flow.load_docs(root)

    def test_docs_an_unknown_marker_kind_is_refused(self):
        self.refuses_in_docs(b"\n<!-- kf-exec: z -->\n```sh\ntrue\n```\n")

    def test_docs_a_marker_with_text_after_it_is_refused(self):
        self.refuses_in_docs(b"\n<!-- kf-run: z --> extra\n```sh\ntrue\n```\n")

    def test_docs_a_duplicate_step_name_is_refused(self):
        self.refuses_in_docs(b"\n<!-- kf-run: clone -->\n```sh\ntrue\n```\n")

    def test_docs_a_bad_user_is_refused(self):
        self.refuses_in_docs(b"\n<!-- kf-run: z user=admin -->\n```sh\ntrue\n```\n")

    def test_docs_a_timeout_past_the_cap_is_refused(self):
        self.refuses_in_docs(b"\n<!-- kf-run: z timeout=99999 -->\n```sh\ntrue\n```\n")

    def test_docs_an_empty_block_is_refused(self):
        self.refuses_in_docs(b"\n<!-- kf-run: z -->\n```sh\n```\n")

    def test_docs_sudo_in_an_unprivileged_block_is_refused(self):
        self.refuses_in_docs(b"\n<!-- kf-run: z -->\n```sh\nsudo true\n```\n")

    def test_docs_a_marker_above_an_indented_fence_is_refused(self):
        self.refuses_in_docs(b"\n<!-- kf-run: z -->\n  ```sh\n  true\n  ```\n")

    def test_docs_an_unterminated_fence_is_refused(self):
        self.refuses_in_docs(b"\n```sh\ntrue\n")

    def test_docs_a_marker_at_the_very_end_is_refused(self):
        self.refuses_in_docs(b"\n<!-- kf-run: z -->")

    def test_a_marker_on_the_last_line_without_a_newline_is_refused(self):
        with self.assertRaises(ds.DocStepError):
            parse("<!-- kf-run: z -->")

    def test_a_fence_line_with_an_info_string_inside_a_fence_does_not_end_it(self):
        p = parse("```md\n```sh\n<!-- kf-run: z -->\n```\n")
        self.assertEqual((p.steps, p.fences), ((), 1))

    def test_docs_a_bare_cr_is_refused(self):
        self.refuses_in_docs(b"\ntrue\rtrue\n")

    def test_docs_a_nul_byte_is_refused(self):
        self.refuses_in_docs(b"\ntrue\0true\n")

    def test_docs_an_oversize_file_is_refused(self):
        self.refuses_in_docs(b"x" * (ds.MAX_DOC_BYTES + 1))

    def test_docs_in_crlf_give_the_same_steps_and_the_hash_of_the_bytes(self):
        lf = flow.load_docs(ROOT)
        root = docs_with(self, lambda name, raw: raw.replace(b"\n", b"\r\n"))
        crlf = flow.load_docs(root)
        for name in flow.DOCS:
            self.assertEqual(lf[name].steps, crlf[name].steps, name)
            raw = (Path(root) / "docs/howto" / name).read_bytes()
            self.assertEqual(crlf[name].sha256, hashlib.sha256(raw).hexdigest(), name)
            self.assertNotEqual(lf[name].sha256, crlf[name].sha256, name)

    def test_a_crlf_block_runs_as_lf(self):
        r = Running.runner(self)
        step = parse(block("<!-- kf-run: a -->", "echo one\necho two").replace("\n", "\r\n")).steps[0]
        r.run(step)
        self.assertEqual((r.logs / "01-a.log").read_text(), "one\ntwo\n")

    def test_a_closing_fence_with_an_info_string_does_not_close(self):
        (s,) = parse("<!-- kf-run: a -->\n```sh\necho 1\n```text\necho 2\n```\n").steps
        self.assertEqual(s.text, "echo 1\n```text\necho 2\n")

    def test_docs_a_tilde_fence_around_backticks_is_one_fence(self):
        p = parse("~~~md\n```\nrm -rf /\n~~~\n")
        self.assertEqual((p.steps, p.fences), ((), 1))

    def test_a_shorter_fence_does_not_close_a_longer_one_in_the_docs_either(self):
        p = parse("````md\n```\n<!-- kf-run: z -->\n```\n````\n")
        self.assertEqual((p.steps, p.fences), ((), 1))

    def test_a_marker_kind_exec_is_not_known(self):
        with self.assertRaises(ds.DocStepError):
            parse(block("<!-- kf-exec: z -->"))
        self.assertEqual(ds.KEYS, {"kf-run": ("user", "timeout"), "kf-args": ()})

    def test_the_marker_is_the_whole_line(self):
        with self.assertRaises(ds.DocStepError):
            parse(block("<!-- kf-run: z -->x"))
        with self.assertRaises(ds.DocStepError):
            parse(block("x<!-- kf-run: z -->").replace("x<", " <"))

    def test_the_hash_is_of_the_bytes_even_for_crlf(self):
        raw = block("<!-- kf-run: a -->").replace("\n", "\r\n").encode()
        self.assertEqual(ds.parse(raw, "t.md").sha256, hashlib.sha256(raw).hexdigest())

    def test_a_substitution_literal_that_occurs_twice_in_a_real_block_is_refused(self):
        docs = flow.load_docs(ROOT)
        clone = next(s for s in docs["building.md"].steps if s.name == "clone")
        with tempfile.TemporaryDirectory() as d:
            for literal in ("kayfabe", "no-such-text-in-the-block"):
                with self.assertRaises(ds.DocStepError):
                    ds.substitute(clone, [ds.Substitution("clone", literal, "x")], d)

    def test_a_substitution_with_a_newline_is_refused(self):
        with tempfile.TemporaryDirectory() as d:
            step = parse(block("<!-- kf-run: a -->", "echo X")).steps[0]
            with self.assertRaises(ds.DocStepError):
                ds.substitute(step, [ds.Substitution("a", "X", "y\nz")], d)

    def test_a_substitution_naming_a_sibling_directory_of_the_prefix_is_refused(self):
        with tempfile.TemporaryDirectory() as d:
            step = parse(block("<!-- kf-run: a -->", "echo X")).steps[0]
            with self.assertRaises(ds.DocStepError):
                ds.substitute(step, [ds.Substitution("a", "X", "file://" + d + "-sibling/x")], d)
            self.assertFalse(ds.inside(d, d + "-sibling"))

    def test_a_substitution_naming_a_path_outside_the_prefix_is_refused(self):
        with tempfile.TemporaryDirectory() as d:
            step = parse(block("<!-- kf-run: a -->", "echo X")).steps[0]
            with self.assertRaises(ds.DocStepError):
                ds.substitute(step, [ds.Substitution("a", "X", "/etc/passwd")], d)

    def test_a_symlink_inside_the_prefix_does_not_make_a_path_inside(self):
        with tempfile.TemporaryDirectory() as d, tempfile.TemporaryDirectory() as other:
            (Path(d) / "l").symlink_to(other)
            self.assertFalse(ds.inside(d, d + "/l"))
            step = parse(block("<!-- kf-run: a -->", "echo X")).steps[0]
            with self.assertRaises(ds.DocStepError):
                ds.substitute(step, [ds.Substitution("a", "X", d + "/l/y")], d)

    # ---- runner
    def test_a_block_that_execs_away_fails(self):
        r = Running.runner(self)
        with self.assertRaises(ds.DocStepError):
            r.run(parse(block("<!-- kf-run: a -->", "exec true")).steps[0])
        self.assertEqual(r.records[0]["rc"], 125)

    def test_a_block_with_an_unterminated_heredoc_fails(self):
        r = Running.runner(self)
        with self.assertRaises(ds.DocStepError):
            r.run(parse(block("<!-- kf-run: a -->", "cat <<'X'\nnever closed")).steps[0])
        self.assertNotEqual(r.records[0]["rc"], 0)

    def test_a_cd_to_the_filesystem_root_is_refused(self):
        r = Running.runner(self)
        with self.assertRaises(ds.DocStepError):
            r.run(parse(block("<!-- kf-run: a -->", "cd /")).steps[0])
        self.assertEqual((r.records[0]["rc"], r.records[0]["reason"]), (126, "working directory left the prefix"))

    def test_a_cd_through_a_symlink_out_of_the_prefix_is_refused(self):
        r = Running.runner(self)
        with tempfile.TemporaryDirectory() as outside:
            with self.assertRaises(ds.DocStepError):
                r.run(parse(block("<!-- kf-run: a -->", f"ln -s {outside} esc\ncd esc")).steps[0])
        self.assertEqual(r.records[0]["rc"], 126)

    def carry(self, runner, uid, cap):
        for name, value in (("cwd.new", str(runner.work)), ("uid.new", uid), ("cap.new", "CapEff:\t" + cap), ("path.new", "/bin")):
            (runner.state / name).write_text(value + "\n")
        step = parse(block("<!-- kf-run: a -->")).steps[0]
        record = {}
        runner._carry(step, record)
        return record

    def test_a_unprivileged_step_running_as_uid_zero_is_refused_when_required(self):
        r = Running.runner(self, require_unpriv=True)
        self.assertEqual(self.carry(r, "0", "0000000000000000")["rc"], 127)

    def test_a_unprivileged_step_holding_capabilities_is_refused_when_required(self):
        r = Running.runner(self, require_unpriv=True)
        self.assertEqual(self.carry(r, "1001", "000001ffffffffff")["rc"], 127)
        r = Running.runner(self, require_unpriv=True)
        self.assertNotIn("rc", self.carry(r, "1001", "0000000000000000"))

    def test_a_step_sees_only_the_declared_variables(self):
        r = Running.runner(self)
        with patch.dict(os.environ, {"KF_SECRET_TOKEN": "x", "AWS_SECRET": "y"}):
            r.run(parse(block("<!-- kf-run: a -->", "env | cut -d= -f1 | sort | tr '\\n' ' '")).steps[0])
        names = (r.logs / "01-a.log").read_text().split()
        self.assertEqual(set(names) - {"PWD", "SHLVL", "_", "OLDPWD"}, {"HOME", "TMPDIR", "USER", "LOGNAME", "LANG", "TERM", "PATH", "DEBIAN_FRONTEND"})

    def test_path_set_by_an_env_file_reaches_the_next_block_like_cargo_env(self):
        r = Running.runner(self)
        r.run(parse(block("<!-- kf-run: a -->", 'mkdir -p "$HOME/.cargo/bin"\nprintf \'#!/bin/sh\\necho tool\\n\' > "$HOME/.cargo/bin/rustup-stub"\nchmod +x "$HOME/.cargo/bin/rustup-stub"\nprintf \'export PATH="$HOME/.cargo/bin:$PATH"\\n\' > "$HOME/.cargo/env"\n. "$HOME/.cargo/env"')).steps[0])
        r.run(parse(block("<!-- kf-run: b -->", "rustup-stub")).steps[0])
        self.assertEqual((r.logs / "02-b.log").read_text(), "tool\n")

    def test_the_working_directory_carries_across_three_blocks(self):
        r = Running.runner(self)
        for name, body in (("a", "mkdir -p x/y"), ("b", "cd x"), ("c", "cd y"), ("d", 'basename "$PWD"')):
            r.run(parse(block(f"<!-- kf-run: {name} -->", body)).steps[0])
        self.assertEqual((r.logs / "04-d.log").read_text(), "y\n")

    def test_a_root_step_has_a_sudo_function(self):
        r = Running.runner(self)
        r.run(parse(block("<!-- kf-run: a user=root -->", 'test "$(type -t sudo)" = function')).steps[0])
        self.assertEqual(r.records[0]["rc"], 0)

    def test_the_real_packages_install_block_runs_without_a_sudo_binary(self):
        docs = flow.load_docs(ROOT)
        step = next(s for s in docs["building.md"].steps if s.name == "packages_install")
        with tempfile.TemporaryDirectory() as d:
            bins = Path(d) / "bin"
            bins.mkdir()
            for tool in ("bash", "id", "grep", "cat", "tr", "env"):
                found = shutil_which(tool)
                (bins / tool).symlink_to(found)
            (bins / "apt-get").write_text('#!/bin/sh\necho "$@" > "$HOME/apt.args"\n')
            (bins / "apt-get").chmod(0o755)
            r = ds.Runner(Path(d) / "run", base_env={"PATH": str(bins)}, log=lambda *_: None)
            (r.work / "scripts/bench").mkdir(parents=True)
            (r.work / "scripts/bench/provision_box.sh").write_bytes((ROOT / "scripts/bench/provision_box.sh").read_bytes())
            r.run(step)
            args = (r.home / "apt.args").read_text().split()
            for want in ("-y", "build-essential", "ninja-build", "libslirp-dev", "install"):
                self.assertIn(want, args)

    def test_the_drop_prefix_is_not_applied_to_a_root_step(self):
        r = Running.runner(self, drop=["/usr/bin/env", "KF_DROPPED=1"])
        r.run(parse(block("<!-- kf-run: a user=root -->", 'test -z "${KF_DROPPED:-}"')).steps[0])
        self.assertEqual(r.records[0]["rc"], 0)

    def test_the_drop_prefix_is_applied_to_an_unprivileged_step(self):
        r = Running.runner(self, drop=["/usr/bin/env", "KF_DROPPED=1"])
        r.run(parse(block("<!-- kf-run: a -->", 'test "${KF_DROPPED:-}" = 1')).steps[0])
        self.assertEqual(r.records[0]["rc"], 0)

    def test_a_failed_block_is_recorded_and_the_next_one_is_not_run_by_the_caller(self):
        r = Running.runner(self)
        steps = parse(block("<!-- kf-run: a -->", "false") + block("<!-- kf-run: b -->", "echo ok")).steps
        ran = []
        with self.assertRaises(ds.DocStepError):
            for s in steps:
                r.run(s)
                ran.append(s.name)
        self.assertEqual(ran, [])
        self.assertEqual([x["rc"] for x in r.records], [1])

    def test_a_timeout_is_recorded_as_124_with_its_end_line(self):
        r = Running.runner(self)
        with self.assertRaises(ds.DocStepError):
            r.run(parse(block("<!-- kf-run: a timeout=1 -->", "sleep 4")).steps[0])
        self.assertEqual(r.records[0]["rc"], 124)
        self.assertRegex(self.lines[-1], r"^KF_DOCSTEP_END doc=t.md step=a rc=124 ms=\d+$")

    def test_a_failure_with_a_pipe_in_the_middle_fails_under_pipefail(self):
        r = Running.runner(self)
        with self.assertRaises(ds.DocStepError):
            r.run(parse(block("<!-- kf-run: a -->", "false | cat")).steps[0])
        self.assertEqual(r.records[0]["rc"], 1)

    def test_an_unsafe_prefix_with_shell_characters_is_refused(self):
        with tempfile.TemporaryDirectory() as d:
            for name in ("a;b", "a$b", "a'b"):
                with self.assertRaises(ds.DocStepError):
                    ds.Runner(Path(d) / name)

    # ---- install_flow
    def test_the_attested_path_is_refused_by_the_function(self):
        with self.assertRaises(flow.FlowError):
            flow.resolve_binary(SHA, "/x/qemu")

    def test_the_attested_path_is_refused_by_the_command_before_anything_is_created(self):
        with tempfile.TemporaryDirectory() as d:
            prefix = Path(d) / "p"
            r = subprocess.run(
                [sys.executable, str(HERE / "install_flow.py"), "run", "--sha", SHA, "--checkout", str(ROOT), "--prefix", str(prefix),
                 "--out", str(Path(d) / "o.json"), "--attested-binary", "/x/qemu"], capture_output=True, text=True)
            self.assertEqual(r.returncode, 2)
            self.assertFalse(prefix.exists())

    def test_a_marker_nobody_planned_is_refused_by_check_plan(self):
        root = docs_with(self, append_to("linux-guest.md", b"\n<!-- kf-run: extra_step -->\n```sh\ntrue\n```\n"))
        with self.assertRaises(flow.FlowError) as caught:
            flow.check_plan(flow.load_docs(root))
        self.assertEqual(caught.exception.reason, "unplanned_marker")

    def test_check_plan_refuses_a_marker_absent_from_the_plan_even_in_a_planned_doc(self):
        with patch.object(flow, "PLAN", tuple(x for x in flow.PLAN if x[2] != "clone")):
            with self.assertRaises(flow.FlowError):
                flow.check_plan(flow.load_docs(ROOT))

    def test_a_planned_step_with_no_marker_is_refused_by_check_plan(self):
        root = docs_with(self, lambda n, raw: raw.replace(b"<!-- kf-run: raw_client timeout=3600 -->\n", b"") if n == "building.md" else raw)
        with self.assertRaises(flow.FlowError) as caught:
            flow.check_plan(flow.load_docs(root))
        self.assertEqual(caught.exception.reason, "planned_step_missing")

    def test_check_plan_refuses_a_planned_step_that_no_doc_has(self):
        with patch.object(flow, "PLAN", (*flow.PLAN, ("doc", "building.md", "ghost"))):
            with self.assertRaises(flow.FlowError):
                flow.check_plan(flow.load_docs(ROOT))

    def test_substitutions_replace_the_clone_url_and_the_uuid_and_fb_mb_only_when_needed(self):
        small = flow.substitutions("/p/m.git", 8192)
        big = flow.substitutions("/p/m.git", 24564)
        self.assertEqual([(s.step, s.literal) for s in small], [("clone", flow.REPO_URL), ("qemu_args", flow.UUID_PLACEHOLDER), ("qemu_args", flow.FB_PLACEHOLDER)])
        self.assertEqual([(s.step, s.literal) for s in big], [("clone", flow.REPO_URL), ("qemu_args", flow.UUID_PLACEHOLDER)])
        self.assertEqual(small[2].replacement, "fb-mb=6144")

    def test_every_substitution_literal_occurs_exactly_once_in_the_real_docs(self):
        docs = flow.load_docs(ROOT)
        with tempfile.TemporaryDirectory() as d:
            for sub in flow.substitutions(d + "/m.git", 8192):
                step = next(s for p in docs.values() for s in p.steps if s.name == sub.step)
                self.assertEqual(step.text.count(sub.literal), 1, sub.literal)

    def test_the_drop_has_the_capability_sets_emptied(self):
        argv = flow.drop_argv(1001, 1001, 108)
        self.assertEqual([a for a in argv if a.endswith("=-all")], ["--inh-caps=-all", "--ambient-caps=-all", "--bounding-set=-all"])

    def test_the_drop_sets_no_new_privs(self):
        self.assertIn("--no-new-privs", flow.drop_argv(1001, 1001, 108))

    def test_the_drop_refuses_uid_zero_and_nonpositive_ids(self):
        for bad in ((0, 1, 2), (1, 0, 2), (1, 1, 0), (-5, 1, 1)):
            with self.assertRaises(flow.FlowError):
                flow.drop_argv(*bad)

    def test_the_drop_refuses_a_string_id(self):
        with self.assertRaises(flow.FlowError):
            flow.drop_argv("1001", 1001, 108)

    def test_a_small_card_gets_less_framebuffer(self):
        self.assertEqual(flow.fb_mb_for(8192), 6144)

    def test_a_big_card_keeps_8192(self):
        self.assertEqual(flow.fb_mb_for(49152), 8192)

    def test_the_built_binary_of_another_revision_is_refused(self):
        with tempfile.TemporaryDirectory() as d:
            line = f"KF3_BUILT {d}/k/qemu-system-x86_64 rev=deadbeef mtime=x\n"
            with self.assertRaises(flow.FlowError):
                flow.built_binary(line, SHA, d)

    def test_the_built_binary_outside_the_prefix_is_refused(self):
        with tempfile.TemporaryDirectory() as d:
            line = f"KF3_BUILT /usr/bin/qemu-system-x86_64 rev={SHA[:8]} mtime=x\n"
            with self.assertRaises(flow.FlowError):
                flow.built_binary(line, SHA, d)

    def test_cup8_with_bad_elements_does_not_grade(self):
        self.assertFalse(flow.grade_cup8("CUP8 RESULT N=2048 bad=12 maxerr=0 C[0]=1(exp 1) -> PASS\n"))

    def test_cup8_graded_from_an_indented_line_does_not_grade(self):
        self.assertFalse(flow.grade_cup8("   CUP8 RESULT N=2048 bad=0 maxerr=0 C[0]=1(exp 1) -> PASS\n"))

    def test_a_foreign_directory_is_never_removed_as_a_prefix(self):
        with tempfile.TemporaryDirectory() as d:
            (Path(d) / "keep").write_text("x")
            with self.assertRaises(flow.FlowError):
                flow.prepare_prefix(d)
            self.assertTrue((Path(d) / "keep").exists())

    def test_result_doc_lists_root_steps_of_the_runner_and_of_the_setup(self):
        doc = self.result(root_setup=["root_setup"])
        self.assertEqual(doc["root_steps"], ["packages_install", "root_setup"])

    def test_result_doc_root_steps_cover_every_step_recorded_as_root(self):
        doc = self.result(root_setup=[])
        self.assertTrue({s["step"] for s in doc["steps"] if s["user"] == "root"} <= set(doc["root_steps"]))

    def test_result_doc_carries_the_hash_of_every_doc(self):
        doc = self.result()
        self.assertEqual(doc["docs"], {n: hashlib.sha256((ROOT / "docs/howto" / n).read_bytes()).hexdigest() for n in flow.DOCS})

    def test_result_doc_names_every_doc_consulted(self):
        self.assertEqual(sorted(self.result()["docs"]), sorted(flow.DOCS))

    def result(self, root_setup=("root_setup",)):
        class R:
            records = [{"doc": "building.md", "step": "packages_install", "user": "root", "rc": 0, "ms": 1}]

            def root_steps(self):
                return ["packages_install"]

        return flow.result_doc(SHA, flow.load_docs(ROOT), R(), [], {"source": "compiled"}, {}, "PASS", None, list(root_setup))

    # ---- payload and reporter
    def payload_module(self):
        spec = importlib.util.spec_from_file_location("vast_payload_sa", HERE / "vast_payload.py")
        m = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(m)
        return m

    def rows(self, **over):
        doc = {"schema": 1, "verdict": "PASS", "sha": SHA, "docs": {"building.md": "1" * 64}, "steps": [], "root_steps": []}
        doc.update(over)
        return "KF_INSTALL_FLOW_RESULT " + json.dumps(doc) + "\n"

    def red(self, text, rc=0, sha=SHA):
        m = self.payload_module()
        with self.assertRaises(m.EvidenceError):
            m.install_flow_rows(text, rc, sha, {})

    def test_rows_refuse_a_red_verdict_with_rc_zero(self):
        self.red(self.rows(verdict="FAIL"))

    def test_rows_refuse_a_green_verdict_with_rc_nonzero(self):
        self.red(self.rows(), rc=1)

    def test_rows_refuse_a_record_of_another_commit(self):
        self.red(self.rows(), sha="b" * 40)

    def test_rows_refuse_two_result_lines(self):
        self.red(self.rows() + self.rows())

    def test_rows_refuse_two_result_lines_when_the_last_is_green(self):
        self.red(self.rows(verdict="FAIL") + self.rows())

    def test_rows_keep_the_record_in_the_result_when_it_is_red(self):
        m = self.payload_module()
        result = {}
        with self.assertRaises(m.EvidenceError):
            m.install_flow_rows(self.rows(verdict="FAIL"), 1, SHA, result)
        self.assertEqual(result["install_flow"]["verdict"], "FAIL")

    def test_rows_keep_the_record_in_the_result_when_it_is_green(self):
        m = self.payload_module()
        result = {}
        m.install_flow_rows(self.rows(), 0, SHA, result)
        self.assertEqual(result["install_flow"]["sha"], SHA)

    def test_the_stage_is_not_in_a_default_large_plan(self):
        self.assertFalse(self.payload_module().INSTALL_FLOW_IN_LARGE)

    def test_the_default_plans_of_every_tier_have_no_install_flow(self):
        m = self.payload_module()
        for tier in ("small", "middle", "large", "adhoc"):
            doc = dict(tier=tier, windows=False, windows_runs=1, stages=(), spec=None, winapps=None, linux_apps=None, rehearsal=False)
            self.assertNotIn("install_flow", m.plan_stage_names(pi.Instruction(**doc)))

    def test_only_does_not_add_the_stage_outside_the_large_tier(self):
        m = self.payload_module()
        for tier in ("small", "middle", "adhoc"):
            doc = dict(tier=tier, windows=False, windows_runs=1, stages=(), spec=None, winapps=None, linux_apps=None, rehearsal=False)
            self.assertNotIn("install_flow", m.plan_stage_names(pi.Instruction(**doc), ("install_flow",)))

    def test_only_adds_the_stage_in_the_large_tier(self):
        m = self.payload_module()
        doc = dict(tier="large", windows=False, windows_runs=1, stages=(), spec=None, winapps=None, linux_apps=None, rehearsal=False)
        self.assertIn("install_flow", m.plan_stage_names(pi.Instruction(**doc), ("install_flow",)))

    def test_the_stage_comes_before_the_windows_gate(self):
        m = self.payload_module()
        doc = dict(tier="large", windows=True, windows_runs=1, stages=(), spec=None, winapps=None, linux_apps=None, rehearsal=False)
        names = m.plan_stage_names(pi.Instruction(**doc), ("install_flow",))
        self.assertLess(names.index("install_flow"), names.index("windows_gate"))

    def test_the_stage_comes_after_cuda_guest(self):
        m = self.payload_module()
        doc = dict(tier="large", windows=False, windows_runs=1, stages=(), spec=None, winapps=None, linux_apps=None, rehearsal=False)
        names = [n for n in m.plan_stage_names(pi.Instruction(**doc), ("install_flow",)) if n != "kfml"]
        self.assertEqual(names[-2:], ["cuda_guest", "install_flow"])

    def test_the_stage_needs_hours_left(self):
        m = self.payload_module()
        self.assertGreaterEqual(m.INSTALL_FLOW_MIN_REMAINING_MS, 3 * 3600 * 1000)

    # ---- reporter
    def test_the_reporter_reads_a_record_built_by_the_flow(self):
        import vast_report

        doc = json.loads(self.rows()[len("KF_INSTALL_FLOW_RESULT "):])
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "r.json"
            path.write_text(json.dumps({"verdict": "PASS", "suites": [{"name": "a", "pass": 1, "fail": 0, "skip": 0}], "install_flow": doc}))
            self.assertEqual(vast_report.read_result_file(path)["install_flow"], doc)

    def reporter_refuses(self, **over):
        import vast_report

        doc = json.loads(self.rows()[len("KF_INSTALL_FLOW_RESULT "):])
        doc.update(over)
        with self.assertRaises(ValueError):
            vast_report.validate_install_flow(doc)

    def test_the_reporter_refuses_a_doc_hash_that_is_not_hex(self):
        self.reporter_refuses(docs={"building.md": "zz"})

    def test_the_reporter_refuses_a_doc_name_with_a_path(self):
        self.reporter_refuses(docs={"../building.md": "1" * 64})

    def test_the_reporter_refuses_a_record_over_its_size_bound(self):
        self.reporter_refuses(guest={"pad": "x" * 9000})

    def test_the_reporter_refuses_an_unknown_field(self):
        self.reporter_refuses(extra=1)

    def test_the_reporter_ignores_nothing_unvalidated_in_a_result_file(self):
        import vast_report

        doc = json.loads(self.rows()[len("KF_INSTALL_FLOW_RESULT "):])
        doc["sha"] = "short"
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "r.json"
            path.write_text(json.dumps({"verdict": "PASS", "suites": [{"name": "a", "pass": 1, "fail": 0, "skip": 0}], "install_flow": doc}))
            with self.assertRaises(vast_report.ResultError):
                vast_report.read_result_file(path)


class Budget(unittest.TestCase):
    """scripts/ci/profile_resources.py: a manual-only stage needs no budget row, and only while it is out of every default plan."""

    def setUp(self):
        import profile_resources

        self.pr = profile_resources
        self.data = json.loads((ROOT / "ci/vast-broker/defaults.json").read_text())
        self.source = (HERE / "vast_payload.py").read_text()

    def test_the_live_payload_and_the_live_table_agree(self):
        self.assertEqual(self.pr.validate(self.data, self.source), [])
        self.assertNotIn("install_flow", self.data["payloadStageResources"])

    def test_the_flag_being_true_while_listed_manual_only_is_refused(self):
        errors = self.pr.validate(self.data, self.source.replace("INSTALL_FLOW_IN_LARGE = False", "INSTALL_FLOW_IN_LARGE = True"))
        self.assertIn("manual-only stage is in a default plan: install_flow", errors)

    def test_a_stage_that_is_neither_budgeted_nor_manual_only_is_refused(self):
        errors = self.pr.validate(self.data, self.source.replace('MANUAL_ONLY_STAGES = {"propimage_apps": "PROPIMAGE_APPS_IN_LARGE", "propimage_apps_b": "PROPIMAGE_APPS_B_IN_LARGE", "matlab": "MATLAB_IN_LARGE", "install_flow": "INSTALL_FLOW_IN_LARGE"}', 'MANUAL_ONLY_STAGES = {"propimage_apps": "PROPIMAGE_APPS_IN_LARGE", "propimage_apps_b": "PROPIMAGE_APPS_B_IN_LARGE", "matlab": "MATLAB_IN_LARGE"}'))
        self.assertEqual(errors, ["unbudgeted payload stage: install_flow"])

    def test_a_manual_only_stage_that_also_has_a_row_is_refused(self):
        data = json.loads(json.dumps(self.data))
        data["payloadStageResources"]["install_flow"] = dict(data["payloadStageResources"]["specviewperf"], profiles=[])
        self.assertIn("manual-only stage has a budget row: install_flow", self.pr.validate(data, self.source))

    def test_a_flag_that_is_not_a_literal_false_is_refused(self):
        errors = self.pr.validate(self.data, self.source.replace("INSTALL_FLOW_IN_LARGE = False", "INSTALL_FLOW_IN_LARGE = bool(1)"))
        self.assertIn("manual-only stage is in a default plan: install_flow", errors)

    def test_the_live_table_has_no_unbudgeted_stage(self):
        self.assertEqual([e for e in self.pr.validate(self.data, self.source) if e.startswith("unbudgeted")], [])

    def test_a_row_for_a_manual_only_stage_is_refused_even_when_it_charges_a_profile(self):
        data = json.loads(json.dumps(self.data))
        data["payloadStageResources"]["install_flow"] = dict(data["payloadStageResources"]["specviewperf"], profiles=["large"])
        self.assertIn("manual-only stage has a budget row: install_flow", self.pr.validate(data, self.source))

    def test_a_flag_of_zero_is_not_false(self):
        errors = self.pr.validate(self.data, self.source.replace("INSTALL_FLOW_IN_LARGE = False", "INSTALL_FLOW_IN_LARGE = 0"))
        self.assertIn("manual-only stage is in a default plan: install_flow", errors)

    def test_a_non_literal_table_is_refused(self):
        with self.assertRaises(ValueError):
            self.pr.manual_only(self.source.replace('MANUAL_ONLY_STAGES = {"propimage_apps": "PROPIMAGE_APPS_IN_LARGE", "propimage_apps_b": "PROPIMAGE_APPS_B_IN_LARGE", "matlab": "MATLAB_IN_LARGE", "install_flow": "INSTALL_FLOW_IN_LARGE"}', "MANUAL_ONLY_STAGES = dict(a=1)"))


def shutil_which(tool):
    import shutil

    found = shutil.which(tool)
    if not found:
        raise unittest.SkipTest("no " + tool)
    return found


class StageInMain(unittest.TestCase):
    """main() builds the stage from the large instruction plus --only, runs it as `install_flow`, and keeps its record."""

    setUp = v7.V7Tests.setUp
    _main_fixture = v7.V7Tests._main_fixture

    def run_main(self, doc, extra, stage_logs=None):
        orig = self.m.parse_args
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "i.json"
            path.write_text(json.dumps(doc))
            with patch.object(self.m, "parse_args", side_effect=lambda argv: orig([*argv, "--instruction", str(path), *extra])):
                return self._main_fixture(windows=None, soft_ms=900 * 60000, stage_logs=stage_logs)

    DOC = {
        "schema": pi.SCHEMA_ID, "tier": "large", "windows": False, "windows_runs": 1, "stages": [], "spec": None,
        "winapps": None, "linux_apps": None, "rehearsal": False,
    }

    def record(self, **over):
        doc = {"schema": 1, "verdict": "PASS", "sha": v7.SHA, "docs": {"building.md": "1" * 64}, "steps": [], "root_steps": ["root_setup", "packages_install"]}
        doc.update(over)
        return doc, "KF_INSTALL_FLOW_RESULT " + json.dumps(doc) + "\n"

    def test_only_install_flow_runs_one_stage_and_a_missing_result_is_a_named_failure(self):
        rc, result, calls = self.run_main(self.DOC, ["--only", "install_flow"], {"install_flow": ("crashed\n", 1)})
        self.assertEqual(calls, ["install_flow"])
        self.assertEqual(rc, 1)
        self.assertEqual([r["name"] for r in result["suites"] if r["fail"]], ["install_flow"])
        self.assertEqual(result["selection"], {"only": ["install_flow"], "skip": []})
        self.assertNotIn("install_flow", result)

    def test_a_green_record_passes_and_lands_in_the_result_file_with_the_doc_hashes(self):
        doc, text = self.record()
        rc, result, calls = self.run_main(self.DOC, ["--only", "install_flow"], {"install_flow": (text, 0)})
        self.assertEqual((rc, result["verdict"]), (0, "PASS"))
        self.assertEqual(result["install_flow"], doc)
        self.assertIn({"name": "install_flow", "pass": 1, "fail": 0, "skip": 0}, result["suites"])

    def test_a_red_record_fails_the_run_and_is_still_kept(self):
        doc, text = self.record(verdict="FAIL", reason="step_failed")
        rc, result, calls = self.run_main(self.DOC, ["--only", "install_flow"], {"install_flow": (text, 1)})
        self.assertEqual((rc, result["verdict"]), (1, "FAIL"))
        self.assertEqual(result["install_flow"], doc)

    def test_a_red_record_with_exit_zero_still_fails_the_run(self):
        doc, text = self.record(verdict="FAIL")
        rc, result, calls = self.run_main(self.DOC, ["--only", "install_flow"], {"install_flow": (text, 0)})
        self.assertEqual(rc, 1)

    def test_a_green_record_of_another_commit_fails_the_run(self):
        doc, text = self.record(sha="b" * 40)
        rc, result, calls = self.run_main(self.DOC, ["--only", "install_flow"], {"install_flow": (text, 0)})
        self.assertEqual(rc, 1)

    def test_a_green_record_with_a_nonzero_exit_fails_the_run(self):
        doc, text = self.record()
        rc, result, calls = self.run_main(self.DOC, ["--only", "install_flow"], {"install_flow": (text, 1)})
        self.assertEqual(rc, 1)

    def test_two_result_lines_fail_the_run(self):
        doc, text = self.record()
        rc, result, calls = self.run_main(self.DOC, ["--only", "install_flow"], {"install_flow": (text + text, 0)})
        self.assertEqual(rc, 1)

    def test_with_too_little_time_left_the_stage_is_not_started(self):
        doc, text = self.record()
        orig = self.m.parse_args
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "i.json"
            path.write_text(json.dumps(self.DOC))
            with patch.object(self.m, "parse_args", side_effect=lambda argv: orig([*argv, "--instruction", str(path), "--only", "install_flow"])):
                rc, result, calls = self._main_fixture(windows=None, soft_ms=2 * 3600 * 1000, stage_logs={"install_flow": (text, 0)})
        self.assertEqual((rc, calls), (1, []))
        self.assertEqual([r["name"] for r in result["suites"] if r["fail"]], ["install_flow_retry_time"])

    def test_with_one_hour_left_there_is_no_record_and_the_run_fails(self):
        doc, text = self.record()
        orig = self.m.parse_args
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "i.json"
            path.write_text(json.dumps(self.DOC))
            with patch.object(self.m, "parse_args", side_effect=lambda argv: orig([*argv, "--instruction", str(path), "--only", "install_flow"])):
                rc, result, calls = self._main_fixture(windows=None, soft_ms=3600 * 1000, stage_logs={"install_flow": (text, 0)})
        self.assertEqual(result["verdict"], "FAIL")
        self.assertNotIn("install_flow", result)

    def test_with_hours_left_the_stage_is_started(self):
        doc, text = self.record()
        orig = self.m.parse_args
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "i.json"
            path.write_text(json.dumps(self.DOC))
            with patch.object(self.m, "parse_args", side_effect=lambda argv: orig([*argv, "--instruction", str(path), "--only", "install_flow"])):
                rc, result, calls = self._main_fixture(windows=None, soft_ms=4 * 3600 * 1000, stage_logs={"install_flow": (text, 0)})
        self.assertEqual((rc, calls), (0, ["install_flow"]))

    def test_a_default_large_run_does_not_run_it(self):
        rc, result, calls = self.run_main(self.DOC, [])
        self.assertNotIn("install_flow", calls)

    def test_the_stage_command_is_the_script_with_the_exact_commit(self):
        seen = []
        real = self.m.watched
        with patch.object(self.m, "watched", lambda command, root, name: (seen.append(list(command)), real(command, root, name))[1]):
            self.run_main(self.DOC, ["--only", "install_flow"])
        (cmd,) = [c for c in seen if "install_flow.py" in " ".join(c)]
        self.assertEqual(cmd[0], "python3")
        self.assertEqual(cmd[2], "run")
        self.assertEqual(cmd[cmd.index("--prefix") + 1], self.m.INSTALL_FLOW_PREFIX)
        self.assertRegex(cmd[cmd.index("--sha") + 1], "^[0-9a-f]{40}$")


class Flow(unittest.TestCase):
    def test_the_attested_binary_hook_is_defined_and_refuses_for_now(self):
        self.assertIsNone(flow.resolve_binary(SHA))
        with self.assertRaises(flow.FlowError) as caught:
            flow.resolve_binary(SHA, "/tmp/some/qemu-system-x86_64")
        self.assertEqual(caught.exception.reason, "attested_not_implemented")
        r = subprocess.run(
            [sys.executable, str(HERE / "install_flow.py"), "run", "--sha", SHA, "--checkout", str(ROOT), "--prefix", "/tmp/kf-if-test",
             "--out", "/tmp/kf-if-test.json", "--attested-binary", "/x/qemu"], capture_output=True, text=True)
        self.assertEqual(r.returncode, 2)
        self.assertIn("reason=attested_not_implemented", r.stdout)
        self.assertFalse(Path("/tmp/kf-if-test").exists())

    def test_the_plan_command_runs_nothing_and_lists_docs_with_hashes(self):
        r = subprocess.run([sys.executable, str(HERE / "install_flow.py"), "plan"], capture_output=True, text=True, cwd="/")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout.count("KF_INSTALL_FLOW_PLAN"), len(flow.PLAN))
        for name in flow.DOCS:
            self.assertRegex(r.stdout, r"KF_INSTALL_FLOW_DOC " + name.replace(".", r"\.") + r" sha256=[0-9a-f]{64} ")
        self.assertIn("packages_install run user=root", r.stdout)

    def test_a_marker_nobody_planned_and_a_planned_step_nobody_marked_are_refused(self):
        docs = flow.load_docs(ROOT)
        with patch.object(flow, "PLAN", tuple(x for x in flow.PLAN if x[2] != "raw_client")):
            with self.assertRaises(flow.FlowError) as caught:
                flow.check_plan(docs)
            self.assertEqual(caught.exception.reason, "unplanned_marker")
        with patch.object(flow, "PLAN", (*flow.PLAN, ("doc", "building.md", "ghost"))):
            with self.assertRaises(flow.FlowError) as caught:
                flow.check_plan(docs)
            self.assertEqual(caught.exception.reason, "planned_step_missing")
        with patch.object(flow, "PLAN", (*flow.PLAN, ("py", None, "clone"))):
            with self.assertRaises(flow.FlowError):
                flow.check_plan(docs)
        with patch.object(flow, "PLAN", tuple(x for x in flow.PLAN if x[2] != "raw_client")), patch.object(flow, "NOT_RUN", {("building.md", "raw_client"): "why"}):
            flow.check_plan(docs)

    def test_a_missing_step_in_a_doc_is_refused_before_anything_runs(self):
        with tempfile.TemporaryDirectory() as d:
            (Path(d) / "docs").mkdir()
            import shutil

            shutil.copytree(ROOT / "docs/howto", Path(d) / "docs/howto")
            p = Path(d) / "docs/howto/attach-to-existing-vm.md"
            p.write_text(p.read_text().replace("<!-- kf-args: qemu_args -->\n", ""))
            with self.assertRaises(flow.FlowError) as caught:
                flow.check_plan(flow.load_docs(d))
            self.assertEqual(caught.exception.reason, "planned_step_missing")
            p.write_text(p.read_text() + "\n<!-- kf-exec: a -->\n")
            with self.assertRaises(ds.DocStepError):
                flow.load_docs(d)

    def test_the_card_rule_for_fb_mb(self):
        self.assertEqual(flow.fb_mb_for(24564), 8192)
        self.assertEqual(flow.fb_mb_for(10240), 8192)
        self.assertEqual(flow.fb_mb_for(8192), 6144)
        self.assertEqual(flow.fb_mb_for(10000), 7168)
        with self.assertRaises(flow.FlowError):
            flow.fb_mb_for(2048)

    def test_the_drop_has_no_capabilities_and_no_root_identity(self):
        argv = flow.drop_argv(1001, 1001, 108)
        for need in ("--no-new-privs", "--inh-caps=-all", "--ambient-caps=-all", "--bounding-set=-all"):
            self.assertIn(need, argv)
        self.assertEqual(argv[argv.index("--reuid") + 1], "1001")
        self.assertEqual(argv[argv.index("--groups") + 1], "108")
        for bad in ((0, 1, 2), (1, 0, 2), (1, 1, 0), (-1, 1, 1), ("1", 1, 1)):
            with self.assertRaises(flow.FlowError):
                flow.drop_argv(*bad)

    def test_the_qemu_command_keeps_every_documented_word_in_order(self):
        words = ["-object", "memory-backend-memfd,id=ram0,size=8192M,share=on", "-device", "kf3-gpu,id=kf0"]
        cmd = flow.qemu_command("/p/q", words, "/p/o.qcow2", "/p/s.log", 2222)
        self.assertEqual(cmd[:5], ["/p/q", *words])
        self.assertIn("if=virtio,file=/p/o.qcow2,format=qcow2", cmd)
        self.assertTrue(any("hostfwd=tcp:127.0.0.1:2222-192.168.77.2:22" in c for c in cmd))
        self.assertNotIn("-daemonize", cmd)
        self.assertEqual(cmd[cmd.index("-display") + 1], "none")

    def test_the_built_binary_must_be_this_commit_and_inside_the_prefix(self):
        with tempfile.TemporaryDirectory() as d:
            ok = f"x\nKF3_BUILT {d}/kayfabe/kf3-bins/{SHA[:8]}/qemu-system-x86_64 rev={SHA[:8]} mtime=now (shared build dir /y)\n"
            self.assertEqual(flow.built_binary(ok, SHA, d), (f"{d}/kayfabe/kf3-bins/{SHA[:8]}/qemu-system-x86_64", SHA[:8]))
            for bad in (
                ok.replace("rev=" + SHA[:8], "rev=" + SHA[:8] + "-dirty"), ok.replace("rev=" + SHA[:8], "rev=deadbeef"),
                ok.replace(d, "/usr/local"), ok.replace(d, d + "/../x"), ok + ok, "nothing\n",
            ):
                with self.subTest(bad=bad[:60]):
                    with self.assertRaises(flow.FlowError):
                        flow.built_binary(bad, SHA, d)

    def test_cup8_is_graded_on_its_own_result_line(self):
        good = "CUP8 matmul N=2048\nCUP8 RESULT N=2048 bad=0 maxerr=0 C[0]=2048(exp 2048) -> PASS\nDONE\n"
        self.assertTrue(flow.grade_cup8(good))
        for bad in (
            good.replace("bad=0", "bad=3"), good.replace("maxerr=0", "maxerr=0.5"), good.replace("N=2048 bad", "N=16 bad"),
            good.replace("-> PASS", "-> MISMATCH"), "\n".join("  " + x for x in good.splitlines()), "no result\n", "",
        ):
            with self.subTest(bad=bad[:50]):
                self.assertFalse(flow.grade_cup8(bad))

    def test_the_result_carries_doc_hashes_root_steps_and_substitutions(self):
        docs = flow.load_docs(ROOT)

        class R:
            records = [
                {"doc": "building.md", "step": "clone", "user": "unpriv", "rc": 0, "ms": 3, "uid": 1001, "cap_eff": "0000000000000000"},
                {"doc": "building.md", "step": "packages_install", "user": "root", "rc": 0, "ms": 9, "uid": 0, "cap_eff": "x"},
            ]

            def root_steps(self):
                return ["packages_install"]

        subs = [ds.Substitution("clone", flow.REPO_URL, "file:///p/m")]
        doc = flow.result_doc(SHA, docs, R(), subs, {"source": "compiled"}, {"cup8": "PASS"}, "PASS", None, ["root_setup", "guest_access"])
        self.assertEqual(set(doc["docs"]), set(flow.DOCS))
        for name, parsed in docs.items():
            self.assertEqual(doc["docs"][name], hashlib.sha256((ROOT / "docs/howto" / name).read_bytes()).hexdigest())
        self.assertEqual(doc["root_steps"], ["packages_install", "root_setup", "guest_access"])
        self.assertEqual(doc["substitutions"], [{"step": "clone", "literal": flow.REPO_URL}])
        self.assertEqual(doc["steps"][0]["uid"], 1001)
        import vast_report

        vast_report.validate_install_flow(doc)

    def test_a_prefix_that_is_not_ours_is_not_deleted(self):
        with tempfile.TemporaryDirectory() as d:
            mine = Path(d) / "mine"
            mine.mkdir()
            (mine / "precious").write_text("x")
            with self.assertRaises(flow.FlowError):
                flow.prepare_prefix(mine)
            self.assertTrue((mine / "precious").exists())
            fresh = flow.prepare_prefix(Path(d) / "fresh")
            (fresh / "junk").write_text("x")
            again = flow.prepare_prefix(fresh)
            self.assertEqual(sorted(p.name for p in again.iterdir()), [flow.MARK])
            for bad in ("/", "relative", d + "/a b", d + "/$x"):
                with self.assertRaises(flow.FlowError):
                    flow.prepare_prefix(bad)


if __name__ == "__main__":
    unittest.main()
