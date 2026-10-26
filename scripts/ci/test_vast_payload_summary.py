"""Gate visibility falsifiers: the payload ends with a compact FINAL SUMMARY block.

Measured 2026-10-26 (run 40241597724): the job log of a real gate run showed only the first ~25 KB
of the box output, so the stage results that decide the verdict were invisible. The payload now
prints, as its last lines, one bounded block with every stage's name, rc, duration, its suite
rows and the first failing line of each failed stage.
"""

import contextlib
import importlib.util
import io
import json
import re
import tempfile
import os
import unittest
from pathlib import Path
from unittest.mock import patch

import test_vast_payload_v7 as v7

BEGIN = "KF_FINAL_SUMMARY_BEGIN"
END = "KF_FINAL_SUMMARY_END"


class SummaryTests(unittest.TestCase):
    _main_fixture = v7.V7Tests._main_fixture

    def setUp(self):
        spec = importlib.util.spec_from_file_location(
            "payload_summary", Path(__file__).with_name("vast_payload.py")
        )
        self.m = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.m)

    def stages_run(self, specs, result=None):
        """Run execute_stages over fake stages; specs: name, rc, text, rows or fail."""
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            stages, texts, codes = [], {}, {}
            for spec in specs:
                name = spec["name"]
                out = root / (name + ".run")
                progress = root / (name + ".progress")
                texts[name] = (spec.get("text", ""), spec.get("progress_text", ""))
                codes[name] = spec.get("rc", 0)

                def grade(rc, spec=spec):
                    if spec.get("fail"):
                        raise self.m.EvidenceError(spec["fail"], spec.get("partial", []))
                    return spec.get("rows", [self.m.row(spec["name"], 1)])

                stages.append(
                    dict(
                        name=name,
                        command=["true"],
                        output=out,
                        progress=[progress],
                        grade=grade,
                    )
                )

            def runner(command, soft, output, progress):
                name = Path(output).name.rsplit(".", 1)[0]
                Path(output).write_text(texts[name][0])
                progress[0].write_text(texts[name][1])
                return codes[name]

            result = result or {"verdict": "FAIL", "suites": [], "hardware": {}}
            buffer = io.StringIO()
            with contextlib.redirect_stdout(buffer):
                rc = self.m.execute_stages(stages, 10**13, result, root / "r.json", runner=runner)
            return rc, buffer.getvalue(), result

    def block(self, output):
        lines = output.rstrip("\n").split("\n")
        self.assertEqual(lines[-1], END, "the summary must be the last line printed")
        start = max(i for i, s in enumerate(lines) if s.startswith(BEGIN))
        return lines[start:]

    FAILING = [
        dict(name="provision", rows=[dict(name="provision", **{"pass": 1, "fail": 0, "skip": 0})]),
        dict(
            name="cuda_host",
            rc=1,
            text="CUDA_LADDER_STARTED=now\nnoise\nBOX_RC=0\nDRIVER_RC=6\nlater noise\n",
            fail="cuda_host",
        ),
        dict(name="merge_bar"),
    ]

    def test_summary_is_the_last_block_on_failure(self):
        rc, out, _ = self.stages_run(self.FAILING)
        self.assertEqual(rc, 1)
        block = self.block(out)
        self.assertTrue(block[0].startswith(BEGIN + " verdict=FAIL"))
        self.assertLess(out.index("KF_PAYLOAD_FAIL"), out.index(BEGIN))

    def test_summary_is_the_last_block_on_pass(self):
        rc, out, _ = self.stages_run([dict(name="a"), dict(name="b")])
        self.assertEqual(rc, 0)
        block = self.block(out)
        self.assertTrue(block[0].startswith(BEGIN + " verdict=PASS"))
        self.assertEqual([s for s in block if s.startswith("first_fail")], [])

    def test_summary_names_every_stage_with_rc_and_duration(self):
        _, out, _ = self.stages_run(self.FAILING)
        block = self.block(out)
        stage = [s for s in block if s.startswith("stage ")]
        self.assertRegex(stage[0], r"^stage name=provision rc=0 duration_ms=\d+ result=PASS$")
        self.assertRegex(stage[1], r"^stage name=cuda_host rc=1 duration_ms=\d+ result=FAIL")
        self.assertRegex(stage[2], r"^stage name=merge_bar result=NOT_RUN$")

    def test_summary_lists_suite_counts(self):
        rows = [self.m.row("kf_tests", 5, 0, 2)]
        specs = [dict(name="a", rows=rows), dict(name="b", rc=1, fail="gates")]
        _, out, _ = self.stages_run(specs)
        block = self.block(out)
        self.assertIn("suite name=kf_tests pass=5 fail=0 skip=2", block)
        self.assertIn("suite name=gates pass=0 fail=1 skip=0", block)

    def test_summary_gives_the_first_failing_line_of_each_failed_stage(self):
        _, out, _ = self.stages_run(self.FAILING)
        first = [s for s in self.block(out) if s.startswith("first_fail")]
        self.assertEqual(first, ["first_fail stage=cuda_host line=DRIVER_RC=6"])

    def test_first_failing_line_is_found_in_the_progress_file_too(self):
        specs = [
            dict(
                name="provision",
                rc=1,
                text="[xid-watch] tag=x\nSTART\n",
                progress_text="START\nBOX_RC=0\nerror[E0432]: unresolved import `x`\nTREE_RC=1\n",
                fail="provision",
            )
        ]
        _, out, _ = self.stages_run(specs)
        first = [s for s in self.block(out) if s.startswith("first_fail")]
        self.assertEqual(
            first, ["first_fail stage=provision line=error[E0432]: unresolved import `x`"]
        )

    def test_a_failed_stage_without_a_red_line_still_names_its_last_line(self):
        specs = [
            dict(name="provision", rc=1, text="one\nthe last thing printed\n", fail="provision")
        ]
        _, out, _ = self.stages_run(specs)
        first = [s for s in self.block(out) if s.startswith("first_fail")]
        self.assertEqual(len(first), 1)
        self.assertIn("the last thing printed", first[0])

    def test_step_cause_reads_the_log_of_the_failing_step(self):
        with tempfile.TemporaryDirectory() as d:
            (Path(d) / "driver.log").write_text("l1\nl2\n\nl3\nPURGE FAILED (rc=7)\n")
            self.assertEqual(self.m.step_cause(d, "DRIVER_RC=6"), "l2 | l3 | PURGE FAILED (rc=7)")
            self.assertEqual(self.m.step_cause(d, "TREE_RC=1"), "")
            self.assertEqual(self.m.step_cause(d, "NOT_READY driver=575.51.03"), "")

    def test_non_ascii_never_reaches_the_summary(self):
        specs = [dict(name="bad", rc=1, text="\u202eerror: boom\u200b \u00e9\n", fail="bad")]
        _, out, _ = self.stages_run(specs)
        block = "\n".join(self.block(out))
        self.assertTrue(all(" " <= c <= "~" or c == "\n" for c in block), block)
        self.assertIn("error: boom", block)

    def test_summary_keeps_the_failure_when_it_must_drop_lines(self):
        specs = [
            dict(name="stage%03d" % i, rows=[self.m.row("suite%03d" % i, 1)]) for i in range(90)
        ]
        specs.append(dict(name="bad", rc=1, text="DRIVER_RC=6\n", fail="bad"))
        _, out, _ = self.stages_run(specs)
        block = self.block(out)
        self.assertLessEqual(len(("\n".join(block) + "\n").encode()), 4096)
        self.assertIn("first_fail stage=bad line=DRIVER_RC=6", block)
        self.assertIn("suite name=bad pass=0 fail=1 skip=0", block)
        self.assertTrue(any(s.startswith("truncated lines=") for s in block))

    def test_provision_failure_summary_carries_the_step_log_tail(self):
        with tempfile.TemporaryDirectory() as d:
            (Path(d) / "driver.log").write_text("x\ny\na\nb\nPURGE FAILED (rc=6)\n")
            specs = [
                dict(
                    name="provision",
                    rc=1,
                    progress_text="BOX_RC=0\nDRIVER_RC=6\n",
                    fail="provision",
                )
            ]
            with patch.object(self.m, "PROV_DIR", d):
                _, out, _ = self.stages_run(specs)
        block = self.block(out)
        self.assertIn("first_fail stage=provision line=DRIVER_RC=6", block)
        self.assertIn("cause stage=provision log_tail=a | b | PURGE FAILED (rc=6)", block)

    def test_timeout_is_marked(self):
        specs = [dict(name="merge_bar", rc=124, text="start\n", fail="merge_bar")]
        _, out, _ = self.stages_run(specs)
        stage = [s for s in self.block(out) if s.startswith("stage ")][0]
        self.assertRegex(stage, r"^stage name=merge_bar rc=124 duration_ms=\d+ result=TIMEOUT")

    def test_summary_is_bounded_to_4_kib(self):
        specs = [dict(name="s%02d" % i, rows=[self.m.row("r%02d" % i, 1)]) for i in range(80)]
        specs.append(dict(name="bad", rc=1, text="DRIVER_RC=6 " + "x " * 100000, fail="bad"))
        _, out, _ = self.stages_run(specs)
        block = self.block(out)
        text = "\n".join(block) + "\n"
        self.assertLessEqual(len(text.encode()), 4096)
        self.assertEqual(block[-1], END)
        self.assertTrue(block[0].startswith(BEGIN + " verdict=FAIL"))
        self.assertTrue(any(s.startswith("first_fail stage=bad") for s in block))

    def test_a_very_long_failing_line_is_cut(self):
        specs = [dict(name="bad", rc=1, text="DRIVER_RC=6 " + "y " * 5000 + "\n", fail="bad")]
        _, out, _ = self.stages_run(specs)
        line = [s for s in self.block(out) if s.startswith("first_fail")][0]
        self.assertLess(len(line), 400)

    def test_control_characters_never_reach_the_summary(self):
        specs = [dict(name="bad", rc=1, text="\x1b[31mDRIVER_RC=6\x1b[0m\x00\r\n", fail="bad")]
        _, out, _ = self.stages_run(specs)
        block = "\n".join(self.block(out))
        self.assertNotIn("\x1b", block)
        self.assertNotIn("\x00", block)
        self.assertIn("DRIVER_RC=6", block)

    def test_summary_is_printed_when_preflight_fails(self):
        with contextlib.redirect_stdout(io.StringIO()) as captured:
            rc, result, calls = self._main_fixture(free=100)
        self.assertEqual(rc, 1)
        block = self.block(captured.getvalue())
        self.assertIn("suite name=preflight pass=0 fail=1 skip=0", block)
        self.assertTrue(block[0].startswith(BEGIN + " verdict=FAIL"))

    def test_summary_is_printed_when_kvm_is_missing(self):
        with contextlib.redirect_stdout(io.StringIO()) as captured:
            rc, result, calls = self._main_fixture(kvm=False)
        self.assertEqual(rc, 1)
        block = self.block(captured.getvalue())
        self.assertTrue(block[0].startswith(BEGIN + " verdict=FAIL stages=0"))
        self.assertIn("suite name=preflight pass=0 fail=1 skip=0", block)

    def test_whole_adapter_prints_the_summary_last(self):
        with contextlib.redirect_stdout(io.StringIO()) as captured:
            rc, result, calls = self._main_fixture(failure="merge_bar")
        self.assertEqual(rc, 1)
        block = self.block(captured.getvalue())
        names = [re.match(r"stage name=(\w+)", s)[1] for s in block if s.startswith("stage ")]
        self.assertEqual(names, ["provision", "cuda_host", "merge_bar", "cuda_guest"])
        self.assertIn("stage name=cuda_guest result=NOT_RUN", block)
        self.assertRegex(
            [s for s in block if s.startswith("stage name=merge_bar")][0], r"rc=124 .*TIMEOUT"
        )

    def test_whole_adapter_pass_summary(self):
        with contextlib.redirect_stdout(io.StringIO()) as captured:
            rc, result, calls = self._main_fixture()
        self.assertEqual(rc, 0)
        block = self.block(captured.getvalue())
        self.assertTrue(block[0].startswith(BEGIN + " verdict=PASS"))
        self.assertEqual(len([s for s in block if s.startswith("stage ")]), 4)
        json.dumps(result)


class PayloadQuietEnvironment(unittest.TestCase):
    """The payload quiets cargo's own chatter only; lint level and RUSTFLAGS are not touched."""

    def seen(self):
        seen = {}
        helper = v7.V7Tests("test_whole_adapter_uses_existing_flows_in_order")
        helper.setUp()
        with patch.dict(os.environ, {}, clear=False):
            os.environ.pop("RUSTFLAGS", None)
            os.environ.pop("CARGO_ENCODED_RUSTFLAGS", None)
            helper._main_fixture(probe=lambda env: seen.update(env))
        return seen

    def test_payload_exports_quiet_cargo_settings(self):
        seen = self.seen()
        self.assertEqual(seen.get("CARGO_TERM_QUIET"), "true")
        self.assertEqual(seen.get("CARGO_TERM_COLOR"), "never")
        self.assertEqual(seen.get("CARGO_TERM_PROGRESS_WHEN"), "never")
        self.assertNotIn("RUSTFLAGS", seen)

    def test_cargo_quiet_is_the_exact_value_cargo_reads(self):
        self.assertEqual(self.seen().get("CARGO_TERM_QUIET"), "true")

    def test_cargo_output_is_plain(self):
        seen = self.seen()
        self.assertEqual(
            (seen.get("CARGO_TERM_COLOR"), seen.get("CARGO_TERM_PROGRESS_WHEN")), ("never", "never")
        )

    def test_payload_leaves_rustflags_alone(self):
        seen = self.seen()
        self.assertNotIn("RUSTFLAGS", seen)
        self.assertNotIn("CARGO_ENCODED_RUSTFLAGS", seen)
        self.assertNotIn("CARGO_BUILD_RUSTFLAGS", seen)


if __name__ == "__main__":
    unittest.main()
