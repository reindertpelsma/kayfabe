# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free coverage meter regression tests."""
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
sys.path.insert(0, os.environ.get("KF_COVERAGE_METER_MODULES", str(HERE)))

from coverage_meter import __main__ as meter
from coverage_meter.evidence import parse
from coverage_meter.interface import Definition, extract


def rows(source):
    return {key: (counts, issued) for key, counts, issued in parse(source.splitlines()).rows()}


class Evidence(unittest.TestCase):
    def test_real_census_keeps_rpc_control_class_and_host_escape_separate(self):
        report = rows((HERE / "fixtures/coverage_meter/census.log").read_text())
        self.assertEqual(report["rpc", None, 76][0]["unserviced"], 1)
        self.assertEqual(report["control", 0x2080, 0x20800173][0]["non_ok_status"], 1)
        self.assertEqual(report["class", None, 0x402c][0]["non_ok_status"], 1)
        self.assertEqual(report["host_escape", None, 0xf0][0]["refused"], 1)
        self.assertNotIn(("control", 0, 76), report)
        self.assertEqual(report["control", 0x2080, 0x20800a36][0]["serviced"], 2)
        self.assertEqual(report["control", 0x2080, 0x20800a41][0]["serviced"], 2)

    def test_trace_and_legacy_reply_preserve_nonzero_refusals(self):
        report = rows(
            "kf-rm: rpc-trace fn=76 RmControl seq=0 cmd=0x20800301 result=0x56\n"
            "nk: nvkvm: control 0x20800301 result 0x56 x2\n"
            "nk: nvkvm: control 0x20800301 result 0x00 x1"
        )
        counts, issued = report["control", 0x2080, 0x20800301]
        self.assertEqual((counts["refused"], counts["serviced"], issued), (2, 1, 3))

    def test_alloc_refusal_is_not_a_success(self):
        report = rows("kf-rm: rpc-trace fn=103 RmAlloc class=0xc5b5 result=0x56")
        counts, issued = report["class", None, 0xc5b5]
        self.assertEqual((counts["refused"], counts["serviced"], issued), (1, 0, 1))

    def test_heartbeat_counts_use_per_status_high_water_and_overlap_unserviced(self):
        report = rows(
            "kf3: GSP REFUSED fn76/0x20800173=0x56 (GSP_RM_CONTROL)\n"
            "kf3: unserviced=[0x20800173,0x20800a38,fn77] "
            "gsp_refusals[total=7 distinct=2 [fn76/0x20800173=0x56x6 fn76/0x20800173=0x1fx1]]\n"
            "kf3: unserviced=[0x20800173,0x20800a38,fn77] "
            "gsp_refusals[total=9 distinct=2 [fn76/0x20800173=0x56x8 fn76/0x20800173=0x1fx1]]"
        )
        counts, issued = report["control", 0x2080, 0x20800173]
        self.assertEqual((counts["non_ok_status"], counts["unserviced"], issued), (9, 1, 9))
        self.assertEqual(report["control", 0x2080, 0x20800a38][1], 1)
        self.assertEqual(report["rpc", None, 77][1], 1)

    def test_code_count_and_no_reply_do_not_invent_control_ids(self):
        report = rows(
            "unserviced 945 (code 76)\n"
            "kf3: GSP rpc UNSERVICED Unserviced { code: 76, sequence: 0 }\n"
            "kf-rm: rpc-trace fn=76 RmControl seq=0 cmd=0xa06c0103 result=none"
        )
        self.assertEqual(set(report), {("rpc", None, 76), ("control", 0xa06c, 0xa06c0103)})
        self.assertEqual(report["rpc", None, 76][1], 945)
        self.assertEqual(report["control", 0xa06c, 0xa06c0103][0]["unserviced"], 1)

    def test_passthrough_birth_is_only_attributed_with_matching_channel_facts(self):
        report = rows(
            "kf-rm: chanlink: CHAN-FACTS 0x1:0x2 parent=0x1 class=0xc56f engine=Some(1)\n"
            "kf3: chan 0x1:0x2 BORN Passthrough: token 0x3\n"
            "kf3: chan 0x1:0x4 BORN Passthrough: token 0x5"
        )
        self.assertEqual(report["class", None, 0xc56f][0]["passthrough"], 1)
        self.assertEqual(report["class", None, 0xc56f][1], 1)
        self.assertEqual(report["route", None, 0][0]["passthrough"], 1)

    def test_fabrication_and_unattributed_faults(self):
        report = rows(
            "kf-rm: WINDOWS PROFILE NV0073 0x00730282: NV_OK — FABRICATED answer\n"
            "kf-rm: rpc-trace fn=76 RmControl seq=1 cmd=0x00730282 result=0x0\n"
            "NVRM: Xid (withheld): 31, details\n"
            "kf3: ring REFUSED SwMethod { method: 256 }\n"
            "kf3: ring REFUSED ForeignClass { subch: 4, class: 50613 }"
        )
        self.assertEqual(report["control", 0x73, 0x00730282][0]["fabricated"], 1)
        self.assertEqual(report["control", 0x73, 0x00730282][1], 1)
        self.assertEqual(report["method", None, 256][0]["refused"], 1)
        self.assertEqual(report["class", None, 50613][0]["refused"], 1)
        self.assertEqual(report["xid", None, 31][0]["observed"], 1)


class Interface(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        files = {
            "src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080gpu.h": (
                "#define NV2080_CTRL_CMD_TEST \\\n (0x20800123U) /* a cmd */\n"
                "#define NV2080_CTRL_CMD_ALIAS NV2080_CTRL_CMD_TEST\n"
                "#define NV2080_CTRL_CMD_TEST_FLAGS 0x4\n"
                "#define NV2080_CTRL_CMD_TEST_RANGE 5:4\n"
                "#define NV2080_CTRL_CMD_TEST_UNKNOWN MAKE_ID(7)\n"
                "#define NV0000_CTRL_CMD_ROOT_TEST 0x201\n"
            ),
            "src/common/sdk/nvidia/inc/class/cl2080.h": "#define NV20_SUBDEVICE_0 (0x2080U)\n",
            "src/common/sdk/nvidia/inc/class/cla0b5.h": (
                "#define KEPLER_DMA_COPY_A (0x0000A0B5)\n"
                "#define NVA0B5_SET_OBJECT (0x00000000)\n"
                "#define NVA0B5_NOP (0x00000100)\n"
                "#define NVA0B5_NOP_PARAMETER 31:0\n"
                "#define NVA0B5_SLOT(i) (0x00000200 + (i)*4)\n"
            ),
            "src/nvidia/generated/g_allclasses.h": "#define NV01_EVENT_OS_EVENT (0x00000079)\n",
            "src/nvidia/arch/nvalloc/unix/include/nv_escape.h": "#define NV_ESC_RM_CONTROL 0x2a\n",
            "kernel-open/common/inc/nv-ioctl-numbers.h": (
                "#define NV_IOCTL_BASE 200\n#define NV_ESC_CARD_INFO (NV_IOCTL_BASE + 0)\n"
            ),
            "kernel-open/nvidia-uvm/uvm_ioctl.h": "#define UVM_TEST UVM_IOCTL_BASE(42)\n",
            "kernel-open/nvidia-uvm/uvm_test_ioctl.h": "#define UVM_TEST_CASE UVM_TEST_IOCTL_BASE(8)\n",
        }
        for path, content in files.items():
            target = root / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(content)
        self.entries, self.unresolved = extract(root, "test")

    def test_subdevice_control_with_alias(self):
        self.assertEqual(self.entries["control", 0x2080, 0x20800123], {
            Definition("NV2080_CTRL_CMD_TEST", "test"), Definition("NV2080_CTRL_CMD_ALIAS", "test")
        })
        self.assertNotIn(("control", 0, 4), self.entries)
        self.assertEqual(self.unresolved, {"NV2080_CTRL_CMD_TEST_UNKNOWN"})

    def test_root_control_has_root_class(self):
        self.assertEqual(self.entries["control", 0, 0x201], {
            Definition("NV0000_CTRL_CMD_ROOT_TEST", "test")
        })

    def test_methods_are_class_qualified_and_generated_classes_are_included(self):
        self.assertIn(("class", None, 0x79), self.entries)
        self.assertIn(("class", None, 0xa0b5), self.entries)
        self.assertIn(("method", 0xa0b5, 0), self.entries)
        self.assertIn(("method", 0xa0b5, 0x100), self.entries)
        self.assertIn(("method", 0xa0b5, 0x200), self.entries)

    def test_escapes_and_uvm_ioctl_numbers_are_distinct(self):
        self.assertIn(("escape", None, 0x2a), self.entries)
        self.assertIn(("escape", None, 200), self.entries)
        self.assertIn(("ioctl", None, 0x2a), self.entries)
        self.assertIn(("ioctl", None, 208), self.entries)


class Inputs(unittest.TestCase):
    def test_only_committed_blobs_and_one_copy_per_directory(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            path = root / "traces/run/a.log"
            path.parent.mkdir(parents=True)
            line = "kf3: GSP rpc UNSERVICED Unserviced { code: 76, sequence: 0 }\n"
            path.write_text(line)
            path.with_name("copy.txt").write_text(line)
            subprocess.run(["git", "init", "-q", tmp], check=True)
            subprocess.run(["git", "-C", tmp, "add", "."], check=True)
            subprocess.run(["git", "-C", tmp, "-c", "user.name=fixture",
                            "-c", "user.email=fixture@example.invalid", "-c", "commit.gpgsign=false",
                            "commit", "-qm", "fixture"], check=True)
            path.write_text(line * 2)
            path.with_name("untracked.log").write_text(line)
            with patch.object(meter, "ROOT", root), patch.object(meter, "git",
                                                               side_effect=lambda *a: meter_git(*a, cwd=root)):
                data = [(trace, list(census.rows())) for trace, census in meter.trace_evidence("HEAD", "traces")
                        if census is not None]
            self.assertEqual(data, [("run", [(("rpc", None, 76), {"unserviced": 1}, 1)])])

    def test_disjoint_case_windows_keep_named_counts(self):
        encoded = b'''{"schema":1,"windows":[{"case":"first","rows":[
            {"kind":"method","class":41141,"id":256,"outcome":"passthrough","count":2}]},
            {"case":"second","rows":[]}]}'''
        with patch.object(meter, "git", return_value=encoded):
            data = [(case, list(census.rows())) for case, census in meter.windows("HEAD", "traces/window.json")]
        self.assertEqual(data, [("first", [(("method", 41141, 256), {"passthrough": 2}, 2)]), ("second", [])])


meter_git = meter.git


if __name__ == "__main__":
    unittest.main()
