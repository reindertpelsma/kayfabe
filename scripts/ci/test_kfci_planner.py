"""J2 interop falsifier: schema-invalid or proposed input runs, or wire order is lost.

The synthetic assignment follows J2's read-only schema snapshot. No real box,
installer or driver is fetched, and the Worker planner is never invoked.
"""

import copy
import io
import json
from pathlib import Path
import sys
import unittest

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
from kfci.assignment import AssignmentError, load, validate
from kfci.catalog import Catalog, CatalogError
from kfci.runner import Runner
from kfci.testing import FakeClock, FakeStep


def wire():
    return json.loads(
        (SCRIPTS / "ci/fixtures/kfci/planner_assignment.json").read_text()
    )


def modern_wire():
    value = wire()
    value["capabilities"]["can_expose_port"] = False
    value["seed_server"] = None
    value["execution_steps"] = [
        {"kind": "run", "step_id": item["id"]} for item in value["runs"]
    ]
    for pin in value["driver_pins"]:
        pin["abi_version"] = pin["version"]
    return value


class PlannerCurrentContract(unittest.TestCase):
    def test_reviewed_wire_without_seed_operations_is_accepted(self):
        value = modern_wire()
        self.assertEqual(validate(value), value)

    def test_reviewed_abi_metadata_survives_validation(self):
        value = modern_wire()
        value["driver_pins"][0]["abi_version"] = "580.1"
        self.assertEqual(validate(value)["driver_pins"][0]["abi_version"], "580.1")

    def test_seed_server_is_refused_before_any_action(self):
        value = modern_wire()
        value["seed_server"] = {"container_port": 1234, "serve_until_minute": 50}
        with self.assertRaises(AssignmentError):
            validate(value)

    def test_another_seed_server_is_not_silently_ignored(self):
        value = modern_wire()
        value["seed_server"] = {"container_port": 8080, "serve_until_minute": 100}
        with self.assertRaises(AssignmentError):
            validate(value)

    def test_execution_sequence_cannot_drop_a_run(self):
        value = modern_wire()
        value["execution_steps"] = []
        with self.assertRaises(AssignmentError):
            validate(value)

    def test_execution_sequence_cannot_duplicate_a_run(self):
        value = modern_wire()
        value["execution_steps"] *= 2
        with self.assertRaises(AssignmentError):
            validate(value)

    def test_wait_operation_is_refused_instead_of_skipped(self):
        value = modern_wire()
        value["execution_steps"].insert(
            0,
            dict(
                kind="wait_for_artifact",
                artifact="open_image",
                scope_key="open_image@box:fake",
                deadline_minute=10,
                fetch_until_minute=5,
                minutes=1,
                fallback="build_local",
                fallback_step_ids=["setup:0"],
            ),
        )
        with self.assertRaises(AssignmentError):
            validate(value)


class PlannerSchema(unittest.TestCase):
    def test_j2_wire_assignment_loads(self):
        value = wire()
        self.assertEqual(load(io.StringIO(json.dumps(value))), value)

    def test_driver_version_pattern_uses_ascii_digits_like_javascript(self):
        value = wire()
        value["driver_pins"][0]["version"] = "٥٨٠.١٥٩.٠٤"
        with self.assertRaises(AssignmentError):
            validate(value)

    def test_driver_date_pattern_uses_ascii_digits_like_javascript(self):
        value = wire()
        value["driver_pins"][0]["checked"] = "٢٠٢٦-١١-٠٨"
        with self.assertRaises(AssignmentError):
            validate(value)

    def test_driver_version_disallows_trailing_newline_like_javascript(self):
        value = wire()
        value["driver_pins"][0]["version"] += "\n"
        with self.assertRaises(AssignmentError):
            validate(value)

    def test_driver_date_disallows_trailing_newline_like_javascript(self):
        value = wire()
        value["driver_pins"][0]["checked"] += "\n"
        with self.assertRaises(AssignmentError):
            validate(value)

    def test_driver_pin_identifier_disallows_trailing_newline_like_javascript(self):
        value = wire()
        pin = value["driver_pins"][0]
        pin["id"] += "\n"
        for entry in value["runs"]:
            entry["drivers"] = {axis: pin["id"] for axis in entry["drivers"]}
        with self.assertRaises(AssignmentError):
            validate(value)

    def test_parameter_string_limit_counts_javascript_utf16_units(self):
        value = wire()
        value["runs"][0]["parameters"]["label"] = "😀" * 151
        with self.assertRaises(AssignmentError):
            validate(value)

    def test_parameter_array_item_limit_counts_javascript_utf16_units(self):
        value = wire()
        value["runs"][0]["parameters"]["labels"] = ["😀" * 151]
        with self.assertRaises(AssignmentError):
            validate(value)

    def test_nested_unknown_step_field_is_refused(self):
        value = wire()
        value["runs"][0]["command"] = ["arbitrary"]
        with self.assertRaises(AssignmentError):
            validate(value)

    def test_another_nested_unknown_field_is_refused(self):
        value = wire()
        value["capabilities"]["unknown"] = True
        with self.assertRaises(AssignmentError):
            validate(value)

    def test_wrong_nested_types_are_refused(self):
        value = wire()
        value["runs"][0]["minutes"] = True
        with self.assertRaises(AssignmentError):
            validate(value)

    def test_nonfinite_nested_minutes_are_refused(self):
        value = wire()
        value["runs"][0]["minutes"] = float("nan")
        with self.assertRaises(AssignmentError):
            validate(value)

    def test_proposed_assignment_is_not_executable(self):
        value = wire()
        value["status"] = "proposed"
        with self.assertRaises(AssignmentError):
            validate(value)

    def test_another_proposed_assignment_cannot_execute(self):
        value = wire()
        value["id"] = "another_box"
        value["status"] = "proposed"
        with self.assertRaises(AssignmentError):
            validate(value)

    def test_unpinned_driver_is_not_executable(self):
        value = wire()
        value["driver_pins"][0]["status"] = "UNPINNED"
        value["driver_pins"][0]["sha256"] = None
        value["driver_pins"][0]["size"] = None
        with self.assertRaises(AssignmentError):
            validate(value)

    def test_incomplete_pin_is_not_executable(self):
        value = wire()
        value["driver_pins"][0]["sha256"] = None
        with self.assertRaises(AssignmentError):
            validate(value)

    def test_duplicate_instance_ids_are_refused(self):
        value = wire()
        value["runs"][1]["id"] = value["runs"][0]["id"]
        value["execution_steps"] = [
            {"kind": "run", "step_id": item["id"]} for item in value["runs"]
        ]
        with self.assertRaises(AssignmentError):
            validate(value)

    def test_duplicate_later_instance_is_refused(self):
        value = wire()
        value["runs"].append(copy.deepcopy(value["runs"][1]))
        value["execution_steps"] = [
            {"kind": "run", "step_id": item["id"]} for item in value["runs"]
        ]
        with self.assertRaises(AssignmentError):
            validate(value)

    def test_missing_guest_driver_reference_is_refused(self):
        value = wire()
        value["runs"][1]["drivers"]["linux_guest"] = "missing_guest"
        with self.assertRaises(AssignmentError):
            validate(value)

    def test_undeclared_driver_reference_is_refused(self):
        value = wire()
        value["runs"][0]["drivers"]["host"] = "missing"
        with self.assertRaises(AssignmentError):
            validate(value)

    def test_seed_hint_needs_a_migrated_seed_step(self):
        value = wire()
        value["seed_hints"] = [
            dict(
                artifact="open_image",
                scope_key="open_image@box:fake",
                producer_assignment_id="producer",
                producer_box_id="producer_box",
                available_minute=1,
                transfer_minutes=1,
                deadline_minute=2,
                fallback="build_local",
                fetch_until_minute=1,
                fallback_step_ids=["setup:0"],
                gb=1,
            )
        ]
        with self.assertRaises(AssignmentError):
            validate(value)

    def test_seed_export_needs_a_migrated_seed_step(self):
        value = wire()
        value["seed_exports"] = [
            dict(
                artifact="open_image",
                scope_key="open_image@box:fake",
                consumer_assignment_id="consumer",
                ready_after_step_id="setup:0",
                gb=1,
                minutes=1,
                available_minute=1,
                deadline_minute=2,
            )
        ]
        with self.assertRaises(AssignmentError):
            validate(value)

    def test_duplicate_driver_pin_is_refused(self):
        value = wire()
        value["driver_pins"].append(copy.deepcopy(value["driver_pins"][0]))
        with self.assertRaises(AssignmentError):
            validate(value)

    def test_conflicting_duplicate_driver_pin_is_refused(self):
        value = wire()
        duplicate = copy.deepcopy(value["driver_pins"][0])
        duplicate["sha256"] = "c" * 64
        value["driver_pins"].append(duplicate)
        with self.assertRaises(AssignmentError):
            validate(value)

    def test_assignment_marker_cannot_contain_newline(self):
        value = wire()
        value["id"] += "\n"
        with self.assertRaises(AssignmentError):
            validate(value)

    def test_instance_marker_cannot_contain_newline(self):
        value = wire()
        value["runs"][1]["id"] += "\n"
        value["execution_steps"] = [
            {"kind": "run", "step_id": item["id"]} for item in value["runs"]
        ]
        with self.assertRaises(AssignmentError):
            validate(value)

    def test_worst_case_cannot_exceed_reservation(self):
        value = wire()
        value["worst_case_minutes"] = 121
        with self.assertRaises(AssignmentError):
            validate(value)

    def test_smaller_reservation_cannot_admit_worst_case(self):
        value = wire()
        value["reservation_minutes"] = 1
        with self.assertRaises(AssignmentError):
            validate(value)


class PlannerExecution(unittest.TestCase):
    def setup_plan(self):
        cat = Catalog()
        first = cat.register(FakeStep("prepare", produces=("linux_apps_ready",)))
        second = cat.register(FakeStep("probe", needs=("linux_apps_ready",)))
        output = io.StringIO()
        return (
            wire(),
            Runner(cat, output=output, clock=FakeClock()),
            first,
            second,
            output,
        )

    def test_wire_instance_order_and_metadata_are_preserved(self):
        value, runner, first, second, output = self.setup_plan()
        summary = runner.run(value)
        self.assertEqual(summary["verdict"], "PASS")
        self.assertEqual(
            [row["id"] for row in summary["steps"]], ["setup:0", "probe:0"]
        )
        self.assertEqual(second.calls[0].drivers, value["runs"][1]["drivers"])
        self.assertEqual(second.calls[0].assignment_id, "wire_box")
        self.assertIn("KFCI_STEP_START id=probe:0", output.getvalue())

    def test_unpinned_driver_is_refused_before_setup_runs(self):
        value, runner, first, second, _ = self.setup_plan()
        pin = value["driver_pins"][0]
        pin.update(
            status="UNPINNED", sha256=None, size=None, version=None, abi_version=None
        )
        with self.assertRaises(AssignmentError):
            runner.run(value)
        self.assertEqual(first.calls, [])
        self.assertEqual(second.calls, [])

    def test_repeated_run_type_is_not_deduplicated(self):
        value, runner, first, second, _ = self.setup_plan()
        repeated = copy.deepcopy(value["runs"][1])
        repeated["id"] = "probe:1"
        value["runs"].append(repeated)
        value["execution_steps"] = [
            {"kind": "run", "step_id": item["id"]} for item in value["runs"]
        ]
        summary = runner.run(value)
        self.assertEqual(summary["verdict"], "PASS")
        self.assertEqual(len(second.calls), 2)
        self.assertEqual(
            [r["id"] for r in summary["steps"]], ["setup:0", "probe:0", "probe:1"]
        )

    def test_unknown_run_is_refused_before_setup(self):
        value, runner, first, _, _ = self.setup_plan()
        value["runs"][1]["run_id"] = "unmigrated"
        with self.assertRaises(CatalogError):
            runner.run(value)
        self.assertEqual(first.calls, [])

    def test_scoped_artifact_order_is_checked(self):
        value, runner, first, _, _ = self.setup_plan()
        value["runs"].reverse()
        value["execution_steps"] = [
            {"kind": "run", "step_id": item["id"]} for item in value["runs"]
        ]
        with self.assertRaises(CatalogError):
            runner.run(value)
        self.assertEqual(first.calls, [])

    def test_wire_needs_cannot_erase_registered_preconditions(self):
        value, runner, first, _, _ = self.setup_plan()
        value["runs"][1]["needs"] = []
        with self.assertRaises(CatalogError):
            runner.run(value)
        self.assertEqual(first.calls, [])

    def test_wire_products_cannot_claim_unproduced_artifacts(self):
        value, runner, first, _, _ = self.setup_plan()
        value["runs"][0]["produces"].append("forged@box:fake")
        with self.assertRaises(CatalogError):
            runner.run(value)
        self.assertEqual(first.calls, [])

    def test_reservation_is_a_budget_guard(self):
        value, runner, first, second, _ = self.setup_plan()
        value["reservation_minutes"] = 1
        value["worst_case_minutes"] = 1
        value["runs"][0]["minutes"] = 2
        result = runner.run(value)
        self.assertEqual(result["verdict"], "RETRY")
        self.assertEqual(first.calls, [])
        self.assertEqual(second.calls, [])

    def test_different_scoped_artifact_is_unavailable(self):
        value, runner, first, _, _ = self.setup_plan()
        value["runs"][1]["needs"] = ["linux_apps_ready@other:box"]
        with self.assertRaises(CatalogError):
            runner.run(value)
        self.assertEqual(first.calls, [])

    def test_later_declared_estimate_is_a_budget_guard(self):
        value, runner, first, second, _ = self.setup_plan()
        value["reservation_minutes"] = 2
        value["runs"][1]["minutes"] = 3
        result = runner.run(value)
        self.assertEqual(result["verdict"], "RETRY")
        self.assertEqual(second.calls, [])


class PlannerStageAliases(unittest.TestCase):
    def test_full_apps_alias_matches_j2_catalog_artifacts(self):
        from kfci.cli import catalog

        step = catalog().get("linux_apps_full")
        self.assertEqual(set(step.needs), {"kf3_bins", "open_image"})
        self.assertEqual(step.produces, ())

    def test_middle_apps_alias_matches_j2_catalog_artifacts(self):
        from kfci.cli import catalog

        step = catalog().get("linux_apps_middle")
        self.assertEqual(set(step.needs), {"kf3_bins", "open_image"})
        self.assertEqual(step.produces, ())


if __name__ == "__main__":
    unittest.main()
