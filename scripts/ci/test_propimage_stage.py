#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of scripts/ci/propimage_stage.py (2026-11-19): the generic plan against synthetic tables (rows, boots, the harness argv, the arm),
and the EQUIVALENCE of the plan of the project's entries (imga, imgb, imgc) with the committed golden and with what the hardcoded stages compute.
`python3 test_propimage_stage.py --write-golden` rewrites the golden from the hardcoded stages (review the diff: it must be empty)."""
import copy
import importlib.util
import io
import json
import sys
import tempfile
import textwrap
import unittest
from contextlib import redirect_stdout
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))
import propimage_config as pc  # noqa: E402
import propimage_stage as ps  # noqa: E402

GOLDEN = HERE / "fixtures/propimage_stage_golden.json"


# ---------------------------------------------------------------- synthetic bundles
TABLE = textwrap.dedent('''
    CHECKS = {
        "r1": {"timeout": 100},
        "r2": {"timeout": 100, "lane": "gate"},
        "r3": {"timeout": 100, "lane": "discovery"},
        "r4": {"timeout": 100, "disabled": "needs a licence that is not on the image"},
        "r5": {"timeout": 100},
    }
''')
LINUX_TABLE = textwrap.dedent('''
    PREAMBLE = "info"
    ROW_ORDER = ("info", "k1", "k2")
    ROWS = {"info": ("info.m", 10), "k1": ("k1.m", 20), "k2": ("k2.m", 30)}
    def plan_boots(rows, isolation):
        ordered = [PREAMBLE] + [r for r in rows if r != PREAMBLE]
        return [[r] for r in ordered] if isolation == "row" else [ordered]
''')


def config(**run):
    win = {"rows_file": "t/table.py", "pins_file": "scripts/ci/pins.json"}
    win.update(run)
    return {"schema": pc.SCHEMA, "bundles": {
        "w": {"os": "windows", "r2": "k/w.zip", "size_bytes": 10, "apps": ["a"], "run": win},
        "l": {"os": "linux", "r2": "k/l.zip", "size_bytes": 10, "apps": ["m"], "run": {"rows_file": "t/lin.py", "pins_file": "scripts/ci/p.json", "isolation": "row"}},
        "f": {"os": "linux", "r2": "k/f.zip", "size_bytes": 10, "apps": ["n"]}}}


class Root:
    def __enter__(self):
        self.tmp = tempfile.TemporaryDirectory()
        base = Path(self.tmp.name)
        (base / "t").mkdir()
        (base / "t/table.py").write_text(TABLE)
        (base / "t/lin.py").write_text(LINUX_TABLE)
        return base

    def __exit__(self, *exc):
        self.tmp.cleanup()


def reason(fn, *args, **kwargs):
    try:
        fn(*args, **kwargs)
    except (ps.StageError, pc.ConfigError) as exc:
        return exc.reason
    return None


def paths(rows, **extra):
    return dict(ps.PATHS, apps=rows, **extra)


class Boots(unittest.TestCase):
    T = {"a": 1000, "b": 1000, "c": 1000, "d": 700}

    def test_rows_share_a_boot_while_the_budget_holds(self):
        self.assertEqual(ps.plan_boots(self.T, ["a", "b"], budget=2000), [["a", "b"]])

    def test_a_row_that_would_exceed_the_budget_starts_the_next_boot(self):
        self.assertEqual(ps.plan_boots(self.T, ["a", "b", "c"], budget=2000), [["a", "b"], ["c"]])

    def test_a_sum_of_exactly_the_budget_still_fits(self):
        self.assertEqual(ps.plan_boots(self.T, ["a", "b"], budget=2000), [["a", "b"]])
        self.assertEqual(ps.plan_boots(self.T, ["a", "b"], budget=1999), [["a"], ["b"]])

    def test_the_default_budget_is_the_first_stages(self):
        self.assertEqual(ps.BOOT_BUDGET_S, 2700)
        self.assertEqual(ps.plan_boots({"a": 1350, "b": 1350, "c": 1}, ["a", "b", "c"]), [["a", "b"], ["c"]])
        self.assertEqual(ps.plan_boots({"a": 1350, "b": 1351}, ["a", "b"]), [["a"], ["b"]])

    def test_the_order_of_the_rows_is_kept(self):
        self.assertEqual(ps.plan_boots(self.T, ["d", "c", "b", "a"], budget=10**6), [["d", "c", "b", "a"]])

    def test_a_discovery_row_boots_alone_between_the_others(self):
        self.assertEqual(ps.plan_boots(self.T, ["a", "b", "c", "d"], budget=10**6, lanes={"b": "discovery"}), [["a"], ["b"], ["c", "d"]])

    def test_a_discovery_row_first_or_last_boots_alone(self):
        self.assertEqual(ps.plan_boots(self.T, ["a", "b"], budget=10**6, lanes={"a": "discovery"}), [["a"], ["b"]])
        self.assertEqual(ps.plan_boots(self.T, ["a", "b"], budget=10**6, lanes={"b": "discovery"}), [["a"], ["b"]])

    def test_two_discovery_rows_in_a_row_each_boot_alone(self):
        self.assertEqual(ps.plan_boots(self.T, ["a", "b", "c"], budget=10**6, lanes={"a": "discovery", "b": "discovery"}), [["a"], ["b"], ["c"]])

    def test_a_gate_lane_row_is_not_solo(self):
        self.assertEqual(ps.plan_boots(self.T, ["a", "b"], budget=10**6, lanes={"a": "gate", "b": "gate"}), [["a", "b"]])

    def test_a_named_solo_row_boots_alone_in_its_place(self):
        self.assertEqual(ps.plan_boots(self.T, ["a", "b", "c"], budget=10**6, solo=["b"]), [["a"], ["b"], ["c"]])

    def test_a_solo_row_at_the_end_is_last_and_alone(self):
        self.assertEqual(ps.plan_boots(self.T, ["a", "b", "d"], budget=10**6, solo=("d",)), [["a", "b"], ["d"]])

    def test_the_solo_row_does_not_reset_the_running_total_of_the_boot_after_it(self):
        self.assertEqual(ps.plan_boots(self.T, ["a", "d", "b", "c"], budget=2000, solo=("d",)), [["a"], ["d"], ["b", "c"]])

    def test_a_discovery_row_resets_the_running_total(self):
        self.assertEqual(ps.plan_boots(self.T, ["a", "b", "c", "d"], budget=1700, lanes={"b": "discovery"}), [["a"], ["b"], ["c", "d"]])

    def test_no_rows_no_boots(self):
        self.assertEqual(ps.plan_boots({}, []), [])


class Command(unittest.TestCase):
    RUN = {"rows_file": "x.py", "pins_file": "y.json"}

    def cmd(self, run=None, rows=("r1",), **kw):
        return ps.boot_command(run if run is not None else self.RUN, paths(list(rows), work="/w", out="/o"), **kw)

    def test_the_fixed_words(self):
        self.assertEqual(self.cmd(), ["<python>", "-B", "-I", "/p/propimage_check.py", "--dir", "/p/unpacked", "--work", "/w", "--out", "/o", "--qemu", "/p/qemu",
                                      "--flags", "/p/flags.env", "--broker", "/p/broker", "--gop-efi", "/p/gop.efi", "--display", "broker", "--apps", "r1"])

    def test_the_interpreter_is_the_callers(self):
        self.assertEqual(self.cmd(exe="/usr/bin/python3")[0], "/usr/bin/python3")

    def test_apps_are_a_comma_list_in_boot_order(self):
        self.assertEqual(self.cmd(rows=("r2", "r1"))[-1], "r2,r1")

    def test_before_apps_come_between_the_display_and_the_apps(self):
        cmd = self.cmd(dict(self.RUN, harness_args={"before_apps": ["--app-set", "b"]}))
        self.assertEqual(cmd[cmd.index("broker") + 1:cmd.index("--apps")], ["--app-set", "b"])

    def test_the_data_volume_follows_before_apps_and_precedes_the_apps(self):
        cmd = self.cmd(dict(self.RUN, data_iso=True, harness_args={"before_apps": ["--app-set", "b"]}))
        i = cmd.index("--data-iso")
        self.assertEqual((cmd[i - 2:i], cmd[i + 1], cmd[i + 2]), (["--app-set", "b"], "/p/data.iso", "--apps"))

    def test_no_data_volume_when_the_entry_has_none(self):
        self.assertNotIn("--data-iso", self.cmd(dict(self.RUN, data_iso=False)))
        self.assertNotIn("--data-iso", self.cmd())

    def test_after_apps_follow_the_app_list_then_ram_then_audio(self):
        cmd = self.cmd(dict(self.RUN, guest_ram_mb=16384, audio=True, harness_args={"after_apps": ["--fb-mb", "8192"]}))
        self.assertEqual(cmd[cmd.index("--apps") + 1:], ["r1", "--fb-mb", "8192", "--ram-mb", "16384", "--audio"])

    def test_zero_ram_and_no_audio_add_nothing(self):
        cmd = self.cmd(dict(self.RUN, guest_ram_mb=0, audio=False))
        self.assertNotIn("--ram-mb", cmd)
        self.assertNotIn("--audio", cmd)

    def test_per_row_words_only_for_rows_on_the_boot_in_boot_order(self):
        run = dict(self.RUN, harness_args={"per_row": {"r1": ["--one"], "r2": ["--two", "x"], "r3": ["--three"]}})
        self.assertEqual(self.cmd(run, rows=("r2", "r1"))[-4:], ["r2,r1", "--two", "x", "--one"])
        self.assertNotIn("--three", self.cmd(run, rows=("r2", "r1")))

    def test_per_row_words_follow_the_boot_row_order(self):
        run = dict(self.RUN, harness_args={"per_row": {"r1": ["--one"], "r2": ["--two"], "r3": ["--three"]}})
        self.assertEqual(self.cmd(run, rows=("r1", "r2", "r3"))[-4:], ["r1,r2,r3", "--one", "--two", "--three"])
        self.assertEqual(self.cmd(run, rows=("r3", "r1"))[-3:], ["r3,r1", "--three", "--one"][-3:])

    def test_per_row_words_come_after_after_apps_and_audio(self):
        run = dict(self.RUN, audio=True, harness_args={"after_apps": ["--fb-mb", "1"], "per_row": {"r1": ["--one"]}})
        cmd = self.cmd(run)
        self.assertEqual(cmd[cmd.index("--apps") + 1:], ["r1", "--fb-mb", "1", "--audio", "--one"])

    def test_the_default_arm_adds_nothing(self):
        self.assertEqual(self.cmd(arm="kf3"), self.cmd())
        self.assertNotIn("--platform", self.cmd())

    def test_the_vfio_arm_adds_the_platform_and_the_gpu_last(self):
        run = dict(self.RUN, audio=True, harness_args={"per_row": {"r1": ["--one"]}})
        self.assertEqual(self.cmd(run, arm="vfio", vfio_gpu="0000:01:00.0")[-5:], ["--one", "--platform", "vfio", "--vfio-gpu", "0000:01:00.0"])

    def test_vfio_changes_only_the_tail(self):
        self.assertEqual(self.cmd(arm="vfio", vfio_gpu="0000:41:00.0")[:-4], self.cmd())

    def test_an_unknown_arm_is_refused(self):
        self.assertEqual(reason(self.cmd, arm="tcg"), "ARM:tcg")

    def test_vfio_needs_a_pci_address_of_function_zero(self):
        for bad in (None, "", "01:00.0", "0000:01:00.1", "0000:01:00.0 ; rm", "0000:0G:00.0"):
            self.assertEqual(reason(self.cmd, arm="vfio", vfio_gpu=bad), "VFIO_GPU", bad)

    def test_a_gpu_address_without_the_vfio_arm_is_refused(self):
        self.assertEqual(reason(self.cmd, arm="kf3", vfio_gpu="0000:01:00.0"), "VFIO_GPU_WITHOUT_VFIO")


class Rows(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        class T:
            pass
        cls.table = T()
        cls.table.CHECKS = {"r1": {"timeout": 1}, "r2": {"timeout": 1}, "r3": {"timeout": 1, "disabled": "off " + "x" * 200}, "r4": {"timeout": 1}}

    def test_without_rows_the_enabled_rows_of_the_table_in_table_order(self):
        self.assertEqual(ps.windows_rows({}, self.table)[0], ("r1", "r2", "r4"))

    def test_the_entry_rows_win_and_keep_their_order(self):
        self.assertEqual(ps.windows_rows({"rows": ["r4", "r1"]}, self.table)[0], ("r4", "r1"))

    def test_entry_rows_the_table_lacks_are_refused(self):
        self.assertEqual(reason(ps.windows_rows, {"rows": ["r1", "zz"]}, self.table), "ROW_UNKNOWN")

    def test_only_selects_rows_in_table_order(self):
        self.assertEqual(ps.windows_rows({"rows": ["r1", "r2"]}, self.table, only=["r4", "r2"])[0], ("r2", "r4"))

    def test_only_may_name_a_disabled_row_and_then_it_is_not_reported_as_not_run(self):
        order, not_run = ps.windows_rows({}, self.table, only=["r3", "r1"])
        self.assertEqual((order, not_run), (("r1", "r3"), {}))

    def test_only_with_an_unknown_row_is_refused(self):
        self.assertEqual(reason(ps.windows_rows, {}, self.table, only=["r1", "nope"]), "ROW_UNKNOWN")

    def test_an_empty_only_is_refused(self):
        self.assertEqual(reason(ps.windows_rows, {}, self.table, only=[]), "ROW_UNKNOWN")

    def test_disabled_rows_are_reported_with_their_reason_trimmed_to_160(self):
        order, not_run = ps.windows_rows({}, self.table)
        self.assertEqual(list(not_run), ["r3"])
        self.assertEqual(not_run["r3"], ("off " + "x" * 200)[:160])

    def test_a_reason_of_161_characters_loses_its_last_one(self):
        class T:
            CHECKS = {"r1": {"timeout": 1}, "r2": {"timeout": 1, "disabled": "y" * 161}}
        self.assertEqual(ps.windows_rows({}, T)[1]["r2"], "y" * 160)

    def test_a_table_with_nothing_enabled_is_refused(self):
        class T:
            CHECKS = {"r1": {"timeout": 1, "disabled": "off"}}
        self.assertEqual(reason(ps.windows_rows, {}, T), "NO_ROWS")


class Plan(unittest.TestCase):
    def test_a_windows_plan_has_rows_lanes_timeouts_boots_and_commands(self):
        with Root() as base:
            p = ps.plan(config(), "w", base)
        self.assertEqual(p["rows"], ["r1", "r2", "r3", "r5"])
        self.assertEqual(p["lanes"], {"r1": "gate", "r2": "gate", "r3": "discovery", "r5": "gate"})
        self.assertEqual(p["timeouts"], {"r1": 100, "r2": 100, "r3": 100, "r5": 100})
        self.assertEqual(p["not_run"], {"r4": "needs a licence that is not on the image"})
        self.assertEqual(p["boots"], [["r1", "r2"], ["r3"], ["r5"]])
        self.assertEqual([c[c.index("--apps") + 1] for c in p["commands"]], ["r1,r2", "r3", "r5"])
        self.assertEqual((p["bundle"], p["os"], p["arm"]), ("w", "windows", "kf3"))

    def test_the_commands_of_a_plan_name_their_own_boot_directories(self):
        with Root() as base:
            p = ps.plan(config(), "w", base)
        self.assertEqual([c[c.index("--work") + 1] for c in p["commands"]], ["/p/work/boot1", "/p/work/boot2", "/p/work/boot3"])
        self.assertEqual([c[c.index("--out") + 1] for c in p["commands"]], ["/p/out/boot1", "/p/out/boot2", "/p/out/boot3"])

    def test_solo_rows_of_the_entry_split_the_boots(self):
        with Root() as base:
            p = ps.plan(config(rows=["r1", "r2", "r5"], solo_rows=["r2"]), "w", base)
        self.assertEqual(p["boots"], [["r1"], ["r2"], ["r5"]])

    def test_a_plan_can_be_restricted_to_rows(self):
        with Root() as base:
            p = ps.plan(config(), "w", base, only=["r5", "r1"])
        self.assertEqual(p["rows"], ["r1", "r5"])

    def test_the_arm_is_in_the_plan_and_in_every_command(self):
        with Root() as base:
            p = ps.plan(config(), "w", base, arm="vfio", vfio_gpu="0000:01:00.0")
        self.assertEqual(p["arm"], "vfio")
        self.assertTrue(all(c[-4:] == ["--platform", "vfio", "--vfio-gpu", "0000:01:00.0"] for c in p["commands"]))

    def test_the_default_arm_is_the_product(self):
        with Root() as base:
            self.assertEqual(ps.plan(config(), "w", base)["arm"], "kf3")
            self.assertEqual(ps.plan(config(), "w", base), ps.plan(config(), "w", base, arm="kf3"))

    def test_a_linux_plan_has_the_rows_the_timeouts_and_the_boots(self):
        with Root() as base:
            p = ps.plan(config(), "l", base)
        self.assertEqual(p, {"bundle": "l", "os": "linux", "arm": "kf3", "rows": ["k1", "k2"], "timeouts": {"k1": 20, "k2": 30}, "isolation": "row",
                             "boots": [["info"], ["k1"], ["k2"]]})

    def test_a_linux_plan_can_share_one_boot(self):
        with Root() as base:
            self.assertEqual(ps.plan(config(), "l", base, isolation="shared")["boots"], [["info", "k1", "k2"]])

    def test_a_linux_plan_can_be_restricted_to_rows_in_the_modules_order(self):
        with Root() as base:
            self.assertEqual(ps.plan(config(), "l", base, only=["k2"])["rows"], ["k2"])
            self.assertEqual(ps.plan(config(), "l", base, only=["k2", "k1"])["rows"], ["k1", "k2"])

    def test_a_linux_plan_can_be_restricted_to_rows_in_the_modules_order_twice(self):
        with Root() as base:
            self.assertEqual(ps.plan(config(), "l", base, only=["k2", "k1"])["boots"], [["info"], ["k1"], ["k2"]])

    def test_a_linux_row_the_module_lacks_is_refused(self):
        with Root() as base:
            self.assertEqual(reason(ps.plan, config(), "l", base, only=["zz"]), "ROW_UNKNOWN")

    def test_an_empty_linux_row_request_is_refused(self):
        with Root() as base:
            self.assertEqual(reason(ps.plan, config(), "l", base, only=[]), "ROW_UNKNOWN")

    def test_the_preamble_is_never_a_requested_row(self):
        with Root() as base:
            self.assertEqual(ps.plan(config(), "l", base, only=["info", "k1"])["rows"], ["k1"])

    def test_a_request_for_only_the_preamble_has_no_rows(self):
        with Root() as base:
            self.assertEqual(reason(ps.plan, config(), "l", base, only=["info"]), "NO_ROWS")

    def test_a_bad_isolation_is_refused(self):
        with Root() as base:
            self.assertEqual(reason(ps.plan, config(), "l", base, isolation="vm"), "ISOLATION")

    def test_an_isolation_in_the_wrong_case_is_refused(self):
        with Root() as base:
            self.assertEqual(reason(ps.plan, config(), "l", base, isolation="Row"), "ISOLATION")

    def test_an_entry_row_the_table_lacks_refuses_the_plan(self):
        with Root() as base:
            self.assertEqual(reason(ps.plan, config(rows=["r1", "zz"]), "w", base), "ROW_UNKNOWN")

    def test_an_entry_row_that_is_disabled_is_planned_and_not_reported_as_not_run(self):
        with Root() as base:
            p = ps.plan(config(rows=["r1", "r4"]), "w", base)
        self.assertEqual((p["rows"], p["not_run"]), (["r1", "r4"], {}))

    def test_zero_guest_ram_adds_no_ram_words_to_a_plan(self):
        with Root() as base:
            self.assertTrue(all("--ram-mb" not in c for c in ps.plan(config(guest_ram_mb=0), "w", base)["commands"]))
            self.assertTrue(all(c[c.index("--ram-mb") + 1] == "4096" for c in ps.plan(config(guest_ram_mb=4096), "w", base)["commands"]))

    def test_a_plan_refuses_a_gpu_address_on_the_default_arm(self):
        with Root() as base:
            self.assertEqual(reason(ps.plan, config(), "w", base, vfio_gpu="0000:01:00.0"), "VFIO_GPU_WITHOUT_VFIO")

    def test_a_plan_refuses_a_gpu_address_with_trailing_text(self):
        with Root() as base:
            self.assertEqual(reason(ps.plan, config(), "w", base, arm="vfio", vfio_gpu="0000:01:00.0 x"), "VFIO_GPU")

    def test_a_linux_table_with_only_a_preamble_has_no_plan(self):
        with Root() as base:
            (base / "t/lin.py").write_text(LINUX_TABLE.replace('("info", "k1", "k2")', '("info",)'))
            self.assertEqual(reason(ps.plan, config(), "l", base), "NO_ROWS")

    def test_a_plan_refuses_function_one_of_the_gpu(self):
        with Root() as base:
            self.assertEqual(reason(ps.plan, config(), "w", base, arm="vfio", vfio_gpu="0000:01:00.1"), "VFIO_GPU")

    def test_a_plan_refuses_a_missing_schema(self):
        with Root() as base:
            self.assertEqual(reason(ps.plan, {}, "w", base), "SCHEMA")

    def test_a_plan_of_a_table_with_nothing_enabled_is_refused(self):
        with Root() as base:
            (base / "t/table.py").write_text('CHECKS = {"r1": {"timeout": 5, "disabled": "off"}}\n')
            self.assertEqual(reason(ps.plan, config(), "w", base), "NO_ROWS")

    def test_isolation_is_not_a_windows_setting(self):
        with Root() as base:
            self.assertEqual(reason(ps.plan, config(), "w", base, isolation="row"), "ISOLATION_ON_WINDOWS")

    def test_a_fetch_only_bundle_has_no_plan(self):
        with Root() as base:
            self.assertEqual(reason(ps.plan, config(), "f", base), "NO_RUN:f")

    def test_an_unknown_bundle_is_refused_by_name(self):
        with Root() as base:
            self.assertEqual(reason(ps.plan, config(), "nope", base), "UNKNOWN_BUNDLE:nope")

    def test_an_invalid_configuration_is_refused_first(self):
        d = config()
        d["bundles"]["w"]["os"] = "bsd"
        with Root() as base:
            self.assertEqual(reason(ps.plan, d, "w", base), "BUNDLE_OS:w")

    def test_the_vfio_arm_is_for_windows_bundles(self):
        with Root() as base:
            self.assertEqual(reason(ps.plan, config(), "l", base, arm="vfio", vfio_gpu="0000:01:00.0"), "ARM_NOT_SUPPORTED:linux")

    def test_the_arm_is_checked_before_the_rows_are_read(self):
        with Root() as base:
            self.assertEqual(reason(ps.plan, config(), "w", base, arm="vfio"), "VFIO_GPU")
            self.assertEqual(reason(ps.plan, config(), "w", base, arm="x"), "ARM:x")


class Cli(unittest.TestCase):
    def run_cli(self, *argv):
        out = io.StringIO()
        with redirect_stdout(out):
            rc = ps.main(list(argv))
        return rc, out.getvalue()

    def test_plan_prints_json_for_a_committed_bundle(self):
        rc, out = self.run_cli("plan", "--bundle", "imga")
        self.assertEqual(rc, 0)
        self.assertEqual(json.loads(out)["boots"][-1], ["wukong"])

    def test_plan_of_a_fetch_only_bundle_exits_2(self):
        rc, out = self.run_cli("plan", "--bundle", "imgd")
        self.assertEqual((rc, out.strip()), (2, "KF_PROPIMAGE_STAGE_REFUSED NO_RUN:imgd"))

    def test_plan_with_the_vfio_arm(self):
        rc, out = self.run_cli("plan", "--bundle", "imgb", "--arm", "vfio", "--vfio-gpu", "0000:01:00.0")
        self.assertEqual(rc, 0)
        doc = json.loads(out)
        self.assertEqual((doc["arm"], doc["commands"][0][-4:]), ("vfio", ["--platform", "vfio", "--vfio-gpu", "0000:01:00.0"]))

    def test_plan_of_an_unknown_bundle_exits_2(self):
        self.assertEqual(self.run_cli("plan", "--bundle", "imgz"), (2, "KF_PROPIMAGE_STAGE_REFUSED UNKNOWN_BUNDLE:imgz\n"))

    def test_plan_of_a_linux_bundle_with_the_vfio_arm_exits_2(self):
        self.assertEqual(self.run_cli("plan", "--bundle", "imgc", "--arm", "vfio", "--vfio-gpu", "0000:01:00.0"), (2, "KF_PROPIMAGE_STAGE_REFUSED ARM_NOT_SUPPORTED:linux\n"))

    def test_plan_of_a_windows_bundle_with_an_isolation_exits_2(self):
        self.assertEqual(self.run_cli("plan", "--bundle", "imga", "--isolation", "row"), (2, "KF_PROPIMAGE_STAGE_REFUSED ISOLATION_ON_WINDOWS\n"))

    def test_plan_with_the_vfio_arm_and_apps(self):
        rc, out = self.run_cli("plan", "--bundle", "imgb", "--arm", "vfio", "--vfio-gpu", "0000:01:00.0", "--apps", "acrobat")
        self.assertEqual((rc, json.loads(out)["boots"]), (0, [["acrobat"]]))

    def test_vfio_without_an_address_exits_2(self):
        self.assertEqual(self.run_cli("plan", "--bundle", "imgb", "--arm", "vfio"), (2, "KF_PROPIMAGE_STAGE_REFUSED VFIO_GPU\n"))

    def test_apps_restricts_the_rows(self):
        rc, out = self.run_cli("plan", "--bundle", "imgb", "--apps", "gpudiag,acrobat")
        self.assertEqual((rc, json.loads(out)["rows"]), (0, ["gpudiag", "acrobat"]))

    def test_an_unreadable_config_exits_2(self):
        self.assertEqual(self.run_cli("plan", "--bundle", "x", "--config", "/nonexistent/c.json"), (2, "KF_PROPIMAGE_STAGE_UNREADABLE FileNotFoundError\n"))

    def test_a_bad_config_exits_2(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "c.json"
            path.write_text("{}")
            self.assertEqual(self.run_cli("plan", "--bundle", "x", "--config", str(path)), (2, "KF_PROPIMAGE_STAGE_REFUSED SCHEMA\n"))


# ---------------------------------------------------------------- equivalence with the hardcoded stages
def load(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    sys.path.insert(0, str(ROOT / "scripts/bench/windows"))
    sys.path.insert(0, str(ROOT / "scripts/bench/linux"))
    spec.loader.exec_module(mod)
    return mod


def norm(command):
    return [ps.PYTHON] + command[1:]


def legacy_plans():
    """The plans the three hardcoded stages compute now: the first (propimage_apps), the second (propimage_apps_b_stage), the MATLAB rows. Written
    independently of propimage_stage.py: only the stages' own functions and tables are called."""
    pa = load(HERE / "propimage_apps.py", "kf_eq_pa")
    sb = load(HERE / "propimage_apps_b_stage.py", "kf_eq_sb")
    ms = load(HERE / "matlab_stage.py", "kf_eq_ms")
    check = pa.harness(ROOT)
    out = {}
    base = dict(ps.PATHS)
    # first stage
    timeouts = {a: check.CHECKS[a]["timeout"] for a in pa.APPS}
    boots = pa.plan_boots(timeouts)
    cmds = []
    for i, rows in enumerate(boots, 1):
        a = {"dir": base["dir"], "work": "%s/boot%d" % (base["work"], i), "out": "%s/boot%d" % (base["out"], i), "qemu": base["qemu"], "flags": base["flags"],
             "broker": base["broker"], "gop": base["gop"], "apps": rows}
        cmds.append(norm(pa.boot_command(base["check_py"], a)))
    out["imga"] = {"bundle": "imga", "os": "windows", "arm": "kf3", "rows": list(pa.APPS), "lanes": {a: "gate" for a in pa.APPS}, "timeouts": timeouts,
                   "not_run": {}, "boots": boots, "commands": cmds}

    def second(apps):
        mod = sb.apps_module(ROOT)
        order = tuple(a for a in mod.CHECKS if (a in apps if apps else not mod.CHECKS[a].get("disabled")))
        t = {a: mod.CHECKS[a]["timeout"] for a in order}
        lanes = {n: mod.CHECKS[n].get("lane") for n in order}
        bs = sb.plan_boots(t, order, lanes=lanes)
        cs = []
        for i, rows in enumerate(bs, 1):
            a = {"dir": base["dir"], "work": "%s/boot%d" % (base["work"], i), "out": "%s/boot%d" % (base["out"], i), "qemu": base["qemu"], "flags": base["flags"],
                 "broker": base["broker"], "gop": base["gop"], "data": base["data"], "apps": rows, "ram_mb": sb.GUEST_RAM_MB, "audio": sb.AUDIO}
            cs.append(norm(sb.boot_command(base["check_py"], a)))
        return {"bundle": "imgb", "os": "windows", "arm": "kf3", "rows": list(order), "lanes": lanes, "timeouts": t,
                "not_run": {a: pa.clean(c["disabled"], 160) for a, c in mod.CHECKS.items() if c.get("disabled") and a not in order}, "boots": bs, "commands": cs}
    out["imgb"] = second(None)
    out["imgb_only"] = second(["gpudiag", "illustrator", "photoshop", "aeparallel"])
    mr = ms.rows_module(ROOT)
    rows = [r for r in mr.ROW_ORDER if r != mr.PREAMBLE]
    for iso in ("row", "shared", "default"):
        iso_used = "row" if iso == "default" else iso
        out["imgc_" + iso] = {"bundle": "imgc", "os": "linux", "arm": "kf3", "rows": rows, "timeouts": {r: mr.ROWS[r][1] for r in rows},
                              "isolation": iso_used, "boots": mr.plan_boots(rows, iso_used)}
    return out


def generic_plans():
    cfg = pc.load()
    return {"imga": ps.plan(cfg, "imga"), "imgb": ps.plan(cfg, "imgb"),
            "imgb_only": ps.plan(cfg, "imgb", only=["gpudiag", "illustrator", "photoshop", "aeparallel"]),
            "imgc_row": ps.plan(cfg, "imgc", isolation="row"), "imgc_default": ps.plan(cfg, "imgc"), "imgc_shared": ps.plan(cfg, "imgc", isolation="shared")}


def canon(doc):
    return json.dumps(doc, indent=1, sort_keys=True) + "\n"


class Equivalence(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.legacy, cls.generic = legacy_plans(), generic_plans()
        cls.golden = json.loads(GOLDEN.read_text())

    def test_the_golden_names_every_case(self):
        self.assertEqual(sorted(self.golden), ["imga", "imgb", "imgb_only", "imgc_default", "imgc_row", "imgc_shared"])

    def test_first_image_plan_is_byte_identical_to_the_golden(self):
        self.assertEqual(canon(self.generic["imga"]), canon(self.golden["imga"]))

    def test_second_image_plan_is_byte_identical_to_the_golden(self):
        self.assertEqual(canon(self.generic["imgb"]), canon(self.golden["imgb"]))

    def test_second_image_row_subset_plan_is_byte_identical_to_the_golden(self):
        self.assertEqual(canon(self.generic["imgb_only"]), canon(self.golden["imgb_only"]))

    def test_matlab_plan_is_byte_identical_to_the_golden_for_both_isolations(self):
        self.assertEqual(canon(self.generic["imgc_row"]), canon(self.golden["imgc_row"]))
        self.assertEqual(canon(self.generic["imgc_shared"]), canon(self.golden["imgc_shared"]))
        self.assertEqual(canon(self.generic["imgc_default"]), canon(self.golden["imgc_default"]))

    def test_the_golden_is_what_the_hardcoded_stages_compute_now(self):
        self.assertEqual(canon(self.legacy), canon(self.golden))

    def test_generic_and_hardcoded_plans_agree_case_by_case(self):
        for name in self.legacy:
            self.assertEqual(canon(self.generic[name]), canon(self.legacy[name]), name)

    def test_the_shapes_the_golden_pins_are_not_trivial(self):
        self.assertEqual(self.golden["imga"]["boots"][-1], ["wukong"])
        self.assertGreater(len(self.golden["imgb"]["boots"]), 5)
        self.assertTrue(any("--superposition-run" in c for c in self.golden["imga"]["commands"]))
        self.assertTrue(all("--data-iso" in c and "--audio" in c for c in self.golden["imgb"]["commands"]))
        self.assertIn("animatesample", self.golden["imgb"]["not_run"])
        self.assertEqual(self.golden["imgc_row"]["boots"][0], ["gpu_info"])

    def test_every_first_image_row_is_in_one_boot_exactly_once(self):
        for name in ("imga", "imgb"):
            flat = [r for b in self.generic[name]["boots"] for r in b]
            self.assertEqual(sorted(flat), sorted(self.generic[name]["rows"]), name)

    def test_the_vfio_arm_changes_only_the_tail_of_each_committed_command(self):
        cfg = pc.load()
        for bundle in ("imga", "imgb"):
            kf3, vfio = ps.plan(cfg, bundle), ps.plan(cfg, bundle, arm="vfio", vfio_gpu="0000:01:00.0")
            self.assertEqual(kf3["boots"], vfio["boots"], bundle)
            for a, b in zip(kf3["commands"], vfio["commands"]):
                self.assertEqual(b[:-4], a)
                self.assertEqual(b[-4:], ["--platform", "vfio", "--vfio-gpu", "0000:01:00.0"])

    def test_the_budget_constant_is_the_first_stages(self):
        pa = load(HERE / "propimage_apps.py", "kf_eq_pa2")
        self.assertEqual(ps.BOOT_BUDGET_S, pa.BOOT_BUDGET_S)

    def test_the_harness_knows_the_platform_flag_the_arm_adds(self):
        text = (ROOT / "scripts/bench/windows/propimage_check.py").read_text()
        self.assertIn('"--platform"', text)
        self.assertIn('"--vfio-gpu"', text)
        self.assertIn('choices=("kf3", "vfio")', text)

    def test_the_committed_config_is_not_changed_by_planning(self):
        cfg = pc.load()
        before = copy.deepcopy(cfg)
        ps.plan(cfg, "imgb")
        self.assertEqual(cfg, before)


if __name__ == "__main__":
    if "--write-golden" in sys.argv:
        GOLDEN.write_text(canon(legacy_plans()))
        print("wrote", GOLDEN)
    else:
        unittest.main()
