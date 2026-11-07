#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free contracts for the research estimator; no installer/network access."""

import contextlib
import csv
import importlib.util
import io
import json
import pathlib
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("image_size_estimate", HERE / "image_size_estimate.py")
estimate_module = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(estimate_module)
m = estimate_module


def row(name="app", **changes):
    value = dict.fromkeys(m.FIELDS, "")
    value.update(id=name, platform="windows", app=name, nvidia_features="INFERRED: CUDA",
                 obtain_route="free", automation_route="CLI", size_statement="10 GB footprint",
                 size_kind="INSTALLED SIZE", confidence="VENDOR-STATED",
                 source_url="https://example.com/requirements", retrieved_on="2026-11-07",
                 source_low_gb="10", source_high_gb="20", installed_low_gb="10",
                 installed_typical_gb="15", installed_high_gb="20",
                 installed_basis="INFERRED: midpoint of vendor endpoints",
                 compression_class="app", free="1", minimal="1", in_base="0")
    value.update(changes)
    return value


class CatalogTests(unittest.TestCase):
    def read(self, rows, fields=None):
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / "catalog.csv"
            with path.open("w", newline="") as stream:
                writer = csv.DictWriter(stream, fieldnames=fields or m.FIELDS)
                writer.writeheader()
                writer.writerows(rows)
            return m.read_catalog(path)

    def test_valid_catalog(self):
        self.assertEqual(self.read([row()])[0]["app"], "app")

    def test_csv_quoted_comma(self):
        self.assertEqual(self.read([row(app="app, toolkit")])[0]["app"], "app, toolkit")

    def test_missing_column(self):
        fields = [key for key in m.FIELDS if key != "app"]
        data = row()
        del data["app"]
        with self.assertRaises(ValueError):
            self.read([data], fields)

    def test_unexpected_column(self):
        data = row(extra="value")
        with self.assertRaises(ValueError):
            self.read([data], [*m.FIELDS, "extra"])

    def test_renamed_column_same_width(self):
        data = row()
        data["notes"] = data.pop("owner_notes")
        with self.assertRaises(ValueError):
            self.read([data], ["notes" if key == "owner_notes" else key for key in m.FIELDS])

    def test_duplicate_header_same_width(self):
        data = row()
        del data["owner_notes"]
        with self.assertRaises(ValueError):
            self.read([data], ["status" if key == "owner_notes" else key for key in m.FIELDS])

    def test_duplicate_id(self):
        with self.assertRaises(ValueError):
            self.read([row(), row()])

    def test_duplicate_different_name(self):
        with self.assertRaises(ValueError):
            self.read([row(), row(app="other")])

    def test_empty_catalog(self):
        with self.assertRaises(ValueError):
            self.read([])

    def test_invalid_values(self):
        for changes in ({"platform": "mac"}, {"compression_class": "games"},
                        {"free": "yes"}, {"minimal": "2"}, {"in_base": "maybe"},
                        {"size_kind": "SIZE"}, {"confidence": "MEASURED"},
                        {"source_url": ""}, {"retrieved_on": "yesterday"},
                        {"id": ""}, {"app": ""}, {"free": "0"},
                        {"installed_basis": ""}):
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                self.read([row(**changes)])

    def test_negative_installed(self):
        with self.assertRaises(ValueError):
            self.read([row(installed_low_gb="-1")])

    def test_nan_installed(self):
        with self.assertRaises(ValueError):
            self.read([row(installed_typical_gb="NaN")])

    def test_infinity_installed(self):
        with self.assertRaises(ValueError):
            self.read([row(installed_high_gb="inf")])

    def test_reversed_range(self):
        with self.assertRaises(ValueError):
            self.read([row(installed_low_gb="19", installed_typical_gb="15")])

    def test_reversed_typical_high(self):
        with self.assertRaises(ValueError):
            self.read([row(installed_high_gb="14")])

    def test_partial_triplet(self):
        with self.assertRaises(ValueError):
            self.read([row(installed_typical_gb="")])

    def test_partial_triplet_missing_low(self):
        with self.assertRaises(ValueError):
            self.read([row(installed_low_gb="")])

    def test_partial_source_pair(self):
        with self.assertRaises(ValueError):
            self.read([row(source_low_gb="")])

    def test_reversed_source_pair(self):
        with self.assertRaises(ValueError):
            self.read([row(source_low_gb="21")])

    def test_unknown_source_cannot_have_size(self):
        with self.assertRaises(ValueError):
            self.read([row(size_kind="UNKNOWN")])

    def test_unknown_confidence_cannot_have_source_size(self):
        with self.assertRaises(ValueError):
            self.read([row(confidence="UNKNOWN")])


def unknown(name="unknown", kind="UNKNOWN", **changes):
    return row(name, size_kind=kind, source_low_gb="", source_high_gb="",
               installed_low_gb="", installed_typical_gb="", installed_high_gb="",
               installed_basis="", **changes)


class EstimateTests(unittest.TestCase):
    def test_windows_base_once(self):
        result = m.estimate([row()])
        self.assertEqual(result["installed_gb"], [45, 50, 55])

    def test_measured_base_compression(self):
        result = m.estimate([row()])
        self.assertAlmostEqual(result["compressed_gb"][1], 19.09 + 15 * .55)

    def test_linux_does_not_add_windows_base(self):
        result = m.estimate([row(platform="linux")], platform="linux")
        self.assertEqual(result["installed_gb"], [10, 15, 20])

    def test_linux_compression_no_base(self):
        result = m.estimate([row(platform="linux")], platform="linux")
        self.assertEqual(result["compressed_gb"], [5.5, 8.25, 11])

    def test_platform_excludes_other_os(self):
        self.assertEqual(m.estimate([row(), unknown(platform="linux")])["unknown_apps"], [])

    def test_linux_excludes_windows(self):
        result = m.estimate([unknown(), row(platform="linux")], platform="linux")
        self.assertTrue(result["complete"])

    def test_free_only_excludes_paid(self):
        result = m.estimate([row(), unknown(free="0", minimal="0")], set_name="free-only")
        self.assertTrue(result["complete"])

    def test_free_only_does_not_charge_paid_app(self):
        result = m.estimate([row(), row("paid", free="0", minimal="0")], set_name="free-only")
        self.assertEqual(result["installed_gb"], [45, 50, 55])

    def test_all_includes_paid_unknown(self):
        result = m.estimate([row(), unknown(free="0", minimal="0")])
        self.assertFalse(result["complete"])

    def test_minimal_membership(self):
        result = m.estimate([row(), unknown(minimal="0")], set_name="minimal")
        self.assertTrue(result["complete"])

    def test_minimal_excludes_large_free_app(self):
        result = m.estimate([row(minimal="0"), row("small")], set_name="minimal")
        self.assertEqual(result["installed_gb"], [45, 50, 55])

    def test_base_cuda_not_counted_twice(self):
        result = m.estimate([row(), row("cuda", in_base="1")])
        self.assertEqual(result["installed_gb"], [45, 50, 55])

    def test_unknown_base_component_not_a_gap(self):
        result = m.estimate([row(), unknown(in_base="1")])
        self.assertTrue(result["complete"])

    def test_shared_weights_count_on_windows(self):
        result = m.estimate([row(platform="shared", compression_class="model")])
        self.assertEqual(result["installed_gb"], [45, 50, 55])

    def test_shared_weights_count_on_linux(self):
        result = m.estimate([row(platform="shared", compression_class="model")], platform="linux")
        self.assertEqual(result["installed_gb"], [10, 15, 20])

    def test_game_compression(self):
        result = m.estimate([row(compression_class="game")])
        self.assertAlmostEqual(result["compressed_gb"][1], 19.09 + 15 * .95)

    def test_model_compression(self):
        result = m.estimate([row(compression_class="model")])
        self.assertAlmostEqual(result["compressed_gb"][2], 19.09 + 20 * .95)

    def test_single_stream_decimal_units(self):
        result = m.estimate([row()])
        self.assertAlmostEqual(result["single_minutes"][1], (19.09 + 8.25) * 1000 / 55.6 / 60)

    def test_eight_stream_rate_is_aggregate(self):
        result = m.estimate([row()])
        self.assertAlmostEqual(result["eight_stream_minutes"][1], (19.09 + 8.25) * 1000 / 223 / 60)

    def test_cost_low_uses_compressed_transfer(self):
        result = m.estimate([row()])
        self.assertAlmostEqual(result["inbound_usd_004"][1], 27.34 * .004)

    def test_cost_high_uses_compressed_transfer(self):
        result = m.estimate([row()])
        self.assertAlmostEqual(result["inbound_usd_020"][1], 27.34 * .02)

    def test_disk_keeps_archive_and_reserve(self):
        result = m.estimate([row()])
        # Independent hand calculation: (55 + 30.09 + 50) * 1.2 = 162.108 -> 175.
        self.assertEqual(result["disk_request_gb"][2], 175)

    def test_disk_rounds_up(self):
        result = m.estimate([row()], workspace_gb=0, headroom=0)
        self.assertEqual(result["disk_request_gb"][1], 100)

    def test_unknown_prevents_total(self):
        result = m.estimate([row(), unknown()])
        self.assertIsNone(result["installed_gb"])
        self.assertEqual(result["known_installed_gb"], [45, 50, 55])

    def test_unknown_prevents_disk_and_cost(self):
        result = m.estimate([unknown()])
        for key in m.METRICS:
            self.assertIsNone(result[key])

    def test_required_space_not_installed(self):
        data = unknown(kind="REQUIRED SPACE")
        data.update(source_low_gb="10", source_high_gb="30")
        self.assertIsNone(m.estimate([data])["installed_gb"])

    def test_required_space_does_not_produce_disk_request(self):
        data = unknown(kind="REQUIRED SPACE")
        data.update(source_low_gb="10", source_high_gb="30")
        self.assertIsNone(m.estimate([data])["disk_request_gb"])

    def test_download_size_not_installed(self):
        data = unknown(kind="DOWNLOAD SIZE")
        data.update(source_low_gb="10", source_high_gb="30")
        self.assertIsNone(m.estimate([data])["installed_gb"])

    def test_download_does_not_produce_transfer_forecast(self):
        data = unknown(kind="DOWNLOAD SIZE")
        data.update(source_low_gb="10", source_high_gb="30")
        self.assertIsNone(m.estimate([data], basis="required-space-proxy")["single_minutes"])

    def test_proxy_required_endpoints_and_midpoint(self):
        data = unknown(kind="REQUIRED SPACE")
        data.update(source_low_gb="10", source_high_gb="30")
        result = m.estimate([data], basis="required-space-proxy")
        self.assertEqual(result["installed_gb"], [45, 55, 65])

    def test_proxy_typical_uses_midpoint(self):
        data = unknown(kind="REQUIRED SPACE")
        data.update(source_low_gb="2", source_high_gb="6")
        result = m.estimate([data], basis="required-space-proxy")
        self.assertAlmostEqual(result["compressed_gb"][1], 19.09 + 4 * .55)

    def test_proxy_never_unpacks_downloads(self):
        data = unknown(kind="DOWNLOAD SIZE")
        data.update(source_low_gb="10", source_high_gb="30")
        self.assertFalse(m.estimate([data], basis="required-space-proxy")["complete"])

    def test_proxy_remains_unknown_with_missing_requirement(self):
        self.assertFalse(m.estimate([unknown()], basis="required-space-proxy")["complete"])

    def test_proxy_preserves_explicit_installed_values(self):
        data = row(size_kind="REQUIRED SPACE", source_low_gb="100", source_high_gb="100")
        self.assertEqual(m.estimate([data], basis="required-space-proxy")["installed_gb"], [45, 50, 55])

    def test_empty_selected_set_refused(self):
        with self.assertRaises(ValueError):
            m.estimate([row(platform="linux")])

    def test_invalid_options(self):
        for changes in ({"app_ratio": 0}, {"app_ratio": 1.1}, {"asset_ratio": float("nan")},
                        {"headroom": -1}, {"workspace_gb": float("inf")},
                        {"platform": "other"}, {"set_name": "paid"}, {"basis": "guess"}):
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                m.estimate([row()], **changes)

    def test_zero_asset_ratio_refused(self):
        with self.assertRaises(ValueError):
            m.estimate([row()], asset_ratio=0)

    def test_negative_workspace_refused(self):
        with self.assertRaises(ValueError):
            m.estimate([row()], workspace_gb=-1)

    def test_selected_empty_minimal_refused(self):
        with self.assertRaises(ValueError):
            m.estimate([row(minimal="0")], set_name="minimal")

    def test_linux_transfer_and_cost_hand_calculation(self):
        result = m.estimate([row(platform="linux", compression_class="model")], platform="linux")
        self.assertAlmostEqual(result["single_minutes"][0], 9.5 * 1000 / 55.6 / 60)
        self.assertAlmostEqual(result["eight_stream_minutes"][0], 9.5 * 1000 / 223 / 60)
        self.assertAlmostEqual(result["inbound_usd_004"][0], .038)
        self.assertAlmostEqual(result["inbound_usd_020"][0], .19)

    def test_overflow_refused(self):
        with self.assertRaises(ValueError):
            m.estimate([row(installed_low_gb="1e308", installed_typical_gb="1e308", installed_high_gb="1e308")])

    def test_workspace_headroom_overflow_refused(self):
        with self.assertRaises(ValueError):
            m.estimate([row()], workspace_gb=1e308, headroom=1e308)


class CommandTests(unittest.TestCase):
    def run_cli(self, data, *args):
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / "input.csv"
            with path.open("w", newline="") as stream:
                writer = csv.DictWriter(stream, fieldnames=m.FIELDS)
                writer.writeheader()
                writer.writerows(data)
            stdout, stderr = io.StringIO(), io.StringIO()
            with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                code = m.main([str(path), *args])
            return code, stdout.getvalue(), stderr.getvalue()

    def test_json_cli_three_sets(self):
        code, output, _ = self.run_cli([row()], "--json")
        self.assertEqual(code, 0)
        self.assertEqual([r["set"] for r in json.loads(output)], ["all", "free-only", "minimal"])

    def test_cli_unknown_is_explicit(self):
        code, output, _ = self.run_cli([unknown()])
        self.assertEqual(code, 0)
        self.assertIn("TOTAL UNKNOWN", output)
        self.assertIn("KNOWN COMPONENT SUBTOTAL", output)
        self.assertIn("unknown", output)

    def test_cli_labels_assumptions(self):
        _, output, _ = self.run_cli([row()])
        self.assertIn("INFERRED", output)
        self.assertIn("55.6", output)
        self.assertIn("223", output)

    def test_cli_bad_catalog_returns_two(self):
        code, _, error = self.run_cli([row(installed_high_gb="-1")])
        self.assertEqual(code, 2)
        self.assertIn("error", error)

    def test_cli_invalid_ratio_returns_two(self):
        code, _, _ = self.run_cli([row()], "--app-ratio", "nan")
        self.assertEqual(code, 2)

    def test_cli_complete_numeric_output(self):
        code, output, _ = self.run_cli([row()], "--set", "all", "--json")
        self.assertEqual(code, 0)
        result = json.loads(output)[0]
        self.assertEqual(result["disk_request_gb"], [150, 175, 175])
        self.assertAlmostEqual(result["compressed_gb"][1], 27.34)
        self.assertEqual(result["installed_gb"], [45, 50, 55])

    def test_real_catalog_research_fields(self):
        catalog = HERE.parents[1] / "docs/design/V3_RELEASE_SOFTWARE_WISHLIST.csv"
        rows = m.read_catalog(catalog)
        self.assertGreaterEqual(len(rows), 60)
        for data in rows:
            with self.subTest(app=data["app"]):
                self.assertEqual(data["retrieved_on"], "2026-11-07")
                self.assertEqual(data["status"], "")
                self.assertEqual(data["owner_notes"], "")
                self.assertTrue(data["source_url"].startswith("https://"))
        self.assertFalse(m.estimate(rows)["complete"])

    def test_published_table_matches_research_csv(self):
        design = HERE.parents[1] / "docs/design"
        rows = m.read_catalog(design / "V3_RELEASE_SOFTWARE_WISHLIST.csv")
        document = (design / "V3_RELEASE_SOFTWARE_WISHLIST.md").read_text()
        table = document.split("<!-- SZ1-CATALOG-START -->")[1].split("<!-- SZ1-CATALOG-END -->")[0]
        for row_data in rows:
            with self.subTest(app=row_data["app"]):
                prefix = "| " + row_data["app"] + " |"
                matches = [line for line in table.splitlines() if line.startswith(prefix)]
                self.assertEqual(len(matches), 1)
                cells = [cell.strip() for cell in matches[0].strip("|").split("|")]
                self.assertEqual(cells[:4], [row_data[key] for key in
                                           ("app", "nvidia_features", "obtain_route", "automation_route")])
                self.assertEqual(cells[5:], ["", ""])
                for key in ("size_statement", "size_kind", "confidence", "retrieved_on", "installed_basis"):
                    self.assertIn(row_data[key], cells[4])
                for url in row_data["source_url"].split("; "):
                    self.assertIn("](" + url + ")", cells[4])

    def test_every_requested_product_is_in_catalog(self):
        catalog = HERE.parents[1] / "docs/design/V3_RELEASE_SOFTWARE_WISHLIST.csv"
        ids = {data["id"] for data in m.read_catalog(catalog)}
        required = set("premiere after-effects photoshop media-encoder resolve maya arnold 3ds-max revit inventor solidworks solidworks-simulation solidworks-visualize catia abaqus creo unreal-engine unreal-sample unity omniverse-launcher usd-composer isaac-sim octane redshift vray keyshot houdini matlab-base matlab-gpu-toolboxes ansys-fluent ansys-mechanical nvidia-app broadcast topaz-video-ai sunshine cuda-toolkit cudnn tensorrt vs-build-tools python-pytorch comfyui-windows sdxl-checkpoint gguf-7b gguf-70b blender obs cyberpunk alan-wake-2 portal-rtx metro-enhanced forza-5 vllm tensorrt-llm jax rapids comfyui-linux ollama optix-sdk video-codec-sdk proton dxvk vkd3d gromacs namd".split())
        self.assertEqual(ids, required)


if __name__ == "__main__":
    unittest.main()
