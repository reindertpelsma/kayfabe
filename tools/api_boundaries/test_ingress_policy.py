#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# STATUS: LIVE, 2026-11-20 (task I7; docs/design/V3_GUEST_INGRESS_TABLE.md section 5).
"""Tests of tools/api_boundaries/ingress_policy.py (GPU-free; no network).

  python3 -I -m unittest discover -s tools/api_boundaries -p 'test_ingress_policy.py' -v

- a SYNTHETIC ogkm tag that is not in the checked-in set (renumbered flag bits, a new class, a
  renumbered control) gives a complete result with no manual step;
- the committed table, facts and report are what the generator makes of the committed inputs;
- every served control is, at every measured host tag, admitted, refused by name, or reported, and
  never two of these;
- with an ogkm oracle tree present (KF_OGKM_580 / KF_OGKM_595, or third_party/ogkm-580|595), the
  committed facts at that tag are re-extracted byte for byte; KF_REQUIRE_ORACLES=1 makes a missing
  tree a failure.
"""
import json
import os
import shutil
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.normpath(os.path.join(HERE, "../.."))
sys.path.insert(0, HERE)
import ingress_policy as ip  # noqa: E402

# A newer tag's layout: the RMCTRL bit values are renumbered again (as R555 did), so a decoder that
# used fixed masks would misread every flag; one class and one control are new; 0x20800101 changed number.
CONTROL_H = """
#define RMCTRL_FLAGS_NONE              0x000000000
#define RMCTRL_FLAGS_KERNEL_PRIVILEGED 0x000000000
#define RMCTRL_FLAGS_NON_PRIVILEGED    0x000000100
#define RMCTRL_FLAGS_PRIVILEGED        0x000000200
#define RMCTRL_FLAGS_INTERNAL          0x000000400
#define RMCTRL_FLAGS_ROUTE_TO_PHYSICAL 0x000000001
#define RMCTRL_FLAGS_RM_TEST_ONLY_CODE 0x000000800
"""


def export(flags, mid, cls, access=0):
    return """    {
        /*pFunc=*/      (void (*)(void)) NULL,
        /*flags=*/      0x%xu,
        /*accessRight=*/0x%xu,
        /*methodId=*/   0x%08xu,
        /*paramSize=*/  0,
        /*pClassInfo=*/ &(__nvoc_class_def_%s.classInfo),
    },
""" % (flags, access, mid, cls)


NVOC = "".join([
    export(0x100, 0x20800102, "Subdevice"),        # served, non-privileged -> allow
    export(0x200, 0x20800103, "Subdevice"),        # served, privileged -> refuse
    export(0x500, 0x20800104, "Subdevice"),        # served, non-priv + internal -> refuse Internal
    export(0x001, 0x20800105, "Subdevice"),        # served, no visibility flag -> KernelOnly
    export(0x900, 0x20800106, "Subdevice"),        # served, test-only -> TestOnly
    export(0x100, 0x20800107, "Subdevice"),        # NOT served -> no row
    export(0x100, 0x20800201, "Subdevice"),        # served by NAME only (renumbered at this tag)
    export(0x100, 0x20800108, "Subdevice", 0x2),   # served, non-privileged, needs RS_ACCESS_NICE -> AccessRight
    export(0x100, 0x20800109, "Subdevice", 0x14),  # needs RS_ACCESS_DEBUG plus a bit no right names
])
# rs_access.h's access rights (an index each; an export's accessRight is a MASK of NVBIT(index)).
RS_ACCESS_H = """
#define RS_ACCESS_DUP_OBJECT 0U
#define RS_ACCESS_NICE       1U
#define RS_ACCESS_DEBUG      2U
#define RS_ACCESS_PERFMON    3U
#define RS_ACCESS_COUNT      4U
"""
CTRL_H = """
#define NV2080_CTRL_CMD_A (0x20800102U) /* finn: Evaluated from "x" */
#define NV2080_CTRL_CMD_B (0x20800103U) /* finn: Evaluated from "x" */
#define NV2080_CTRL_CMD_C (0x20800104U) /* finn: Evaluated from "x" */
#define NV2080_CTRL_CMD_D (0x20800105U) /* finn: Evaluated from "x" */
#define NV2080_CTRL_CMD_E (0x20800106U) /* finn: Evaluated from "x" */
#define NV2080_CTRL_CMD_F (0x20800107U) /* finn: Evaluated from "x" */
#define NV2080_CTRL_CMD_NICE (0x20800108U) /* finn: Evaluated from "x" */
#define NV2080_CTRL_CMD_DEBUGX (0x20800109U) /* finn: Evaluated from "x" */
#define NV2080_CTRL_CMD_RENUMBERED (0x20800201U) /* finn: Evaluated from "x" */
#define NV2080_CTRL_X_NO_CMD_WORD (0x20800301U) /* finn: Evaluated from "x" */
"""
CLASS_H = """
#define NV01_ROOT (0x00000000)
#define NEW_COMPUTE_Z (0x0000e9c0)
#define PRIV_THING (0x0000e9c1)
#define KERN_THING (0x0000e9c2)
#define NOFLAG_THING (0x0000e9c3)
"""


def rs(ext, internal, flags):
    return """RS_ENTRY(
    /* External Class         */ %s,
    /* Internal Class         */ %s,
    /* Multi-Instance         */ NV_TRUE,
    /* Parents                */ RS_LIST(classId(Subdevice)),
    /* Alloc Param Info       */ RS_NONE,
    /* Resource Free Priority */ RS_FREE_PRIORITY_DEFAULT,
    /* Flags                  */ %s,
    /* Required Access Rights */ RS_ACCESS_NONE
)
""" % (ext, internal, flags)


RESOURCE_LIST = "".join([
    rs("NV01_ROOT", "RmClientResource", "RS_FLAGS_ACQUIRE_GPUS_LOCK_ON_ALLOC"),
    rs("NEW_COMPUTE_Z", "KernelGraphicsObject", "RS_FLAGS_ALLOC_NON_PRIVILEGED | RS_FLAGS_ACQUIRE_GPUS_LOCK"),
    rs("PRIV_THING", "Thing", "RS_FLAGS_ALLOC_PRIVILEGED"),
    rs("KERN_THING", "Thing", "RS_FLAGS_ALLOC_KERNEL_PRIVILEGED"),
    rs("NOFLAG_THING", "Thing", "RS_FLAGS_ACQUIRE_GPUS_LOCK"),
])

SYN_TAG = "999.1.0"


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


def synthetic_tree(root):
    s = os.path.join(root, "src")
    write(os.path.join(s, "nvidia/inc/kernel/rmapi/control.h"), CONTROL_H)
    write(os.path.join(s, "nvidia/generated/g_subdevice_nvoc.c"), NVOC)
    write(os.path.join(s, "nvidia/src/kernel/rmapi/resource_list.h"), RESOURCE_LIST)
    write(os.path.join(s, "common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080x.h"), CTRL_H)
    write(os.path.join(s, "common/sdk/nvidia/inc/class/clzzzz.h"), CLASS_H)
    write(os.path.join(s, "common/sdk/nvidia/inc/nvos.h"), "#define NV01_EVENT (0x00000005)\n")
    write(os.path.join(s, "common/sdk/nvidia/inc/rs_access.h"), RS_ACCESS_H)


def synthetic_repo(root, rules=""):
    """A repo holding only the generator's inputs, with SYN_TAG as its one tag."""
    write(os.path.join(root, ip.RANGES), "# kind\titem\n# tags\t%s\n" % SYN_TAG)
    served = {"ctrl_ids": ["0x20800102", "0x20800103", "0x20800104", "0x20800105", "0x20800106",
                           "0x20800108", "0x2080dead"],
              "ctrl_names": ["NV2080_CTRL_CMD_RENUMBERED"],
              "class_id_literals": ["0xe9c1", "0xe9c2", "0xe9c3", "0x1234"],
              "classes": {"NV01_ROOT": 0},
              "upper_idents": ["NEW_COMPUTE_Z"]}
    write(os.path.join(root, ip.SERVED), json.dumps(served))
    write(os.path.join(root, ip.RULES), "# header\n" + rules)
    write(os.path.join(root, ip.ANSWERED), "0x20800103\n")


def verdicts(repo, facts_dir):
    tags, out, report, _answered, _rules = ip.compute(repo, facts_dir)
    v = {(k, i): pt[0][2] for k in out for i, pt in out[k].items()}
    return tags, v, report


class SyntheticNewTag(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.tree = os.path.join(self.tmp, "ogkm")
        self.repo = os.path.join(self.tmp, "repo")
        self.facts = os.path.join(self.tmp, "facts")
        synthetic_tree(self.tree)

    def tearDown(self):
        shutil.rmtree(self.tmp)

    def run_all(self, rules=""):
        synthetic_repo(self.repo, rules)
        ip.main(["extract", "--tree", "%s=%s" % (SYN_TAG, self.tree), "--out", self.facts,
                 "--repo", self.repo, "--digest"])
        ip.main(["emit", "--repo", self.repo])
        return verdicts(self.repo, None)

    def test_a_new_tag_needs_only_its_tree(self):
        tags, v, report = self.run_all()
        self.assertEqual(tags, [SYN_TAG])
        self.assertEqual(v[("control", 0x20800102)], "OgkmNonPrivileged")
        self.assertEqual(v[("control", 0x20800201)], "OgkmNonPrivileged", "served by name, resolved at the tag")
        self.assertTrue(os.path.exists(os.path.join(self.repo, ip.OUT_RS)))
        self.assertEqual(ip.main(["check", "--repo", self.repo]), 0)

    def test_renumbered_flag_bits_are_decoded_by_name(self):
        _, v, _ = self.run_all()
        self.assertEqual(v[("control", 0x20800103)], "Privileged")
        self.assertEqual(v[("control", 0x20800104)], "Internal")
        self.assertEqual(v[("control", 0x20800105)], "KernelOnly")
        self.assertEqual(v[("control", 0x20800106)], "TestOnly")

    def test_privileged_is_never_admitted(self):
        _, v, _ = self.run_all()
        for k in [("control", 0x20800103), ("class", 0xe9c1), ("class", 0xe9c2)]:
            self.assertFalse(v[k].startswith("Ogkm"), k)

    def test_classes_follow_resource_list(self):
        _, v, _ = self.run_all()
        self.assertEqual(v[("class", 0xe9c0)], "OgkmNonPrivileged")
        self.assertEqual(v[("class", 0xe9c1)], "Privileged")
        self.assertEqual(v[("class", 0xe9c2)], "KernelOnly")
        self.assertEqual(v[("class", 0xe9c3)], "NoPrivilegeFlag")
        self.assertEqual(v[("class", 0x0000)], "OgkmClientRoot")
        self.assertNotIn(("class", 0x1234), v, "a literal that is no ogkm class is not a class")

    def test_unserved_has_no_row_and_unknown_is_reported_and_refused(self):
        _, v, report = self.run_all()
        self.assertNotIn(("control", 0x20800107), v)
        self.assertNotIn(("control", 0x2080dead), v)
        self.assertIn(("control", 0x2080dead), {(k, i) for k, i, what, *_ in report if what == "unknown-to-ogkm"})

    def test_a_control_that_needs_an_access_right_is_refused_by_derivation(self):
        # Security review of PR #545, F2: ogkm's accessRight is a mask the caller must hold; a NON_PRIVILEGED
        # control that names one is refused by the derivation, so no judgement row has to carry it.
        _, v, _ = self.run_all()
        self.assertEqual(v[("control", 0x20800108)], "AccessRight")
        facts = json.load(open(os.path.join(self.facts, SYN_TAG + ".json")))
        self.assertEqual(facts["controls"]["0x20800108"]["exports"][0]["access"], ["NICE"])
        self.assertEqual(facts["controls"]["0x20800102"]["exports"][0]["access"], [])

    def test_the_access_mask_is_decoded_by_the_tags_own_right_names(self):
        self.run_all()
        facts = json.load(open(os.path.join(self.facts, SYN_TAG + ".json")))
        self.assertEqual(facts["controls"]["0x20800109"]["exports"][0]["access"], ["DEBUG", "UNKNOWN"])
        self.assertEqual(ip.control_verdict(facts["controls"]["0x20800109"]), (False, "AccessRight"))

    def test_rule_rows_override_with_their_basis(self):
        rules = ("control\t0x20800103\tNV2080_CTRL_CMD_B\tallow\tMode2Rpc\t-\t-\twhy\tev\n"
                 "control\t0x20800102\tNV2080_CTRL_CMD_A\trefuse\tHostScope\t-\t-\twhy\tev\n")
        _, v, _ = self.run_all(rules)
        self.assertEqual(v[("control", 0x20800103)], "Mode2Rpc")
        self.assertEqual(v[("control", 0x20800102)], "Judgement:HostScope")

    def test_a_misnamed_rule_row_is_rejected(self):
        with self.assertRaises(SystemExit):
            self.run_all("control\t0x20800103\tWRONG_NAME\tallow\tMode2Rpc\t-\t-\twhy\tev\n")

    def test_the_rust_table_names_the_answered_set(self):
        self.run_all()
        rs = open(os.path.join(self.repo, ip.OUT_RS)).read()
        self.assertIn("SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later", rs)
        self.assertIn("id: 0x20800103,\n        forwarded: false,", rs)
        self.assertIn("id: 0x20800102,\n        forwarded: true,", rs)


class TwoTags(unittest.TestCase):
    """A control that a later tag adds is refused, by its later name, at the earlier tag."""

    def test_absent_at_the_earlier_tag(self):
        tmp = tempfile.mkdtemp()
        try:
            old, new, repo = (os.path.join(tmp, x) for x in ("old", "new", "repo"))
            synthetic_tree(new)
            synthetic_tree(old)
            write(os.path.join(old, "src/nvidia/generated/g_subdevice_nvoc.c"),
                  NVOC.replace(export(0x100, 0x20800102, "Subdevice"), ""))
            synthetic_repo(repo)
            write(os.path.join(repo, ip.RANGES), "# tags\t999.0.0 %s\n" % SYN_TAG)
            ip.main(["extract", "--tree", "999.0.0=" + old, "--tree", SYN_TAG + "=" + new,
                     "--out", os.path.join(tmp, "facts"), "--repo", repo, "--digest"])
            tags, out, _report, _a, _r = ip.compute(repo, None)
            self.assertEqual(tags, ["999.0.0", SYN_TAG])
            self.assertEqual([(n, ok, why) for n, ok, why in out["control"][0x20800102]],
                             [("NV2080_CTRL_CMD_A", False, "NotExported"),
                              ("NV2080_CTRL_CMD_A", True, "OgkmNonPrivileged")])
        finally:
            shutil.rmtree(tmp)


class CommittedTable(unittest.TestCase):
    def test_committed_outputs_are_current(self):
        self.assertEqual(ip.main(["check", "--repo", REPO]), 0)

    def test_served_controls_are_partitioned_at_every_measured_host_tag(self):
        tags, out, report, _a, _r = ip.compute(REPO, None)
        facts, _ = ip.load_facts(REPO)
        served, _cls = ip.served_sets(json.load(open(os.path.join(REPO, ip.SERVED))), facts)
        unknown = {i for k, i, what, *_ in report if k == "control" and what == "unknown-to-ogkm"}
        hosts = set()
        for ln in open(os.path.join(HERE, "measured_points.tsv")):
            f = ln.rstrip("\n").split("\t")
            if f[1] == "host" and f[6] in tags:
                hosts.add(f[6])
        self.assertGreaterEqual(len(hosts), 5)
        for t in sorted(hosts):
            ti = tags.index(t)
            admitted = {i for i, pt in out["control"].items() if pt[ti][1]}
            refused = {i for i, pt in out["control"].items() if not pt[ti][1]}
            self.assertFalse(admitted & refused)
            self.assertFalse((admitted | refused) & (unknown - {i for i in out["control"]}))
            missing = served - admitted - refused - unknown
            self.assertEqual(missing, set(), "served ids with no verdict at %s" % t)
            for i in admitted:
                self.assertTrue(pt_ok(out["control"][i][ti]), (t, hex(i)))


class AccessRightDerivation(unittest.TestCase):
    """Security review of PR #545, F2: the three served controls that need RS_ACCESS_NICE are refused by
    the derivation itself, so deleting their judgement rows (mutant M3: refuse rows ignored) admits nothing."""

    NICE = {0x20801115, 0xa06f0109, 0xa06f0111}

    def test_the_nice_controls_are_refused_without_their_rule_rows(self):
        orig = ip.read_rules
        ip.read_rules = lambda repo: [r for r in orig(repo) if r["verdict"] != "refuse"]
        try:
            tags, out, _report, _a, _r = ip.compute(REPO, None)
        finally:
            ip.read_rules = orig
        for i in self.NICE:
            self.assertIn(i, out["control"], hex(i))
            for t, run in zip(tags, out["control"][i]):
                self.assertEqual(run[1:3], (False, "AccessRight"), (hex(i), t))

    def test_every_access_right_in_the_facts_is_a_refusal(self):
        facts, _ = ip.load_facts(REPO)
        seen = 0
        for tag, f in facts.items():
            for cid, e in f["controls"].items():
                if any(x["access"] for x in e["exports"]):
                    seen += 1
                    self.assertEqual(ip.control_verdict(e), (False, "AccessRight"), (tag, cid))
        self.assertGreaterEqual(seen, 5 * 30, "the committed facts carry the access rights of all 30 tags")


class ServedFreshness(unittest.TestCase):
    """Security review of PR #541, F3, and of PR #545, F7: served.json is an input of the gate and of the
    control partition (crates/kf-abi/tests/control_partition.rs), and only run_all.sh regenerated it.
    Re-derive it from the tree and require every field the partition reads to be current. `upper_idents`
    and `class_id_literals` move with any new identifier (most of which no ogkm tag knows), so they are
    compared through their consumer: the generator must make the same table and report from the fresh
    file as from the committed one."""

    def fresh(self):
        import subprocess
        out = subprocess.run(
            [sys.executable, "-I", os.path.join(HERE, "derive_served.py"), REPO,
             os.path.join(HERE, "extra_ids.txt"), os.path.join(HERE, "windows_ids.txt")],
            capture_output=True, text=True, check=True).stdout
        return json.loads(out)

    def test_served_json_is_current_where_it_is_read(self):
        fresh = self.fresh()
        committed = json.load(open(os.path.join(REPO, ip.SERVED)))
        for k in ("ctrl_ids", "ctrl_names", "classes", "alloc_structs"):
            self.assertEqual(fresh[k], committed[k], "%s in served.json is stale: run "
                             "tools/api_boundaries/derive_served.py and commit the result" % k)
        tmp = tempfile.mkdtemp()
        try:
            path = os.path.join(tmp, "served.json")
            with open(path, "w") as f:
                json.dump(fresh, f)
            before, ip.SERVED = ip.SERVED, path
            try:
                from_fresh = ip.compute(REPO, None)
            finally:
                ip.SERVED = before
        finally:
            shutil.rmtree(tmp)
        self.assertEqual(from_fresh, ip.compute(REPO, None),
                         "the table generated from a fresh served.json differs from the committed one: "
                         "regenerate served.json (derive_served.py), re-emit, commit both")


def pt_ok(run):
    return run[1] and run[2] in ("OgkmNonPrivileged", "Mode2Rpc", "Empirical")


class DeterministicExtraction(unittest.TestCase):
    """One id defined twice (the 0xffffffff "no override" macros) must not depend on directory order.

    CI found it: the same oracle tree gave a different name for control 0xffffffff in a checkout
    whose directory order differed from the one the committed facts were made in."""

    def test_keep_smallest_is_order_independent(self):
        for order in ((("a", 3), ("a", 1), ("a", 2)), (("a", 1), ("a", 2), ("a", 3)), (("a", 2), ("a", 3), ("a", 1))):
            sink = {}
            for k, v in order:
                ip._keep_smallest(sink, k, v)
            self.assertEqual(sink, {"a": 1}, order)

    def test_keep_smallest_orders_names_not_insertion(self):
        first, second = {}, {}
        for n in ("NVC372_X", "NV0000_Y", "NV0000_X"):
            ip._keep_smallest(first, 0xFFFFFFFF, n)
        for n in ("NV0000_X", "NV0000_Y", "NVC372_X"):
            ip._keep_smallest(second, 0xFFFFFFFF, n)
        self.assertEqual(first, second)
        self.assertEqual(first, {0xFFFFFFFF: "NV0000_X"})

    def test_distinct_keys_are_kept(self):
        sink = {}
        ip._keep_smallest(sink, 1, "B")
        ip._keep_smallest(sink, 2, "A")
        self.assertEqual(sink, {1: "B", 2: "A"})


class OracleTrees(unittest.TestCase):
    def check_tag(self, env, sub, tag):
        tree = os.environ.get(env) or os.path.join(REPO, "third_party", sub)
        if not os.path.isfile(os.path.join(tree, "src/nvidia/src/kernel/rmapi/resource_list.h")):
            if os.environ.get("KF_REQUIRE_ORACLES") == "1":
                self.fail("ORACLE-GATE: %s missing and KF_REQUIRE_ORACLES=1" % sub)
            print("ORACLE-GATE: SKIPPED %s: no tree (set %s or run scripts/ci/fetch_oracles.sh) - NOT a pass"
                  % (tag, env))
            return
        fresh = ip._fact_rows(ip.extract_tree(tree, tag))
        self.assertEqual(fresh, ip.facts_at(REPO, tag), "ingress_facts.tsv at %s differs from the tree" % tag)

    def test_extraction_does_not_depend_on_directory_order(self):
        tree = os.environ.get("KF_OGKM_580") or os.path.join(REPO, "third_party", "ogkm-580")
        if not os.path.isfile(os.path.join(tree, "src/nvidia/src/kernel/rmapi/resource_list.h")):
            if os.environ.get("KF_REQUIRE_ORACLES") == "1":
                self.fail("ORACLE-GATE: ogkm-580 missing and KF_REQUIRE_ORACLES=1")
            print("ORACLE-GATE: SKIPPED order test: no tree - NOT a pass")
            return
        normal = ip._fact_rows(ip.extract_tree(tree, "580.159.04"))
        real_walk = os.walk

        def reversed_walk(top, *a, **k):
            for dp, dn, fn in real_walk(top, *a, **k):
                dn.reverse()
                fn.reverse()
                yield dp, dn, fn

        os.walk = reversed_walk
        try:
            flipped = ip._fact_rows(ip.extract_tree(tree, "580.159.04"))
        finally:
            os.walk = real_walk
        self.assertEqual(normal, flipped)

    def test_580_159_04(self):
        self.check_tag("KF_OGKM_580", "ogkm-580", "580.159.04")

    def test_595_84(self):
        self.check_tag("KF_OGKM_595", "ogkm-595", "595.84")


if __name__ == "__main__":
    unittest.main()
