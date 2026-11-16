#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of scripts/bench/derive_native_fault_names.py (STATUS: LIVE, 2026-11-16).

Three layers, each named by what it needs:
  Fixture*    tiny source texts shaped like the open driver's; no oracle, always run.
  Committed*  the committed native_fault_names.json against the script's own constants; always run.
  Oracle*     the real pinned third_party/ogkm-580 (fetch: scripts/ci/fetch_oracles.sh). With the oracle
              absent they skip; KF_REQUIRE_ORACLES=1 turns that skip into a failure, like the Rust oracle tests.
Run: python3 -m unittest scripts/ci/test_derive_native_fault_names.py
Mutants: python3 scripts/ci/derive_fault_names_mutants.py (two named killers each).
"""

import importlib.util
import json
import os
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = Path(os.environ.get("KF_DERIVE_FAULT_NAMES_SCRIPT") or ROOT / "scripts/bench/derive_native_fault_names.py")
JSON_PATH = Path(os.environ.get("KF_NATIVE_FAULT_NAMES_JSON") or ROOT / "scripts/bench/native_fault_names.json")
SPEC = importlib.util.spec_from_file_location("derive_native_fault_names", SCRIPT)
D = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(D)

FIFO = D.FIFO
F_COMMON, F_AMPERE, F_BLACKWELL, F_TURING, H_AMPERE, H_BLACKWELL, H_TURING, H_PROJECT = D.FILES

COMMON = """
  const char *pbdmaIdString[2] = {"_PBDMA0", "_PBDMA1"};
  const char *hosts[] = {"GR_HOST0", "GR_HOST1"};
"""
AMPERE = 'case NV_PFAULT_CLIENT_HUB_ESC: return "HUBCLIENT_ESC";\n'
BLACKWELL = 'case NV_PFAULT_CLIENT_HUB_ESC0: return "HUBCLIENT_ESC0";\n'
HEADER_AMPERE = "#define NV_PFAULT_CLIENT_HUB_ESC         0x00000063 /*       */\n"
HEADER_BLACKWELL = "#define NV_PFAULT_CLIENT_HUB_ESC0        0x00000064 /*       */\n"
TURING = """
kfifoPrintPbdmaId_TU102(void)
{
    static const char* pbdmaIdString[NV_HOST_NUM_PBDMA] = { "HOST0",
                                                            "HOST1",
                                                            "HOST2",
                                                          };
}
            case NV_PFAULT_CLIENT_HUB_HOST:
                return "HUBCLIENT_HOST";
            case NV_PFAULT_CLIENT_HUB_HOST_CPU:
                return "HUBCLIENT_HOST_CPU";
            case NV_PFAULT_CLIENT_HUB_HOST_CPU_NB:
                return "HUBCLIENT_HOST_CPU_NB";
"""
HEADER_TURING = (
    "#define NV_PFAULT_CLIENT_HUB_HOST        0x00000006 /*       */\n"
    "#define NV_PFAULT_CLIENT_HUB_HOST_CPU    0x00000007 /*       */\n"
    "#define NV_PFAULT_CLIENT_HUB_HOST_CPU_NB 0x00000008 /*       */\n"
)
PROJECT = "#define NV_HOST_NUM_PBDMA                          3\n"


def fixture(**changes):
    texts = {
        F_COMMON: COMMON,
        F_AMPERE: AMPERE,
        F_BLACKWELL: BLACKWELL,
        F_TURING: TURING,
        H_AMPERE: HEADER_AMPERE,
        H_BLACKWELL: HEADER_BLACKWELL,
        H_TURING: HEADER_TURING,
        H_PROJECT: PROJECT,
    }
    for path, (old, new) in changes.items():
        key = {
            "common": F_COMMON, "ampere": F_AMPERE, "blackwell": F_BLACKWELL, "turing": F_TURING,
            "h_ampere": H_AMPERE, "h_blackwell": H_BLACKWELL, "h_turing": H_TURING, "project": H_PROJECT,
        }[path]
        assert texts[key].count(old) == 1, (path, old)
        texts[key] = texts[key].replace(old, new)
    return {p: t.encode() for p, t in texts.items()}


class Fixture(unittest.TestCase):
    def test_ampere_and_blackwell_names_are_unchanged(self):
        engines, clients = D.derive_names(fixture())
        self.assertLessEqual({"GR_HOST0", "GR_HOST1", "GR0_PBDMA0", "GR1_PBDMA1"}, engines)
        self.assertLessEqual({"HUBCLIENT_ESC", "HUBCLIENT_ESC0"}, clients)

    def test_turing_engines_are_the_pbdma_id_strings(self):
        engines, _ = D.derive_names(fixture())
        self.assertEqual({e for e in engines if e.startswith("HOST")}, {"HOST0", "HOST1", "HOST2"})

    def test_turing_client_is_host_cpu_only(self):
        _, clients = D.derive_names(fixture())
        self.assertIn("HUBCLIENT_HOST_CPU", clients)
        for other in ("HUBCLIENT_HOST", "HUBCLIENT_HOST_CPU_NB"):
            self.assertNotIn(other, clients)

    def test_turing_engine_set_follows_the_source_not_a_constant(self):
        sources = fixture(
            turing=('"HOST2",\n', ""), project=("   3\n", "   2\n"),
        )
        engines, _ = D.derive_names(sources)
        self.assertEqual({e for e in engines if e.startswith("HOST")}, {"HOST0", "HOST1"})

    def test_the_output_carries_the_turing_pair(self):
        out = json.loads(D.derive(fixture()))
        self.assertIn("HOST0", out["engines"])
        self.assertIn("HUBCLIENT_HOST_CPU", out["clients"])

    def test_pbdma_count_disagreeing_with_hwproject_is_refused(self):
        with self.assertRaises(AssertionError):
            D.derive_names(fixture(project=("   3\n", "   4\n")))

    def test_renumbered_pbdma_names_are_refused(self):
        with self.assertRaises(AssertionError):
            D.derive_names(fixture(turing=('"HOST1",', '"HOSTX",')))

    def test_missing_pbdma_table_is_refused(self):
        with self.assertRaises(AssertionError):
            D.derive_names(fixture(turing=("pbdmaIdString[NV_HOST_NUM_PBDMA]", "other[NV_HOST_NUM_PBDMA]")))

    def test_printer_without_host_cpu_is_refused(self):
        with self.assertRaises(AssertionError):
            D.derive_names(fixture(turing=('return "HUBCLIENT_HOST_CPU";', 'return "HUBCLIENT_X";')))

    def test_header_without_host_cpu_is_refused(self):
        with self.assertRaises(AssertionError):
            D.derive_names(fixture(h_turing=("NV_PFAULT_CLIENT_HUB_HOST_CPU    0x00000007", "NV_PFAULT_CLIENT_HUB_OTHER 0x00000007")))

    def test_esc_printer_and_header_must_agree(self):
        with self.assertRaises(AssertionError):
            D.derive_names(fixture(h_blackwell=("HUB_ESC0 ", "HUB_ESC1 ")))

    def test_an_esc_printer_without_a_header_definition_is_refused(self):
        with self.assertRaises(AssertionError):
            D.derive_names(fixture(blackwell=('"HUBCLIENT_ESC0"', '"HUBCLIENT_ESC5"')))

    def test_derive_binds_every_source_by_hash(self):
        out = json.loads(D.derive(fixture()))
        self.assertEqual(list(out["sha256"]), list(D.FILES))
        self.assertEqual(out["revision"], D.PIN)

    def test_a_source_edit_moves_the_hash(self):
        a = json.loads(D.derive(fixture()))["sha256"]
        c = json.loads(D.derive(fixture(turing=('return "HUBCLIENT_HOST";', 'return "HUBCLIENT_HOST"; '))))["sha256"]
        self.assertNotEqual(a[F_TURING], c[F_TURING])
        self.assertEqual({k: v for k, v in a.items() if k != F_TURING}, {k: v for k, v in c.items() if k != F_TURING})

    def test_the_derived_sets_are_exactly_the_sources_names(self):
        engines, clients = D.derive_names(fixture())
        self.assertEqual(clients, {"HUBCLIENT_ESC", "HUBCLIENT_ESC0", "HUBCLIENT_HOST_CPU"})
        self.assertEqual(
            engines,
            {"GR_HOST0", "GR_HOST1", "GR0_PBDMA0", "GR0_PBDMA1", "GR1_PBDMA0", "GR1_PBDMA1", "HOST0", "HOST1", "HOST2"},
        )

    def test_no_name_is_added_by_hand(self):
        # A name absent from every source text cannot appear: the output is a function of the texts.
        engines, clients = D.derive_names(fixture())
        self.assertNotIn("HOST3", engines)
        self.assertNotIn("HUBCLIENT_PMU", clients)


class Committed(unittest.TestCase):
    """Needs no oracle: the committed JSON against the script's constants."""

    def setUp(self):
        self.data = json.loads(JSON_PATH.read_text())

    def test_the_json_names_exactly_the_scripts_sources_and_pin(self):
        self.assertEqual(list(self.data["sha256"]), list(D.FILES))
        self.assertEqual(self.data["revision"], D.PIN)

    def test_the_turing_sources_are_among_the_scripts_inputs(self):
        for path in (
            "src/nvidia/src/kernel/gpu/fifo/arch/turing/kernel_fifo_tu102.c",
            "src/common/inc/swref/published/turing/tu102/dev_fault.h",
            "src/common/inc/swref/published/turing/tu102/hwproject.h",
        ):
            self.assertIn(path, D.FILES)
            self.assertIn(path, self.data["sha256"])

    def test_turing_pair_is_present(self):
        self.assertIn("HOST0", self.data["engines"])
        self.assertIn("HUBCLIENT_HOST_CPU", self.data["clients"])

    def test_every_family_is_present(self):
        engines, clients = set(self.data["engines"]), set(self.data["clients"])
        self.assertLessEqual({"GR_HOST0", "GR0_PBDMA0", "HOST0", "HOST11"}, engines)
        self.assertLessEqual({"HUBCLIENT_ESC", "HUBCLIENT_ESC11", "HUBCLIENT_HOST_CPU"}, clients)

    def test_lists_are_sorted_and_unique(self):
        for key in ("engines", "clients"):
            self.assertEqual(self.data[key], sorted(set(self.data[key])))


def oracle_available():
    repo = ROOT / "third_party/ogkm-580"
    try:
        subprocess.run(["git", "cat-file", "-e", D.PIN + "^{commit}"], cwd=repo, check=True, capture_output=True)
        return True
    except (OSError, subprocess.CalledProcessError):
        return False


class Oracle(unittest.TestCase):
    """Reads the real pin. KF_REQUIRE_ORACLES=1: a missing oracle is a failure, never a skip."""

    @classmethod
    def setUpClass(cls):
        if not oracle_available():
            if os.environ.get("KF_REQUIRE_ORACLES") == "1":
                raise AssertionError("third_party/ogkm-580 at the pin is missing and KF_REQUIRE_ORACLES=1")
            raise unittest.SkipTest("third_party/ogkm-580 not fetched (scripts/ci/fetch_oracles.sh)")
        cls.sources = D.read_sources()

    def test_committed_json_is_the_derivation(self):
        self.assertEqual(JSON_PATH.read_text(), D.derive(self.sources))

    def test_falsifier_the_pin_prints_the_pair_the_tu106_run_logged(self):
        # RTX 2060 (TU106), release run 20261116 leg 2: ENGINE HOST0 HUBCLIENT_HOST_CPU. If the pinned
        # sources did not print this pair the derivation would be wrong, and a hand-added row would not
        # be the remedy.
        engines, clients = D.derive_names(self.sources)
        self.assertIn("HOST0", engines)
        self.assertIn("HUBCLIENT_HOST_CPU", clients)

    def test_turing_table_is_the_twelve_pbdma_names(self):
        engines, _ = D.derive_names(self.sources)
        self.assertEqual({e for e in engines if e.startswith("HOST")}, {"HOST%d" % i for i in range(12)})

    def test_turing_sources_are_read_at_the_pin(self):
        text = self.sources[F_TURING].decode()
        self.assertIn("kfifoPrintPbdmaId_TU102", text)
        self.assertIn('return "HUBCLIENT_HOST_CPU";', text)


if __name__ == "__main__":
    unittest.main()
