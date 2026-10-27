"""Synthetic PF2 table tests. Fixture fps are not hardware measurements."""
import copy
import unittest

import stamp


def result(arm="A", rep=1, api="direct3d11"):
    fps = (280, 300, 360)[rep - 1] + {"A": 0, "B": -150, "C": 30}[arm]
    fps += 40 if api == "opengl" else 0
    return dict(
        arm=arm, boot=f"{arm}{rep}", api=api, revision=stamp.REVISIONS[arm],
        finished=True, watch_complete=True, adapter_ok=True, settings_ok=True,
        guest_resets=[], host_xids=[], fps=float(fps), min_fps=40.0,
        max_fps=float(fps * 2), duration_s=260.0, frames=fps * 260,
    )


def complete():
    return [result(arm, rep, api) for arm in stamp.REVISIONS
            for rep in (1, 2, 3) for api in stamp.APIS]


class Acceptance(unittest.TestCase):
    def bad(self, api, field, value, reason):
        row = result(api=api)
        row[field] = value
        self.assertEqual(stamp.validate(row), reason)

    def test_valid_d3d11(self):
        self.assertIsNone(stamp.validate(result()))

    def test_valid_opengl(self):
        self.assertIsNone(stamp.validate(result(api="opengl")))

    def test_reset_153_invalidates_fast_d3d11(self):
        self.bad("direct3d11", "guest_resets", [153], "GUEST_RESET")

    def test_reset_141_invalidates_fast_opengl(self):
        self.bad("opengl", "guest_resets", [141], "GUEST_RESET")

    def test_host_fault_invalidates_d3d11(self):
        self.bad("direct3d11", "host_xids", [31], "HOST_XID")

    def test_host_fault_invalidates_opengl(self):
        self.bad("opengl", "host_xids", [109], "HOST_XID")

    def test_unwatched_d3d11_is_not_clean(self):
        self.bad("direct3d11", "watch_complete", False, "UNWATCHED")

    def test_missing_opengl_watch_is_not_clean(self):
        row = result(api="opengl")
        del row["watch_complete"]
        self.assertEqual(stamp.validate(row), "UNWATCHED")

    def test_incomplete_d3d11_is_not_a_result(self):
        self.bad("direct3d11", "finished", False, "INCOMPLETE")

    def test_incomplete_opengl_is_not_a_result(self):
        self.bad("opengl", "finished", False, "INCOMPLETE")

    def test_adapter_failure_invalidates_d3d11(self):
        self.bad("direct3d11", "adapter_ok", False, "ADAPTER")

    def test_adapter_failure_invalidates_opengl(self):
        self.bad("opengl", "adapter_ok", False, "ADAPTER")

    def test_wrong_d3d11_recipe_is_not_accepted(self):
        self.bad("direct3d11", "settings_ok", False, "SETTINGS")

    def test_wrong_opengl_recipe_is_not_accepted(self):
        self.bad("opengl", "settings_ok", False, "SETTINGS")

    def test_wrong_master_revision_is_rejected(self):
        self.bad("direct3d11", "revision", "0" * 40, "REVISION")

    def test_wrong_candidate_revision_is_rejected(self):
        row = result("C", api="opengl")
        row["revision"] = stamp.REVISIONS["A"]
        self.assertEqual(stamp.validate(row), "REVISION")

    def test_nonfinite_d3d11_fps_is_rejected(self):
        self.bad("direct3d11", "fps", float("inf"), "NUMERIC")

    def test_nonfinite_opengl_minimum_is_rejected(self):
        self.bad("opengl", "min_fps", float("nan"), "NUMERIC")

    def test_short_d3d11_duration_is_rejected(self):
        self.bad("direct3d11", "duration_s", 10.0, "DURATION")

    def test_wrong_opengl_duration_is_rejected(self):
        self.bad("opengl", "duration_s", 500.0, "DURATION")

    def test_inconsistent_d3d11_frame_count_is_rejected(self):
        self.bad("direct3d11", "frames", 10, "FRAME_COUNT")

    def test_inconsistent_opengl_frame_count_is_rejected(self):
        self.bad("opengl", "frames", 10, "FRAME_COUNT")

    def test_master_medians_use_middle_values(self):
        summary = stamp.summarize(complete())
        self.assertEqual(summary["A/direct3d11"]["median_fps"], 300)
        self.assertEqual(summary["A/opengl"]["median_fps"], 340)

    def test_candidate_medians_use_middle_values(self):
        summary = stamp.summarize(complete())
        self.assertEqual(summary["C/direct3d11"]["median_fps"], 330)
        self.assertEqual(summary["C/opengl"]["median_fps"], 370)

    def test_complete_d3d11_reports_three_boots_and_spread(self):
        row = stamp.summarize(complete())["A/direct3d11"]
        self.assertEqual((row["status"], row["n"]), ("PASS", 3))
        self.assertEqual(row["fps_range"], [280, 360])

    def test_complete_opengl_reports_three_boots_and_spread(self):
        row = stamp.summarize(complete())["C/opengl"]
        self.assertEqual((row["status"], row["n"]), ("PASS", 3))
        self.assertEqual(row["fps_range"], [350, 430])

    def test_two_master_boots_cannot_make_a_complete_headline(self):
        rows = [r for r in complete() if (r["boot"], r["api"]) != ("A3", "direct3d11")]
        row = stamp.summarize(rows)["A/direct3d11"]
        self.assertEqual((row["status"], row["n"]), ("INCOMPLETE", 2))
        self.assertIsNone(row["median_fps"])

    def test_two_control_boots_cannot_make_a_complete_headline(self):
        rows = [r for r in complete() if (r["boot"], r["api"]) != ("B3", "opengl")]
        row = stamp.summarize(rows)["B/opengl"]
        self.assertEqual((row["status"], row["n"]), ("INCOMPLETE", 2))
        self.assertIsNone(row["median_fps"])

    def test_duplicate_master_boot_is_not_a_fourth_sample(self):
        rows = complete()
        rows.append(copy.deepcopy(rows[0]))
        summary = stamp.summarize(rows)
        self.assertEqual(summary["A/direct3d11"]["status"], "FAIL")
        self.assertIsNone(summary["A/opengl"]["median_fps"])

    def test_duplicate_candidate_boot_is_not_a_fourth_sample(self):
        rows = complete()
        rows.append(copy.deepcopy(rows[-1]))
        summary = stamp.summarize(rows)
        self.assertEqual(summary["C/opengl"]["status"], "FAIL")
        self.assertIsNone(summary["C/direct3d11"]["median_fps"])

    def test_master_reset_prevents_both_headline_medians(self):
        rows = complete()
        rows[0]["guest_resets"] = [153]
        summary = stamp.summarize(rows)
        self.assertIsNone(summary["A/direct3d11"]["median_fps"])
        self.assertIsNone(summary["A/opengl"]["median_fps"])

    def test_candidate_reset_prevents_both_headline_medians(self):
        rows = complete()
        rows[-1]["guest_resets"] = [4101]
        summary = stamp.summarize(rows)
        self.assertIsNone(summary["C/direct3d11"]["median_fps"])
        self.assertIsNone(summary["C/opengl"]["median_fps"])


if __name__ == "__main__":
    unittest.main()
