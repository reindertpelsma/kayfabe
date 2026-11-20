#!/usr/bin/env python3
"""GPU-free tests of the open ML rows (vllm, tensorrt, jax): scripts/apps/mlrows/ and its wiring in the Linux app matrix (docs/design/V3_OPEN_ML_ROWS.md).

FALSIFIER (written before the code, 2026-11-17): a row that passes while the application ran on the CPU, while it installed something no hash anchors, while it read a
model the origin does not list for the pinned revision, or while the guest differs from the host. Each of those has a test below that fails when the protection is
removed; scripts/ci/mlrows_mutants.py removes each protection in turn and needs two named tests to notice.

Fixtures (scripts/ci/fixtures/mlrows/): the origin's real tree listing of the pinned model revision, real nvidia-smi output of the native runs, the real result lines of
the native runs (README.md there says which is which). No GPU UUID anywhere (the fake prefix fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e.... is the only one allowed).
"""
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
ML = ROOT / "scripts" / "apps" / "mlrows"
FIX = HERE / "fixtures" / "mlrows"
GUFIX = HERE / "fixtures" / "gpu_use"


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


sys.path.insert(0, str(ML))
lib = load("mlrows_lib", ML / "mlrows_lib.py")
mr = load("mlrows_runner", ML / "mlrows.py")
gu = load("kf_gpu_use_ml", ROOT / "scripts" / "bench" / "gpu_use.py")
gur = load("kf_gpu_use_rows_ml", ROOT / "scripts" / "apps" / "gpu_use_rows.py")
PINS = lib.load_pins()
NOHASH = json.loads(json.dumps(PINS))
NOHASH["rows"]["vllm"]["expected_token_sha256"] = None          # the tests with made-up token ids run without the committed hash; the real run has its own tests


def fx(name):
    return (FIX / name).read_text()


def gfx(name):
    return (GUFIX / name).read_text()


def facts(**kw):
    base = {"gpus": 1, "name": "NVIDIA GeForce RTX 3060", "driver": "580.159.03", "driver_cuda": (13, 0), "cc": (8, 6), "total_mib": 12288, "python": (3, 12, 3), "gcc": True, "python_h": True, "disk_free_gb": 500.0}
    base.update(kw)
    return base


# ------------------------------------------------------------------------------------------------ pins and the trust rule
class Pins(unittest.TestCase):
    def test_every_row_has_flavours_with_a_lock_that_exists(self):
        self.assertEqual(sorted(PINS["rows"]), sorted(lib.ROWS))
        for row, spec in PINS["rows"].items():
            self.assertTrue(spec["flavours"], row)
            for name, fl in spec["flavours"].items():
                self.assertTrue((ML / fl["lock"]).is_file(), "%s %s lock %s" % (row, name, fl["lock"]))
                self.assertEqual(len(fl["driver_cuda_min"]), 2)

    def test_the_model_is_pinned_by_a_git_commit_and_a_licence_and_bounded(self):
        m = PINS["model"]
        self.assertRegex(m["revision"], r"^[0-9a-f]{40}$")
        self.assertEqual(m["license"], "Apache-2.0")
        self.assertTrue(m["origin"].startswith("https://"))
        self.assertIn("model.safetensors", m["files"])
        self.assertLessEqual(m["max_bytes"], 400_000_000)

    def test_no_sha256_of_a_binary_is_pinned_in_git_outside_the_locks(self):
        """Owner ruling 2026-11-17: no sha256 of a binary we host or build in git. The locks carry the INDEX's digests; nothing else under mlrows/ may hold a 64-hex string.
        The one allowed field is the committed digest of the row's OWN OUTPUT (expected_token_sha256), which is no binary."""
        for p in sorted(ML.rglob("*")):
            if not p.is_file() or p.suffix in (".pyc",) or p.parent.name == "locks" and p.name.endswith(".lock.txt"):
                continue
            text = p.read_text(errors="replace")
            if p.name == "pins.json":
                text = re.sub(r'"expected_token_sha256":\s*"[0-9a-f]{64}"', '"expected_token_sha256": null', text)
            self.assertIsNone(re.search(r"\b[0-9a-f]{64}\b", text), "a 64-hex digest in %s" % p.relative_to(ROOT))

    def test_the_python_range_matches_the_guest_image(self):
        self.assertEqual(PINS["python"]["min"], [3, 12])
        recipe = json.loads((ROOT / "scripts/apps/image/recipe.json").read_text())
        self.assertIn("noble", recipe["base"]["id"])        # Ubuntu 24.04 ships CPython 3.12

    def test_the_oss_trtexec_pin_is_an_upstream_commit(self):
        t = PINS["rows"]["tensorrt"]["trtexec"]
        self.assertRegex(t["oss_commit"], r"^[0-9a-f]{40}$")
        self.assertFalse(t["in_pypi_wheels"])

    def test_the_committed_token_hash_is_bound_to_the_versions_it_was_measured_with(self):
        spec = PINS["rows"]["vllm"]
        ctx = spec["expected_token_context"]
        self.assertRegex(spec["expected_token_sha256"], r"^[0-9a-f]{64}$")
        have = lib.lock_versions((ML / spec["flavours"]["cu13"]["lock"]).read_text())
        self.assertEqual((ctx["vllm"], ctx["torch"]), (have["vllm"], have["torch"]))
        self.assertEqual(ctx["model_revision"], PINS["model"]["revision"])
        self.assertGreaterEqual(len(ctx["dies"]), 2)
        self.assertEqual(ctx["dtype"], "float16")
        self.assertIn('dtype="float16"', (ML / "row_vllm.py").read_text())

    def test_pins_json_is_stable_json(self):
        text = (ML / "pins.json").read_text()
        self.assertEqual(json.loads(text), json.loads(json.dumps(json.loads(text))))
        self.assertTrue(text.endswith("\n"))


# ------------------------------------------------------------------------------------------------ the machine's facts
class Facts(unittest.TestCase):
    def test_smi_header_gives_driver_and_the_cuda_line(self):
        h = lib.parse_smi_header(fx("nvidia_smi_head_580_rtx3060.txt"))
        self.assertEqual((h["driver"], h["cuda"]), ("580.159.03", (13, 0)))

    def test_query_csv_gives_name_cc_and_memory(self):
        g = lib.parse_query_csv(fx("query_gpu_580_rtx3060.csv"))
        self.assertEqual(g, [{"name": "NVIDIA GeForce RTX 3060", "driver": "580.159.03", "cc": (8, 6), "total_mib": 12288}])

    def test_a_query_without_compute_cap_keeps_the_gpu_and_loses_the_cc(self):
        g = lib.parse_query_csv("NVIDIA GeForce RTX 3060, 535.154.05, [N/A], 12288\n")
        self.assertEqual((g[0]["cc"], g[0]["total_mib"]), (None, 12288))

    def test_garbage_gives_nothing_not_a_gpu(self):
        self.assertEqual(lib.parse_query_csv("NVIDIA-SMI has failed because it couldn't communicate with the NVIDIA driver."), [])
        self.assertEqual(lib.parse_smi_header("no such file"), {"driver": None, "cuda": None})

    def test_cuda_from_driver_table_edges(self):
        for drv, want in (("580.65.06", (13, 0)), ("580.64", (12, 0)), ("575.51.03", (12, 0)), ("525.60.13", (12, 0)), ("525.59", None), ("470.82", None), ("595.91.07", (13, 0)), (None, None)):
            self.assertEqual(lib.cuda_from_driver(drv), want, drv)

    def test_machine_facts_uses_the_header_cuda_and_falls_back_to_the_driver_table(self):
        f = lib.machine_facts(fx("nvidia_smi_head_580_rtx3060.txt"), fx("query_gpu_580_rtx3060.csv"), (3, 12, 3), True, True, 100)
        self.assertEqual((f["driver_cuda"], f["cc"], f["total_mib"], f["gpus"]), ((13, 0), (8, 6), 12288, 1))
        g = lib.machine_facts("", fx("query_gpu_580_rtx3060.csv"), (3, 12, 3))
        self.assertEqual(g["driver_cuda"], (13, 0))
        self.assertFalse(g["gcc"])


class Flavour(unittest.TestCase):
    def choose(self, row, **kw):
        return lib.choose_flavour(row, facts(**kw), PINS)

    def test_a_cuda_13_driver_takes_the_newest_flavour_of_every_row(self):
        for row in lib.ROWS:
            c = self.choose(row)
            self.assertEqual((c["status"], c["flavour"]), ("ok", "cu13"), row)

    def test_a_cuda_12_driver_takes_cu12_where_there_is_one_and_not_run_for_vllm(self):
        self.assertEqual(self.choose("jax", driver_cuda=(12, 9))["flavour"], "cu12")
        self.assertEqual(self.choose("tensorrt", driver_cuda=(12, 0))["flavour"], "cu12")
        c = self.choose("vllm", driver_cuda=(12, 9))
        self.assertEqual(c["status"], lib.NOT_RUN)
        self.assertIn("driver_cuda_below_every_wheel", c["reason"])
        self.assertIn("cu13: driver supports CUDA 12.9 < 13.0", c["reason"])

    def test_a_driver_below_cuda_12_is_not_run_for_every_row(self):
        for row in lib.ROWS:
            self.assertEqual(self.choose(row, driver_cuda=(11, 8))["status"], lib.NOT_RUN, row)

    def test_the_cuda_boundary_is_exact(self):
        self.assertEqual(self.choose("jax", driver_cuda=(13, 0))["flavour"], "cu13")
        self.assertEqual(self.choose("jax", driver_cuda=(12, 99))["flavour"], "cu12")

    def test_older_than_turing_is_not_run_with_the_cc_in_the_reason(self):
        c = self.choose("jax", cc=(7, 0), name="NVIDIA TITAN V")
        self.assertEqual(c["status"], lib.NOT_RUN)
        self.assertIn("gpu_older_than_turing: sm_70", c["reason"])
        self.assertEqual(self.choose("jax", cc=(7, 5))["status"], "ok")

    def test_an_unknown_compute_capability_is_an_env_problem_not_a_not_run(self):
        c = self.choose("jax", cc=None)
        self.assertEqual(c["status"], "env")
        self.assertIn("compute_capability_unknown", c["reason"])

    def test_an_unknown_driver_cuda_is_an_env_problem_not_a_not_run(self):
        c = self.choose("vllm", driver_cuda=None)
        self.assertEqual(c["status"], "env")

    def test_python_range(self):
        self.assertIn("python_too_old", self.choose("jax", python=(3, 11, 9))["reason"])
        self.assertIn("python_too_new", self.choose("jax", python=(3, 15, 0))["reason"])
        self.assertEqual(self.choose("jax", python=(3, 14, 1))["status"], "ok")

    def test_no_gpu_and_too_little_memory(self):
        self.assertEqual(self.choose("vllm", gpus=0, name=None)["reason"], "no_gpu_or_no_nvidia_smi")
        c = self.choose("vllm", total_mib=2048)
        self.assertEqual(c["status"], lib.NOT_RUN)
        self.assertIn("vram_below_floor: 2048 MiB < 3000 MiB", c["reason"])
        self.assertEqual(self.choose("vllm", total_mib=3000)["status"], "ok")

    def test_a_forced_flavour_is_checked_against_the_driver_and_never_overrides_a_hard_not_run(self):
        f = facts(driver_cuda=(12, 9))
        c = mr.select("jax", f, PINS, "cu13")
        self.assertEqual(c["status"], lib.NOT_RUN)
        self.assertIn("driver_cuda_below_forced_flavour", c["reason"])
        self.assertEqual(mr.select("jax", facts(), PINS, "cu12")["flavour"], "cu12")
        self.assertEqual(mr.select("jax", facts(gpus=0, name=None), PINS, "cu12")["status"], lib.NOT_RUN)
        self.assertIn("flavour_not_pinned_for_this_row", mr.select("vllm", facts(), PINS, "cu12")["reason"])


# ------------------------------------------------------------------------------------------------ the hashed locks
LOCKS = sorted((ML / "locks").glob("*.lock.txt"))


class Locks(unittest.TestCase):
    def test_there_is_a_lock_for_every_pinned_flavour_and_no_other(self):
        want = sorted(Path(fl["lock"]).name for spec in PINS["rows"].values() for fl in spec["flavours"].values())
        self.assertEqual(sorted(p.name for p in LOCKS), want)

    def test_every_committed_lock_is_sound_and_agrees_with_pins(self):
        for row, spec in PINS["rows"].items():
            for name, fl in spec["flavours"].items():
                text = (ML / fl["lock"]).read_text()
                self.assertEqual(lib.lock_problems(text, require_nvidia_index=(row == "tensorrt")), [], fl["lock"])
                self.assertEqual(lib.pins_vs_lock_problems(row, name, text, PINS), [], fl["lock"])

    def test_every_requirement_has_a_digest_and_the_locks_are_full_closures(self):
        for p in LOCKS:
            reqs, _ = lib.parse_lock(p.read_text())
            self.assertGreaterEqual(len(reqs), 12, p.name)
            for r in reqs:
                self.assertTrue(r["hashes"], "%s %s" % (p.name, r["name"]))

    def test_the_input_files_are_pinned_by_the_lock(self):
        for p in sorted((ML / "locks").glob("*.in")):
            lock = lib.lock_versions((ML / "locks" / (p.stem + ".lock.txt")).read_text())
            for line in p.read_text().splitlines():
                m = re.match(r"^([A-Za-z0-9_.-]+)(?:\[[^\]]*\])?==([^\s]+)$", line.strip())
                if m:
                    self.assertEqual(lock.get(m.group(1).lower().replace("_", "-")), m.group(2), "%s: %s" % (p.name, line))

    def test_the_locks_carry_a_status_header_with_the_command_that_made_them(self):
        for p in LOCKS:
            head = "\n".join(p.read_text().splitlines()[:4])
            self.assertIn("STATUS: HASHED", head, p.name)
            self.assertIn("uv pip compile --generate-hashes", head, p.name)
            self.assertIn("gen_locks.py", head, p.name)

    def test_only_tensorrt_locks_use_nvidia_s_index_and_only_over_https(self):
        for p in LOCKS:
            _, idx = lib.parse_lock(p.read_text())
            hosts = {re.match(r"https://([^/]+)/", u).group(1) for _, u in idx}
            self.assertEqual("pypi.nvidia.com" in hosts, p.name.startswith("tensorrt"), p.name)
            self.assertLessEqual(hosts, set(lib.ALLOWED_INDEX_HOSTS))

    def test_the_cu13_jax_lock_stays_on_the_cuda_13_0_train(self):
        v = lib.lock_versions((ML / "locks/jax_cu13.lock.txt").read_text())
        for name in ("nvidia-cuda-runtime", "nvidia-cuda-nvrtc", "nvidia-cuda-nvcc", "nvidia-nvjitlink", "nvidia-cuda-cupti"):
            self.assertTrue(v[name].startswith("13.0."), "%s %s" % (name, v[name]))

    def test_a_requirement_without_a_hash_is_refused(self):
        text = (ML / "locks/jax_cu12.lock.txt").read_text()
        broken = re.sub(r"(jaxlib==[^\n]*\\\n)(\s+--hash=sha256:[0-9a-f]{64}( \\)?\n)+", r"\1", text, count=1)
        self.assertNotEqual(broken, text)
        self.assertTrue(any("jaxlib" in p and "no --hash" in p for p in lib.lock_problems(broken)))

    def test_an_unpinned_or_url_requirement_is_refused(self):
        base = "--index-url https://pypi.org/simple\n\nfoo==1.0 \\\n    --hash=sha256:" + "a" * 64 + "\n"
        self.assertEqual(lib.lock_problems(base), [])
        for bad in ("bar>=1.0\n", "bar @ https://example.org/bar.whl\n", "git+https://example.org/x.git\n", "-e .\n"):
            self.assertTrue(lib.lock_problems(base + bad), bad)

    HASHED = "foo==1.0 \\\n    --hash=sha256:" + "a" * 64 + "\n"

    def test_a_foreign_index_is_refused(self):
        self.assertTrue(lib.lock_problems("--index-url https://pypi.org/simple\n--extra-index-url https://evil.example/simple\n" + self.HASHED))

    def test_a_plain_http_index_is_refused(self):
        self.assertTrue(lib.lock_problems("--index-url http://pypi.org/simple\n" + self.HASHED))

    def test_a_lock_without_an_index_line_is_refused(self):
        self.assertTrue(lib.lock_problems(self.HASHED))                   # no index line: pip would use the machine's

    def test_a_tensorrt_lock_without_nvidia_s_index_is_refused(self):
        self.assertTrue(lib.lock_problems("--index-url https://pypi.org/simple\n" + self.HASHED, require_nvidia_index=True))
        self.assertEqual(lib.lock_problems("--index-url https://pypi.org/simple\n--extra-index-url https://pypi.nvidia.com/\n" + self.HASHED, require_nvidia_index=True), [])

    def test_a_duplicate_name_is_refused_and_an_empty_lock_too(self):
        h = "foo==1.0 \\\n    --hash=sha256:" + "a" * 64 + "\n"
        self.assertTrue(any("twice" in p for p in lib.lock_problems("--index-url https://pypi.org/simple\n" + h + h)))
        self.assertTrue(lib.lock_problems("--index-url https://pypi.org/simple\n"))

    def test_a_pin_that_disagrees_with_the_lock_is_named(self):
        pins = json.loads(json.dumps(PINS))
        pins["rows"]["jax"]["flavours"]["cu13"]["packages"]["jax"] = "0.0.1"
        probs = lib.pins_vs_lock_problems("jax", "cu13", (ML / "locks/jax_cu13.lock.txt").read_text(), pins)
        self.assertEqual(len(probs), 1)
        self.assertIn("pins.json says 0.0.1", probs[0])

    def test_lockcheck_passes_on_the_tree(self):
        buf = io.StringIO()
        with mock.patch("sys.stdout", buf):
            rc = mr.main(["lockcheck"])
        self.assertEqual(rc, 0, buf.getvalue())
        self.assertEqual(buf.getvalue().count("problems=0"), 5)

    def test_gen_locks_names_exactly_the_committed_locks(self):
        g = load("mlrows_gen_locks", ML / "gen_locks.py")
        self.assertEqual(sorted(g.LOCKS), sorted(p.name[: -len(".lock.txt")] for p in LOCKS))
        for name in g.LOCKS:
            self.assertTrue((ML / "locks" / (name + ".in")).is_file(), name)


# ------------------------------------------------------------------------------------------------ the model
class Model(unittest.TestCase):
    tree = json.loads(fx("hf_tree_smollm2_135m.json"))

    def test_the_plan_is_the_pinned_files_with_the_origin_s_digests(self):
        plan = lib.model_plan(self.tree, PINS)
        self.assertEqual([p["path"] for p in plan], PINS["model"]["files"])
        weights = next(p for p in plan if p["path"] == "model.safetensors")
        self.assertEqual((weights["kind"], weights["size"]), ("sha256", 269060552))
        self.assertRegex(weights["digest"], r"^[0-9a-f]{64}$")
        self.assertTrue(all(p["kind"] == "git-blob" and len(p["digest"]) == 40 for p in plan if p["path"] != "model.safetensors"))

    def test_a_file_the_origin_does_not_list_is_refused(self):
        tree = [e for e in self.tree if e["path"] != "tokenizer.json"]
        with self.assertRaisesRegex(ValueError, "has no tokenizer.json"):
            lib.model_plan(tree, PINS)

    def test_a_file_without_a_digest_or_a_size_is_refused(self):
        tree = json.loads(json.dumps(self.tree))
        for e in tree:
            if e["path"] == "config.json":
                e["oid"] = "x"
        with self.assertRaisesRegex(ValueError, "no digest"):
            lib.model_plan(tree, PINS)

    def test_a_model_larger_than_pinned_is_refused(self):
        pins = json.loads(json.dumps(PINS))
        pins["model"]["max_bytes"] = 1000
        with self.assertRaisesRegex(ValueError, "more than the"):
            lib.model_plan(self.tree, pins)

    def test_an_unsafe_path_in_the_pins_is_refused(self):
        pins = json.loads(json.dumps(PINS))
        pins["model"]["files"] = ["../etc/passwd"]
        with self.assertRaisesRegex(ValueError, "unsafe"):
            lib.model_plan(self.tree, pins)

    def test_bytes_are_checked_by_size_then_by_digest(self):
        data = b"hello world\n"
        e = {"path": "x", "size": len(data), "kind": "sha256", "digest": hashlib.sha256(data).hexdigest()}
        self.assertIsNone(lib.check_model_bytes(e, data))
        self.assertIn("bytes", lib.check_model_bytes(e, data + b"x"))
        self.assertIn("sha256", lib.check_model_bytes(e, b"hello WORLD\n"))
        g = {"path": "y", "size": len(data), "kind": "git-blob", "digest": lib.git_blob_sha1(data)}
        self.assertIsNone(lib.check_model_bytes(g, data))
        self.assertIn("git-blob", lib.check_model_bytes(g, b"hello WORLD\n"))
        self.assertEqual(lib.git_blob_sha1(b""), "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391")      # git's own empty blob

    def fake_origin(self, tamper=None, drop=None):
        files = {}
        tree = []
        for e in self.tree:
            if e["path"] not in PINS["model"]["files"]:
                continue
            data = ("content of %s\n" % e["path"]).encode()
            files[e["path"]] = data
            tree.append({"type": "file", "path": e["path"], "size": len(data), "oid": lib.git_blob_sha1(data)})
        if tamper:
            files[tamper] = b"X" + files[tamper][1:]               # the same size, other bytes: only the digest can notice
        calls = []

        def get(url, timeout=60, limit=1 << 30):
            calls.append(url)
            if "/api/models/" in url:
                self.assertIn(PINS["model"]["revision"], url)
                return json.dumps(tree).encode()
            for path, data in files.items():
                if url.endswith("/resolve/%s/%s" % (PINS["model"]["revision"], path)):
                    return data
            raise OSError("404 " + url)
        return get, calls

    def test_fetch_downloads_at_the_pinned_revision_and_writes_the_manifest(self):
        get, calls = self.fake_origin()
        with tempfile.TemporaryDirectory() as t:
            ok, detail = mr.fetch_model(Path(t) / "model", PINS, get)
            self.assertTrue(ok, detail)
            self.assertEqual(sorted(p.name for p in (Path(t) / "model").iterdir()), sorted(PINS["model"]["files"] + ["model.json"]))
            man = json.loads((Path(t) / "model" / "model.json").read_text())
            self.assertEqual((man["revision"], man["license"]), (PINS["model"]["revision"], "Apache-2.0"))
        self.assertTrue(all(c.startswith("https://huggingface.co/") for c in calls))
        self.assertTrue(all(PINS["model"]["revision"] in c for c in calls))

    def test_a_tampered_file_is_refused_and_not_left_behind(self):
        get, _ = self.fake_origin(tamper="model.safetensors")
        with tempfile.TemporaryDirectory() as t:
            ok, detail = mr.fetch_model(Path(t) / "model", PINS, get)
            self.assertFalse(ok)
            self.assertIn("model.safetensors", detail)
            self.assertFalse((Path(t) / "model" / "model.safetensors").exists())
            self.assertFalse((Path(t) / "model" / "model.json").exists())

    def test_size_is_checked_even_when_the_digest_matches(self):
        data = b"abc"
        e = {"path": "x", "size": len(data) + 1, "kind": "sha256", "digest": hashlib.sha256(data).hexdigest()}
        self.assertIn("bytes", lib.check_model_bytes(e, data))

    def test_an_oversized_listing_fails_the_fetch_before_any_download(self):
        pins = json.loads(json.dumps(PINS))
        pins["model"]["max_bytes"] = 10
        get, calls = self.fake_origin()
        with tempfile.TemporaryDirectory() as t:
            ok, detail = mr.fetch_model(Path(t) / "m", pins, get)
        self.assertFalse(ok)
        self.assertIn("more than the", detail)
        self.assertFalse(any("/resolve/" in c for c in calls))

    def test_a_path_traversal_in_the_pins_is_refused_by_the_fetch_too(self):
        pins = json.loads(json.dumps(PINS))
        pins["model"]["files"] = ["../evil.txt"]
        get, _ = self.fake_origin()
        with tempfile.TemporaryDirectory() as t:
            ok, detail = mr.fetch_model(Path(t) / "m", pins, get)
            self.assertFalse(ok)
            self.assertIn("unsafe", detail)
            self.assertFalse((Path(t) / "evil.txt").exists())

    def test_a_missing_file_or_an_unreachable_origin_is_a_named_failure(self):
        with tempfile.TemporaryDirectory() as t:
            ok, detail = mr.fetch_model(Path(t) / "m", PINS, lambda *a, **k: (_ for _ in ()).throw(OSError("unreachable")))
            self.assertFalse(ok)
            self.assertIn("tree:", detail)

    def test_files_already_verified_are_not_downloaded_again(self):
        get, calls = self.fake_origin()
        with tempfile.TemporaryDirectory() as t:
            self.assertTrue(mr.fetch_model(Path(t) / "model", PINS, get)[0])
            n = len(calls)
            self.assertTrue(mr.fetch_model(Path(t) / "model", PINS, get)[0])
            self.assertEqual(len(calls) - n, 1)               # only the tree listing

    def test_the_http_helper_refuses_plain_http(self):
        with self.assertRaises(ValueError):
            mr.http_get("http://huggingface.co/x")


# ------------------------------------------------------------------------------------------------ the verdicts
def vllm_result(**kw):
    ids = [[100 + i, 200 + i, 300 + i] for i in range(8)]
    r = {"row": "vllm", "token_ids": ids, "token_sha256": lib.canon_sha256(ids), "repeat_identical": True, "unbatched_equal": True, "engine_children": ["VLLM::EngineCore"],
         "cudagraph_n_sizes": 67, "cudagraph_mode": "FULL_AND_PIECEWISE", "mem_growth_mib": 9000, "total_mib": 12288}
    r.update(kw)
    return r


def trt_result(**kw):
    r = {"row": "tensorrt", "onnx_sha256": "a" * 64, "onnx_params": 300000, "ref_sha256": "b" * 64, "ref_max_abs": 3.0,
         "builder": {"built": True, "fp16": True, "timing_cache_bytes": 5000, "engine_bytes": 900000},
         "engine": {"ran": True, "max_abs_err": 0.004, "ref_max_abs": 3.0, "repeat_identical": True},
         "ort_trt": {"status": "PASS", "first_provider": "TensorrtExecutionProvider", "trt_node_events": 2, "max_abs_err": 0.004, "ref_max_abs": 3.0}, "mem_growth_mib": 1500, "total_mib": 12288}
    r.update(kw)
    return r


def jax_result(**kw):
    ops = [{"name": n, "precision": p, "max_abs_err": 1e-3, "ref_max_abs": 100.0, "on_gpu": True} for n, p in (("matmul_f32_highest", "highest"), ("matmul_default", "default"), ("conv", "highest"), ("scan", "highest"))]
    r = {"row": "jax", "default_backend": "gpu", "devices": [{"id": 0, "platform": "gpu"}], "ops": ops, "bytes_limit_mib": 9200.0, "total_mib": 12288, "mem_growth_mib": 9300,
         "autotune_bytes": 4000, "autotune_entries": 6, "autotune_backends": {"CUDNN": 1, "CUBLASLT_FISSION": 2, "NATIVE_EMITTER": 3}, "peak_bytes_in_use_mib": 300.0}
    r.update(kw)
    return r


VLLM_CASES = (
    ("no tokens", {"token_ids": None}, "tokens_for_every_prompt"),
    ("seven prompts", {"token_ids": [[1]] * 7}, "tokens_for_every_prompt"),
    ("an empty answer", {"token_ids": [[1]] * 7 + [[]]}, "tokens_for_every_prompt"),
    ("a negative id", {"token_ids": [[1]] * 7 + [[-1]]}, "tokens_for_every_prompt"),
    ("the hash does not match the ids", {"token_sha256": "0" * 64}, "token_sha256_matches_ids"),
    ("the repeat differs", {"repeat_identical": False}, "greedy_repeat_identical"),
    ("no engine process", {"engine_children": []}, "engine_in_its_own_process"),
    ("seven graph sizes", {"cudagraph_n_sizes": 7}, "cuda_graph_sizes"),
    ("unknown graph sizes", {"cudagraph_n_sizes": None}, "cuda_graph_sizes"),
    ("memory did not grow", {"mem_growth_mib": 299}, "device_memory_grew"),
    ("memory unknown", {"mem_growth_mib": None}, "device_memory_grew"),
)


class JudgeVllm(unittest.TestCase):
    def test_a_good_result_passes(self):
        st, why, checks = lib.judge_vllm(vllm_result())
        self.assertEqual((st, why), (lib.PASS, "ok"))
        self.assertTrue(all(c["ok"] for c in checks))

    def test_each_missing_property_fails_by_its_own_name(self):
        for what, kw, name in VLLM_CASES:
            with self.subTest(what):
                st, why, _ = lib.judge_vllm(vllm_result(**kw))
                self.assertEqual(st, lib.FAIL, what)
                self.assertTrue(why.startswith(name + ":"), "%s: %s" % (what, why))

    def test_the_boundaries(self):
        self.assertEqual(lib.judge_vllm(vllm_result(cudagraph_n_sizes=8))[0], lib.PASS)
        self.assertEqual(lib.judge_vllm(vllm_result(mem_growth_mib=300))[0], lib.PASS)

    def test_no_result_line_is_a_fail_with_that_name(self):
        st, why, _ = lib.judge_vllm(None)
        self.assertEqual(st, lib.FAIL)
        self.assertTrue(why.startswith("no_result"))

    def test_a_committed_expected_hash_is_enforced_when_there_is_one(self):
        r = vllm_result()
        self.assertEqual(lib.judge_vllm(r, r["token_sha256"])[0], lib.PASS)
        st, why, _ = lib.judge_vllm(r, "f" * 64)
        self.assertEqual(st, lib.FAIL)
        self.assertTrue(why.startswith("tokens_equal_committed_hash"))
        self.assertEqual(lib.judge_vllm(r, None)[0], lib.PASS)

    def test_the_hash_is_over_the_ids_in_a_canonical_form(self):
        a, b = [[1, 2], [3]], [[1, 2], [3]]
        self.assertEqual(lib.canon_sha256(a), lib.canon_sha256(b))
        self.assertNotEqual(lib.canon_sha256(a), lib.canon_sha256([[1, 2], [4]]))
        self.assertNotEqual(lib.canon_sha256([[1, 2], [3]]), lib.canon_sha256([[1], [2, 3]]))


def trt_cases():
    b, e, o = trt_result()["builder"], trt_result()["engine"], trt_result()["ort_trt"]
    return (
        ("no model", {"onnx_sha256": None}, "onnx_model_generated"),
        ("not built", {"builder": dict(b, built=False)}, "builder_ran_fp16"),
        ("no fp16", {"builder": dict(b, fp16=False)}, "builder_ran_fp16"),
        ("no tactics timed", {"builder": dict(b, timing_cache_bytes=0)}, "tactics_were_timed"),
        ("engine did not run", {"engine": dict(e, ran=False, error="boom")}, "engine_deserialized_and_ran"),
        ("engine output off", {"engine": dict(e, max_abs_err=0.2)}, "engine_equals_onnxruntime_cpu"),
        ("engine output unknown", {"engine": dict(e, max_abs_err=None)}, "engine_equals_onnxruntime_cpu"),
        ("not repeatable", {"engine": dict(e, repeat_identical=False)}, "engine_repeat_identical"),
        ("ort ran on the cuda ep", {"ort_trt": dict(o, first_provider="CUDAExecutionProvider")}, "ort_trt_ep_ran_on_tensorrt"),
        ("ort ran no trt nodes", {"ort_trt": dict(o, trt_node_events=0)}, "ort_trt_ep_ran_on_tensorrt"),
        ("ort output off", {"ort_trt": dict(o, max_abs_err=1.0)}, "ort_trt_equals_onnxruntime_cpu"),
        ("ort failed", {"ort_trt": {"status": "FAIL", "reason": "x"}}, "ort_trt_ep"),
        ("ort status missing", {"ort_trt": {}}, "ort_trt_status_stated"),
    )


class JudgeTensorrt(unittest.TestCase):
    def test_a_good_result_passes_and_a_not_run_ort_ep_does_not_fail_it(self):
        self.assertEqual(lib.judge_tensorrt(trt_result())[0], lib.PASS)
        self.assertEqual(lib.judge_tensorrt(trt_result(ort_trt={"status": "NOT_RUN", "reason": "cpu wheel"}))[0], lib.PASS)

    def test_each_missing_property_fails_by_its_own_name(self):
        for what, kw, name in trt_cases():
            with self.subTest(what):
                st, why, _ = lib.judge_tensorrt(trt_result(**kw))
                self.assertEqual(st, lib.FAIL, what)
                self.assertTrue(why.startswith(name + ":"), "%s: %s" % (what, why))

    def test_the_tolerance_boundary_is_a_fraction_of_the_reference(self):
        tol = 0.01 * 3.0                                    # the bound is written out: a test that read the constant would follow it wherever it went
        ok = trt_result(engine={"ran": True, "max_abs_err": tol, "ref_max_abs": 3.0, "repeat_identical": True})
        bad = trt_result(engine={"ran": True, "max_abs_err": tol * 1.001, "ref_max_abs": 3.0, "repeat_identical": True})
        self.assertEqual(lib.judge_tensorrt(ok)[0], lib.PASS)
        self.assertEqual(lib.judge_tensorrt(bad)[0], lib.FAIL)

    def test_an_unknown_reference_cannot_pass(self):
        self.assertEqual(lib.judge_tensorrt(trt_result(engine={"ran": True, "max_abs_err": 0.0, "ref_max_abs": None, "repeat_identical": True}))[0], lib.FAIL)

    def test_a_zero_reference_cannot_pass(self):
        self.assertEqual(lib.judge_tensorrt(trt_result(engine={"ran": True, "max_abs_err": 0.0, "ref_max_abs": 0.0, "repeat_identical": True}))[0], lib.FAIL)


def jax_cases():
    ops = jax_result()["ops"]
    return (
        ("cpu backend", {"default_backend": "cpu", "devices": [{"platform": "cpu"}]}, "jax_devices_are_gpu"),
        ("no devices", {"devices": []}, "jax_devices_are_gpu"),
        ("a cpu device among gpus", {"devices": [{"platform": "gpu"}, {"platform": "cpu"}]}, "jax_devices_are_gpu"),
        ("matmul off", {"ops": [dict(ops[0], max_abs_err=1.0)] + ops[1:]}, "matmul_f32_highest_equals_cpu"),
        ("result not on the gpu", {"ops": ops[:3] + [dict(ops[3], on_gpu=False)]}, "scan_equals_cpu"),
        ("conv missing", {"ops": [o for o in ops if o["name"] != "conv"]}, "matmul_conv_scan_all_ran"),
        ("preallocation 50 percent", {"bytes_limit_mib": 6144.0}, "preallocation_is_75_percent"),
        ("preallocation 100 percent", {"bytes_limit_mib": 12288.0}, "preallocation_is_75_percent"),
        ("limit unknown", {"bytes_limit_mib": None}, "preallocation_is_75_percent"),
        ("device memory did not grow", {"mem_growth_mib": 500}, "preallocation_observed_on_the_device"),
        ("no autotune results", {"autotune_bytes": 0}, "autotuner_ran"),
        ("only the emitter's own kernels in the dump", {"autotune_backends": {"NATIVE_EMITTER": 5}}, "autotuner_ran"),
        ("an empty dump table", {"autotune_backends": {}}, "autotuner_ran"),
        ("nothing allocated", {"peak_bytes_in_use_mib": 1.0}, "work_allocated_on_the_device"),
    )


class JudgeJax(unittest.TestCase):
    def test_a_good_result_passes(self):
        self.assertEqual(lib.judge_jax(jax_result())[0], lib.PASS)

    def test_each_missing_property_fails_by_its_own_name(self):
        for what, kw, name in jax_cases():
            with self.subTest(what):
                st, why, _ = lib.judge_jax(jax_result(**kw))
                self.assertEqual(st, lib.FAIL, what)
                self.assertTrue(why.startswith(name + ":"), "%s: %s" % (what, why))

    def test_the_strict_and_the_default_precision_have_their_own_tolerances(self):
        ops = jax_result()["ops"]
        e = 2e-3 * 100.0                                   # 2e-3 of the scale: fine for the default precision (TF32 gives 3e-4), far too much for HIGHEST
        self.assertEqual(lib.judge_jax(jax_result(ops=[dict(ops[0]), dict(ops[1], max_abs_err=e), ops[2], ops[3]]))[0], lib.PASS)
        self.assertEqual(lib.judge_jax(jax_result(ops=[dict(ops[0], max_abs_err=e)] + ops[1:]))[0], lib.FAIL)

    def test_the_preallocation_window_boundaries(self):
        for ratio, ok in ((0.68, True), (0.78, True), (0.679, False), (0.781, False)):
            r = jax_result(bytes_limit_mib=12288 * ratio)
            self.assertEqual(lib.judge_jax(r)[0] == lib.PASS, ok, ratio)

    def test_no_result_is_a_fail(self):
        self.assertEqual(lib.judge_jax(None)[0], lib.FAIL)


def _per_case(cls, judge, base, cases):
    for what, kw, name in cases:
        def t(self, kw=kw, name=name, what=what):
            st, why, _ = judge(base(**kw))
            self.assertEqual(st, lib.FAIL, what)
            self.assertTrue(why.startswith(name + ":"), "%s: %s" % (what, why))
        setattr(cls, "test_case_" + re.sub(r"\W+", "_", what).strip("_"), t)


_per_case(JudgeVllm, lib.judge_vllm, vllm_result, VLLM_CASES)
_per_case(JudgeTensorrt, lib.judge_tensorrt, trt_result, trt_cases())
_per_case(JudgeJax, lib.judge_jax, jax_result, jax_cases())


class Proof(unittest.TestCase):
    """The proof line is read by the SAME function the other rows use (gpu_use.proof_framework): a CPU run, a non-NVIDIA name and too little memory fail."""
    f = facts()

    def test_the_line_has_the_shape_the_judge_reads(self):
        for row, res in (("vllm", vllm_result()), ("tensorrt", trt_result()), ("jax", jax_result())):
            line = lib.proof_for(row, res, self.f)
            self.assertRegex(line, r"^GPU_USE_PROOF %s device=cuda:0 name=NVIDIA GeForce RTX 3060 peak_mib=\d+$" % row)
            p = gu.PROOFS["framework"](line, framework=row, min_mib=1)
            self.assertIs(p["ok"], True, p)

    def test_a_cpu_run_prints_a_cpu_device_and_the_judge_fails_it(self):
        line = lib.proof_for("jax", jax_result(platform="cpu"), self.f)
        self.assertIn("device=cpu:0 name=cpu", line)
        self.assertIs(gu.PROOFS["framework"](line, framework="jax", min_mib=1)["ok"], False)

    def test_a_cpu_platform_prints_a_cpu_device_whatever_the_memory_did(self):
        for row in lib.ROWS:
            line = lib.proof_for(row, dict(vllm_result(), platform="cpu", mem_growth_mib=9000), self.f)
            self.assertIn("device=cpu:0 name=cpu", line, row)
        line = lib.proof_for("jax", dict(jax_result(), device="cpu"), self.f)
        self.assertIn("device=cpu:0", line)

    def test_no_measurement_or_no_gpu_name_prints_no_proof(self):
        self.assertIsNone(lib.proof_for("vllm", vllm_result(mem_growth_mib=None), self.f))
        self.assertIsNone(lib.proof_for("vllm", None, self.f))
        self.assertIsNone(lib.proof_for("vllm", vllm_result(), facts(name=None)))

    def test_the_peak_is_the_memory_growth_and_never_negative(self):
        self.assertIn("peak_mib=0", lib.proof_for("vllm", vllm_result(mem_growth_mib=-5), self.f))
        self.assertIn("peak_mib=9000", lib.proof_for("vllm", vllm_result(), self.f))

    def test_the_floors_in_the_class_table_are_enforced_by_the_judge(self):
        for row in lib.ROWS:
            p = gur.policy(row)
            line = lib.proof_for(row, vllm_result(mem_growth_mib=p["params"]["min_mib"] - 1), self.f)
            self.assertIs(gu.PROOFS[p["proof"]](line, **p["params"])["ok"], False, row)
            line = lib.proof_for(row, vllm_result(mem_growth_mib=p["params"]["min_mib"]), self.f)
            self.assertIs(gu.PROOFS[p["proof"]](line, **p["params"])["ok"], True, row)


# ------------------------------------------------------------------------------------------------ the environment of a row
class Env(unittest.TestCase):
    secrets = {"HF_TOKEN": "x", "GITHUB_TOKEN": "x", "AWS_SECRET_ACCESS_KEY": "x", "VAST_API_KEY": "x", "CONTAINER_API_KEY": "x", "OPENAI_API_KEY": "x", "SSH_AUTH_SOCK": "/x",
               "PYTHONPATH": "/evil", "LD_PRELOAD": "/evil.so", "PIP_INDEX_URL": "http://evil/", "HF_ENDPOINT": "http://evil/", "XLA_FLAGS": "--evil", "VLLM_USE_PRECOMPILED": "0",
               "PATH": "/usr/bin", "LANG": "C.UTF-8"}

    def env(self, row="vllm", **kw):
        t = tempfile.mkdtemp()
        venv = Path(t) / "venv"
        (venv / "lib/python3.12/site-packages/tensorrt_libs").mkdir(parents=True)
        (venv / "lib/python3.12/site-packages/nvidia/cu13/lib").mkdir(parents=True)
        (venv / "lib/python3.12/site-packages/nvidia/cuda_runtime/lib").mkdir(parents=True)
        return mr.row_env(row, venv, Path(t) / "cache", dict(self.secrets, **kw.pop("base", {})), **kw), t

    def test_no_secret_and_no_loader_hook_reaches_the_row(self):
        env, _ = self.env()
        for bad in self.secrets:
            if bad not in ("PATH", "LANG"):
                self.assertNotIn(bad, env, bad)
        self.assertEqual(env["PATH"], "/usr/bin")
        self.assertEqual(env["LANG"], "C.UTF-8")

    def test_the_environment_is_the_allowlist_plus_ours(self):
        env, _ = self.env()
        ours = {"HOME", "XDG_CACHE_HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "TMPDIR", "TRITON_CACHE_DIR", "TORCHINDUCTOR_CACHE_DIR", "TORCH_EXTENSIONS_DIR", "CUDA_CACHE_PATH", "VLLM_CACHE_ROOT",
                "MPLCONFIGDIR", "PYTHONNOUSERSITE", "PYTHONDONTWRITEBYTECODE", "PYTHONUNBUFFERED", "HF_HUB_OFFLINE", "TRANSFORMERS_OFFLINE", "HF_HOME", "VLLM_NO_USAGE_STATS", "DO_NOT_TRACK",
                "CUDA_VISIBLE_DEVICES", "LD_LIBRARY_PATH"}
        self.assertLessEqual(set(env), set(mr.ENV_KEEP) | ours)

    def test_every_cache_is_under_the_run_s_cache_directory(self):
        env, t = self.env()
        for k in ("HOME", "XDG_CACHE_HOME", "TMPDIR", "TRITON_CACHE_DIR", "TORCHINDUCTOR_CACHE_DIR", "TORCH_EXTENSIONS_DIR", "CUDA_CACHE_PATH", "VLLM_CACHE_ROOT", "HF_HOME", "MPLCONFIGDIR"):
            self.assertTrue(env[k].startswith(os.path.join(t, "cache")), k)
            self.assertTrue(os.path.isdir(env[k]), k)

    def test_telemetry_is_off_and_the_hub_is_offline(self):
        env, _ = self.env()
        self.assertEqual((env["VLLM_NO_USAGE_STATS"], env["DO_NOT_TRACK"], env["HF_HUB_OFFLINE"], env["TRANSFORMERS_OFFLINE"]), ("1", "1", "1", "1"))

    def test_the_venv_s_libraries_come_first_then_the_callers(self):
        env, t = self.env(base={"LD_LIBRARY_PATH": "/usr/local/nvidia/lib64"})
        parts = env["LD_LIBRARY_PATH"].split(":")
        self.assertTrue(parts[0].endswith("tensorrt_libs"))
        self.assertTrue(all("nvidia" in p for p in parts[1:3]))
        self.assertEqual(parts[-1], "/usr/local/nvidia/lib64")

    def test_gpu_zero_by_default_and_the_callers_choice_when_given(self):
        self.assertEqual(self.env()[0]["CUDA_VISIBLE_DEVICES"], "0")
        self.assertEqual(self.env(base={"CUDA_VISIBLE_DEVICES": "2"})[0]["CUDA_VISIBLE_DEVICES"], "2")

    def test_the_negative_control_hides_the_gpu_and_jax_gets_its_cpu_backend(self):
        for row in lib.ROWS:
            env, _ = self.env(row, negctl=True)
            self.assertEqual(env["CUDA_VISIBLE_DEVICES"], "", row)
            self.assertEqual(env["KF_NEGCTL"], "1")
            self.assertEqual(env.get("JAX_PLATFORMS"), "cpu" if row == "jax" else None, row)
        env, _ = self.env("jax")
        self.assertNotIn("JAX_PLATFORMS", env)
        self.assertNotIn("XLA_PYTHON_CLIENT_PREALLOCATE", env)           # the defaults are the experiment

    def test_a_missing_library_directory_gives_no_ld_library_path(self):
        t = tempfile.mkdtemp()
        env = mr.row_env("jax", Path(t) / "venv", Path(t) / "cache", {"PATH": "/usr/bin"})
        self.assertNotIn("LD_LIBRARY_PATH", env)


# ------------------------------------------------------------------------------------------------ running a row (fake processes)
class FakeProc:
    def __init__(self, lines, rc=0, hold=None):
        self.lines, self.rc, self.hold, self.pid = lines, rc, hold, 2 ** 22 + 7
        self.release = __import__("threading").Event()

    @property
    def stdout(self):
        def gen():
            for l in self.lines:
                yield l if l.endswith("\n") else l + "\n"
            if self.hold:
                self.release.wait(self.hold)
        return gen()

    def wait(self):
        return -9 if self.hold and self.release.is_set() else self.rc


class FakeSampler:
    used, total = [500, 300, 400, 6000, 9300, 9300, 400], 12288

    def start(self):
        pass

    def stop(self):
        pass


class RunRow(unittest.TestCase):
    def setUp(self):
        self.t = Path(tempfile.mkdtemp())
        self.root, self.out = self.t / "root", self.t / "out"
        for row in lib.ROWS:
            (self.root / row / "venv/bin").mkdir(parents=True)
            (self.root / row / "venv/bin/python").write_text("")
            (self.root / row / "setup.json").write_text(json.dumps({"status": "ok", "flavour": "cu13"}))
        self.captured = {}
        self.patches = [mock.patch.object(mr, "Sampler", FakeSampler), mock.patch.object(mr.time, "sleep", lambda s: None)]
        for p in self.patches:
            p.start()

    def tearDown(self):
        for p in self.patches:
            p.stop()

    def popen(self, lines, rc=0, hold=None):
        def make(argv, **kw):
            self.captured.update(argv=argv, env=kw["env"], cwd=kw["cwd"])
            self.proc = FakeProc(lines, rc, hold)
            return self.proc
        return make

    def run_row(self, row, lines, **kw):
        return mr.run_row(row, self.root, self.out, kw.pop("facts", facts()), kw.pop("pins", NOHASH), popen=self.popen(lines, kw.pop("rc", 0), kw.pop("hold", None)), **kw)

    def good_lines(self, row="vllm"):
        res = {"vllm": vllm_result(), "tensorrt": trt_result(), "jax": jax_result()}[row]
        for k in ("mem_growth_mib", "total_mib"):
            res.pop(k, None)
        return ["noise line", lib.JSON_PREFIX + json.dumps(res)]

    def test_a_good_run_is_judged_and_gets_the_runner_s_memory_record(self):
        st, why, flavour, checks, result, tail = self.run_row("vllm", self.good_lines())
        self.assertEqual((st, why, flavour), (lib.PASS, "ok", "cu13"), why)
        self.assertEqual((result["mem_base_mib"], result["mem_peak_mib"], result["mem_growth_mib"], result["total_mib"]), (300, 9300, 9000, 12288))
        self.assertEqual(tail[0], "noise line")

    def test_the_row_runs_with_the_venv_python_the_row_script_and_its_own_working_directory(self):
        self.run_row("vllm", self.good_lines())
        argv = self.captured["argv"]
        self.assertEqual(argv[0], str(self.root / "vllm/venv/bin/python"))
        self.assertEqual(argv[1:3], ["-B", str(ML / "row_vllm.py")])
        self.assertEqual(argv[3:], ["--model-dir", str(self.root / "vllm/model")])
        self.assertEqual(self.captured["cwd"], str(self.out / "work"))

    def test_the_cache_directory_is_fresh_on_every_run(self):
        (self.out / "cache" / "triton").mkdir(parents=True)
        (self.out / "cache" / "triton" / "stale").write_text("x")
        self.run_row("jax", self.good_lines("jax"))
        self.assertFalse((self.out / "cache" / "triton" / "stale").exists())

    def test_the_working_directory_is_empty_at_the_start_of_every_run(self):
        (self.out / "work").mkdir(parents=True)
        (self.out / "work" / "xla_autotune_results.textproto").write_text("entries {}")       # what the previous run left behind
        seen = {}

        def make(argv, **kw):
            seen["files"] = sorted(os.listdir(kw["cwd"]))
            return FakeProc(self.good_lines("jax"))
        mr.run_row("jax", self.root, self.out, facts(), PINS, popen=make)
        self.assertEqual(seen["files"], [])

    def test_a_second_run_in_the_same_directory_starts_with_empty_caches(self):
        seen = []

        def make(argv, **kw):
            seen.append(sorted(p.name for d in Path(kw["env"]["HOME"]).parent.iterdir() for p in d.iterdir()))
            (Path(kw["env"]["TRITON_CACHE_DIR"]) / "kernel.bin").write_text("x")
            return FakeProc(self.good_lines("jax"))
        for _ in range(2):
            mr.run_row("jax", self.root, self.out, facts(), PINS, popen=make)
        self.assertEqual(seen[1], [])

    def test_the_working_directory_of_every_row_starts_empty(self):
        for row in lib.ROWS:
            (self.out / "work").mkdir(parents=True, exist_ok=True)
            (self.out / "work" / "left_over").write_text("x")
            seen = []

            def make(argv, **kw):
                seen.append(os.listdir(kw["cwd"]))
                return FakeProc(self.good_lines(row))
            mr.run_row(row, self.root, self.out, facts(), PINS, popen=make)
            self.assertEqual(seen, [[]], row)

    def test_the_row_runs_with_the_hub_offline_and_no_secret(self):
        with mock.patch.dict(os.environ, {"HF_TOKEN": "x", "GITHUB_TOKEN": "y"}):
            mr.run_row("vllm", self.root, self.out, facts(), PINS, popen=self.popen(self.good_lines()))
        env = self.captured["env"]
        self.assertEqual((env["HF_HUB_OFFLINE"], env["TRANSFORMERS_OFFLINE"]), ("1", "1"))
        self.assertNotIn("HF_TOKEN", env)
        self.assertNotIn("GITHUB_TOKEN", env)

    def test_the_timeout_fail_names_the_limit(self):
        pins = json.loads(json.dumps(PINS))
        pins["rows"]["jax"]["timeout_s"] = 0.2
        with mock.patch.object(mr, "_kill", lambda proc: proc.release.set()):
            st, why, *_ = self.run_row("jax", ["working"], pins=pins, hold=5)
        self.assertIn("did not finish in 0.2 s", why)

    def test_the_row_s_library_path_leads_with_the_venv(self):
        libdir = self.root / "tensorrt" / "venv" / "lib" / "python3.12" / "site-packages" / "tensorrt_libs"
        libdir.mkdir(parents=True)
        with mock.patch.dict(os.environ, {"LD_LIBRARY_PATH": "/usr/local/nvidia/lib64"}):
            mr.run_row("tensorrt", self.root, self.out, facts(), PINS, popen=self.popen(self.good_lines("tensorrt")))
        parts = self.captured["env"]["LD_LIBRARY_PATH"].split(":")
        self.assertEqual(parts[0], str(libdir))
        self.assertEqual(parts[-1], "/usr/local/nvidia/lib64")

    def test_log_facts_are_read_from_the_whole_log_not_the_tail(self):
        lines = ["INFO GPU KV cache size: 445,056 tokens"] + ["noise"] * (mr.MAX_LOG_LINES + 20) + self.good_lines()
        _, _, _, _, result, _ = self.run_row("vllm", lines)
        self.assertEqual(result["log_facts"]["kv_cache_tokens"], 445056)

    def test_a_run_whose_reference_is_all_zero_is_a_fail_with_the_check_named(self):
        res = trt_result(engine={"ran": True, "max_abs_err": 0.0, "ref_max_abs": 0.0, "repeat_identical": True})
        for k in ("mem_growth_mib", "total_mib"):
            res.pop(k, None)
        st, why, *_ = self.run_row("tensorrt", [lib.JSON_PREFIX + json.dumps(res)])
        self.assertEqual(st, lib.FAIL)
        self.assertTrue(why.startswith("engine_equals_onnxruntime_cpu"), why)

    def test_a_committed_expected_token_hash_reaches_the_judge(self):
        pins = json.loads(json.dumps(PINS))
        pins["rows"]["vllm"]["expected_token_sha256"] = "f" * 64
        st, why, *_ = self.run_row("vllm", self.good_lines(), pins=pins)
        self.assertEqual(st, lib.FAIL)
        self.assertTrue(why.startswith("tokens_equal_committed_hash"), why)

    def test_a_machine_without_a_c_compiler_is_not_run_for_vllm_before_the_row_starts(self):
        self.captured.clear()
        for f in (facts(gcc=False), facts(python_h=False)):
            st, why, *_ = self.run_row("vllm", self.good_lines(), facts=f)
            self.assertEqual(st, lib.NOT_RUN)
            self.assertIn("no_c_compiler_or_python_headers", why)
        self.assertNotIn("argv", self.captured)

    def test_the_other_rows_do_not_need_a_c_compiler(self):
        for row in ("jax", "tensorrt"):
            st, why, *_ = self.run_row(row, self.good_lines(row), facts=facts(gcc=False, python_h=False))
            self.assertEqual(st, lib.PASS, (row, why))

    def test_a_forced_flavour_the_driver_cannot_take_is_not_run(self):
        st, why, *_ = self.run_row("jax", [], facts=facts(driver_cuda=(12, 9)), forced="cu13")
        self.assertEqual(st, lib.NOT_RUN)
        self.assertIn("driver_cuda_below_forced_flavour", why)

    def test_not_set_up_is_not_run_with_the_command_to_run(self):
        (self.root / "jax" / "setup.json").unlink()
        st, why, *_ = self.run_row("jax", [])
        self.assertEqual(st, lib.NOT_RUN)
        self.assertIn("not_set_up", why)
        self.assertIn("mlrows.py setup", why)

    def test_a_venv_for_another_flavour_is_not_run_unless_forced(self):
        (self.root / "jax" / "setup.json").write_text(json.dumps({"status": "ok", "flavour": "cu12"}))
        st, why, *_ = self.run_row("jax", [])
        self.assertEqual(st, lib.NOT_RUN)
        self.assertIn("set_up_for_another_flavour", why)

    def test_hardware_that_cannot_take_the_row_is_not_run_before_anything_starts(self):
        self.captured.clear()
        st, why, *_ = self.run_row("vllm", [], facts=facts(driver_cuda=(12, 8)))
        self.assertEqual(st, lib.NOT_RUN)
        self.assertNotIn("argv", self.captured)

    def test_a_row_that_prints_nothing_is_a_fail_naming_its_last_line(self):
        st, why, _, _, result, _ = self.run_row("jax", ["starting", "Traceback (most recent call last):", "RuntimeError: out of memory"], rc=1)
        self.assertEqual(st, lib.FAIL)
        self.assertIsNone(result)
        self.assertTrue(why.startswith("no_result: rc=1"))
        self.assertIn("RuntimeError: out of memory", why)

    def test_a_row_that_hangs_is_killed_and_is_a_fail_with_the_timeout(self):
        pins = json.loads(json.dumps(PINS))
        pins["rows"]["jax"]["timeout_s"] = 0.2
        with mock.patch.object(mr, "_kill", lambda proc: proc.release.set()):
            st, why, *_ = self.run_row("jax", ["working"], pins=pins, hold=5)
        self.assertEqual(st, lib.FAIL)
        self.assertTrue(why.startswith("timeout:"))

    def test_the_whole_log_is_kept_and_the_judge_reads_the_whole_log_not_the_tail(self):
        noise = ["x"] * (mr.MAX_LOG_LINES + 50)
        lines = [lib.JSON_PREFIX + json.dumps(vllm_result())] + noise
        st, _, _, _, result, tail = self.run_row("vllm", lines)
        self.assertEqual(len(tail), mr.MAX_LOG_LINES)
        self.assertIsNotNone(result)                                       # the result line scrolled out of the tail but is in the log file
        self.assertIn(lib.JSON_PREFIX, (self.out / "vllm.venv.log").read_text())

    def test_the_negative_control_runs_the_cpu_form(self):
        self.run_row("jax", self.good_lines("jax"), negctl=True)
        self.assertEqual(self.captured["env"]["JAX_PLATFORMS"], "cpu")
        self.assertEqual(self.captured["env"]["CUDA_VISIBLE_DEVICES"], "")
        self.run_row("tensorrt", self.good_lines("tensorrt"), negctl=True)
        self.assertIn("--negctl", self.captured["argv"])

    def test_vllm_log_facts_are_recorded_from_the_engine_log(self):
        lines = self.good_lines() + ["INFO GPU KV cache size: 123,456 tokens", "INFO Graph capturing finished in 12 secs, took 0.45 GiB", "INFO Model loading took 0.25 GiB and 1.5 seconds"]
        _, _, _, _, result, _ = self.run_row("vllm", lines)
        self.assertEqual(result["log_facts"], {"kv_cache_tokens": 123456, "graph_capture_s": 12.0, "graph_capture_gib": 0.45, "model_load_gib": 0.25})

    def test_a_missing_nvidia_smi_leaves_the_memory_unknown_and_the_row_fails_its_memory_check(self):
        class Silent(FakeSampler):
            used, total = [], None
        with mock.patch.object(mr, "Sampler", Silent):
            st, why, *_ = self.run_row("vllm", self.good_lines())
        self.assertEqual(st, lib.FAIL)
        self.assertTrue(why.startswith("device_memory_grew"))


class MemoryRecord(unittest.TestCase):
    def test_growth_is_the_peak_over_the_baseline_of_the_first_samples(self):
        self.assertEqual(mr.memory_record([500, 300, 400, 9000, 200], 12288), {"mem_base_mib": 300, "mem_peak_mib": 9000, "mem_growth_mib": 8700, "total_mib": 12288})

    def test_no_samples_is_unknown_not_zero(self):
        self.assertIsNone(mr.memory_record([], 12288)["mem_growth_mib"])

    def test_the_sampler_line_parser(self):
        self.assertEqual(mr.parse_mem_line("1234, 12288\n"), (1234, 12288))
        for bad in ("[N/A], 12288", "", "x", "1, 2, 3"):
            self.assertEqual(mr.parse_mem_line(bad), (None, None), bad)


class CmdRun(unittest.TestCase):
    """What `mlrows.py run` prints, which is what run_apps.sh, gpu_use and `compare` read."""

    def run_main(self, row, ret, negctl=False):
        buf = io.StringIO()
        env = {"KF_NEGCTL": "1"} if negctl else {}
        with mock.patch.object(mr, "collect_facts", lambda *a, **k: facts()), mock.patch.object(mr, "run_row", lambda *a, **k: ret), mock.patch.dict(os.environ, env, clear=False), \
                mock.patch("sys.stdout", buf):
            if not negctl:
                os.environ.pop("KF_NEGCTL", None)
            rc = mr.main(["run", row, "--root", "/nonexistent", "--out", "/nonexistent/out"])
        return rc, buf.getvalue().splitlines()

    def passed(self, row):
        res = {"vllm": vllm_result(), "tensorrt": trt_result(), "jax": jax_result()}[row]
        st, why, checks = lib.JUDGES[row](res)
        return st, why, "cu13", checks, dict(res, secs=12.0), ["a log line"]

    def test_a_pass_prints_the_summary_the_proof_and_the_judged_line_and_exits_zero(self):
        for row, name in (("vllm", "VLLM_SUMMARY"), ("tensorrt", "TENSORRT_SUMMARY"), ("jax", "JAX_SUMMARY")):
            rc, lines = self.run_main(row, self.passed(row))
            self.assertEqual(rc, 0, row)
            self.assertTrue(any(l.startswith(name + " status=PASS ") for l in lines), row)
            self.assertTrue(any(l.startswith("GPU_USE_PROOF %s device=cuda:0 " % row) for l in lines), row)
            self.assertTrue(any(l.startswith(lib.RESULT_PREFIX) for l in lines), row)
            self.assertFalse(any(l.startswith("APPNOTRUN") for l in lines))
            self.assertTrue(any(l.startswith("MLROWS_CHECK name=result_present ok=1") for l in lines))

    def test_a_fail_exits_one_and_names_the_reason_in_the_summary(self):
        res = vllm_result(repeat_identical=False)
        st, why, checks = lib.judge_vllm(res)
        rc, lines = self.run_main("vllm", (st, why, "cu13", checks, res, []))
        self.assertEqual(rc, 1)
        self.assertTrue(any(l.startswith("VLLM_SUMMARY status=FAIL ") and "greedy_repeat_identical" in l for l in lines))

    def test_not_run_exits_zero_prints_appnotrun_and_never_a_pass_marker(self):
        rc, lines = self.run_main("vllm", (lib.NOT_RUN, "driver_cuda_below_every_wheel: cu13: x", None, [], None, []))
        self.assertEqual(rc, 0)
        self.assertTrue(any(l.startswith("APPNOTRUN driver_cuda_below_every_wheel") for l in lines))
        self.assertFalse(any("status=PASS" in l for l in lines))
        self.assertFalse(any(l.startswith("GPU_USE_PROOF") for l in lines))

    def test_a_run_without_a_memory_measurement_prints_no_proof_and_does_not_crash(self):
        res = vllm_result(mem_growth_mib=None)
        st, why, checks = lib.judge_vllm(res)
        rc, lines = self.run_main("vllm", (st, why, "cu13", checks, res, []))
        self.assertEqual(rc, 1)
        self.assertFalse(any(l.startswith("GPU_USE_PROOF") for l in lines))
        self.assertTrue(any(l.startswith("VLLM_SUMMARY status=FAIL ") for l in lines))

    def test_one_clean_note_line_for_the_matrix_whatever_the_reason_says(self):
        res = trt_result(engine={"ran": True, "max_abs_err": 0.2, "ref_max_abs": 3.0, "repeat_identical": True})
        st, why, checks = lib.judge_tensorrt(res)
        rc, lines = self.run_main("tensorrt", (st, why, "cu13", checks, res, []))
        notes = [l for l in lines if l.startswith("APPNOTE ")]
        self.assertEqual(len(notes), 1)
        self.assertTrue(notes[0].startswith("APPNOTE FAIL on cu13: engine_equals_onnxruntime_cpu"), notes[0])
        self.assertNotRegex(notes[0], r"[=|]")
        self.assertLessEqual(len(notes[0]), 160)
        rc, lines = self.run_main("vllm", self.passed("vllm"))
        self.assertIn("APPNOTE PASS on cu13: ok", lines)

    def test_the_exit_code_is_one_only_for_a_fail(self):
        res = vllm_result()
        for status, want in ((lib.PASS, 0), (lib.NOT_RUN, 0), (lib.FAIL, 1)):
            rc, _ = self.run_main("vllm", (status, "x", "cu13", [], res, []))
            self.assertEqual(rc, want, status)

    def test_the_judged_line_round_trips_through_extract(self):
        rc, lines = self.run_main("jax", self.passed("jax"))
        got = lib.extract_judged("\n".join(lines), "jax")
        self.assertEqual((got["row"], got["status"], got["flavour"]), ("jax", lib.PASS, "cu13"))
        self.assertIsNone(lib.extract_judged("\n".join(lines), "vllm"))

    def test_a_negative_control_run_is_marked_in_its_row_line(self):
        rc, lines = self.run_main("jax", self.passed("jax"), negctl=True)
        self.assertTrue(any(l.startswith("MLROWS_ROW ") and "negctl=1" in l for l in lines))


# ------------------------------------------------------------------------------------------------ setup (fake pip, fake origin)
class Setup(unittest.TestCase):
    def setUp(self):
        self.t = Path(tempfile.mkdtemp())
        self.addCleanup(__import__("shutil").rmtree, self.t, True)          # the vllm test writes a 266 MB fake model here: without this every run left 266 MB in /tmp (it filled a 6 GB tmpfs)
        self.calls = []
        self.fail_pip = False

    def run_cmd(self, argv, env=None, timeout=60, cwd=None, stdin=None):
        self.calls.append(list(argv))
        if argv[1:3] == ["-m", "venv"]:
            (Path(argv[3]) / "bin").mkdir(parents=True)
            (Path(argv[3]) / "bin" / "python").write_text("")
            return 0, "", False
        if "pip" in argv:
            if self.fail_pip:
                return 1, "ERROR: THESE PACKAGES DO NOT MATCH THE HASHES FROM THE REQUIREMENTS FILE.\n    jaxlib==0.12.0.1 from x\n", False
            if getattr(self, "pip_output", None):
                return 1, self.pip_output, False
            return 0, "Successfully installed jax\n", False
        return 0, "", False

    def setup(self, row="jax", f=None, get=None, **kw):
        with mock.patch.object(mr, "run_cmd", self.run_cmd):
            return mr.setup_row(row, self.t, f or facts(), PINS, get=get, **kw)

    def test_the_install_is_one_hashed_only_binary_no_deps_pip_run_of_the_lock(self):
        st, why = self.setup("jax")
        self.assertEqual(st, "ok", why)
        pip = next(c for c in self.calls if "pip" in c)
        for flag in ("--require-hashes", "--only-binary=:all:", "--no-deps", "--no-cache-dir", "-r"):
            self.assertIn(flag, pip)
        self.assertEqual(pip[pip.index("-r") + 1], str(ML / "locks/jax_cu13.lock.txt"))
        for bad in ("--index-url", "--extra-index-url", "--trusted-host", "--no-verify-hashes", "-U", "--upgrade", "--pre"):
            self.assertNotIn(bad, pip)
        self.assertEqual(len([c for c in self.calls if "pip" in c]), 1)
        state = json.loads((self.t / "jax" / "setup.json").read_text())
        self.assertEqual((state["status"], state["flavour"], state["lock_sha256"]), ("ok", "cu13", lib.lock_sha256((ML / "locks/jax_cu13.lock.txt").read_text())))

    def test_a_second_setup_with_the_same_lock_does_nothing(self):
        self.setup("jax")
        n = len(self.calls)
        st, why = self.setup("jax")
        self.assertEqual(st, "ok")
        self.assertIn("already set up", why)
        self.assertEqual(len(self.calls), n)

    def test_another_flavour_or_a_changed_lock_starts_clean(self):
        self.setup("jax")
        n = len(self.calls)
        st, _ = self.setup("jax", facts(driver_cuda=(12, 9)))
        self.assertEqual(st, "ok")
        self.assertGreater(len(self.calls), n)
        self.assertEqual(json.loads((self.t / "jax" / "setup.json").read_text())["flavour"], "cu12")

    def test_a_hash_mismatch_is_a_named_fail_and_leaves_no_ok_state(self):
        self.fail_pip = True
        st, why = self.setup("jax")
        self.assertEqual(st, lib.FAIL)
        self.assertTrue(why.startswith("hash_mismatch"), why)
        self.assertFalse((self.t / "jax" / "setup.json").exists())

    def not_run(self, f, frag, row="vllm"):
        self.calls.clear()
        st, why = self.setup(row, f)
        self.assertEqual(st, lib.NOT_RUN, frag)
        self.assertIn(frag, why)
        self.assertEqual(self.calls, [], frag)

    def test_hardware_and_environment_preconditions_are_named_not_runs_and_nothing_runs(self):
        for f, frag in ((facts(driver_cuda=(11, 8)), "driver_cuda_below"), (facts(cc=(6, 1)), "gpu_older_than_turing"), (facts(gcc=False), "no_c_compiler"), (facts(python_h=False), "no_c_compiler"),
                        (facts(disk_free_gb=3.0), "disk_free_below_need"), (facts(total_mib=2048), "vram_below_floor"), (facts(python=(3, 11, 9)), "python_too_old")):
            self.not_run(f, frag)

    def test_no_c_compiler_is_not_run_for_vllm(self):
        self.not_run(facts(gcc=False), "no_c_compiler")

    def test_no_python_headers_is_not_run_for_vllm(self):
        self.not_run(facts(python_h=False), "no_c_compiler")

    def test_too_little_disk_is_not_run_with_the_numbers(self):
        self.not_run(facts(disk_free_gb=3.0), "3 GB free, 30 GB needed")

    def test_too_little_video_memory_is_not_run(self):
        self.not_run(facts(total_mib=2048), "vram_below_floor")

    def test_an_old_python_is_not_run(self):
        self.not_run(facts(python=(3, 11, 9)), "python_too_old", "jax")

    def test_an_unknown_driver_cuda_is_a_fail_with_the_reason_not_a_not_run(self):
        st, why = self.setup("jax", facts(driver_cuda=None))
        self.assertEqual(st, lib.FAIL)
        self.assertIn("driver_cuda_unknown", why)
        self.assertEqual(self.calls, [])

    def test_only_vllm_needs_a_c_compiler(self):
        self.assertEqual(self.setup("jax", facts(gcc=False, python_h=False))[0], "ok")

    def test_a_lock_that_lost_a_hash_is_refused_before_pip_runs(self):
        bad = (ML / "locks/jax_cu13.lock.txt").read_text()
        bad = re.sub(r"(jaxlib==[^\n]*\\\n)(\s+--hash=sha256:[0-9a-f]{64}( \\)?\n)+", r"\1", bad, count=1)
        with mock.patch.object(mr, "lock_path", lambda *a: self.t / "bad.lock"):
            (self.t / "bad.lock").write_text(bad)
            st, why = self.setup("jax")
        self.assertEqual(st, lib.FAIL)
        self.assertTrue(why.startswith("lock_refused"))
        self.assertEqual(self.calls, [])

    def refused(self, edit, frag):
        text = edit((ML / "locks/jax_cu13.lock.txt").read_text())
        (self.t / "bad.lock").write_text(text)
        with mock.patch.object(mr, "lock_path", lambda *a: self.t / "bad.lock"):
            st, why = self.setup("jax")
        self.assertEqual(st, lib.FAIL)
        self.assertTrue(why.startswith("lock_refused"), why)
        self.assertIn(frag, why)
        self.assertEqual(self.calls, [])

    def test_a_lock_from_a_foreign_index_is_refused_before_pip_runs(self):
        self.refused(lambda t: t.replace("--index-url https://pypi.org/simple", "--index-url https://evil.example/simple"), "index host")

    def test_a_lock_that_names_a_plain_http_index_is_refused_before_pip_runs(self):
        self.refused(lambda t: t.replace("--index-url https://pypi.org/simple", "--index-url http://pypi.org/simple"), "not https")

    def test_a_lock_without_an_index_is_refused_before_pip_runs(self):
        self.refused(lambda t: t.replace("--index-url https://pypi.org/simple\n", ""), "--index-url")

    def test_a_lock_with_a_duplicate_package_is_refused_before_pip_runs(self):
        def dup(t):
            m = re.search(r"^(numpy==[^\n]*\\\n(?:\s+--hash=sha256:[0-9a-f]{64}(?: \\)?\n)+)", t, re.M)
            return t + m.group(1)
        self.refused(dup, "twice")

    def test_a_tensorrt_lock_without_nvidia_s_index_is_refused_before_pip_runs(self):
        text = (ML / "locks/tensorrt_cu13.lock.txt").read_text().replace("--extra-index-url https://pypi.nvidia.com/\n", "")
        (self.t / "bad.lock").write_text(text)
        with mock.patch.object(mr, "lock_path", lambda *a: self.t / "bad.lock"):
            st, why = self.setup("tensorrt")
        self.assertEqual(st, lib.FAIL)
        self.assertIn("pypi.nvidia.com", why)
        self.assertEqual(self.calls, [])

    def test_a_lock_that_disagrees_with_the_pins_is_refused_before_pip_runs(self):
        self.refused(lambda t: t.replace("jax==0.12.0.1", "jax==0.11.0", 1), "pins.json says 0.12.0.1")

    def test_a_pip_failure_other_than_hashes_is_named_and_leaves_no_ok_state(self):
        self.pip_output = "ERROR: Could not find a version that satisfies the requirement x\n"
        st, why = self.setup("jax")
        self.assertEqual(st, lib.FAIL)
        self.assertTrue(why.startswith("pip_failed"), why)
        self.assertFalse((self.t / "jax" / "setup.json").exists())

    def test_a_second_setup_does_not_touch_the_staged_files(self):
        self.setup("jax")
        marker = self.t / "jax" / "venv" / "bin" / "marker"
        marker.write_text("x")
        self.setup("jax")
        self.assertTrue(marker.exists())

    def test_a_changed_lock_starts_clean(self):
        self.setup("jax")
        (self.t / "jax" / "venv" / "bin" / "old").write_text("x")
        changed = (ML / "locks/jax_cu13.lock.txt").read_text() + "\n# a comment changes the bytes\n"
        (self.t / "changed.lock").write_text(changed)
        with mock.patch.object(mr, "lock_path", lambda *a: self.t / "changed.lock"):
            st, _ = self.setup("jax")
        self.assertEqual(st, "ok")
        self.assertFalse((self.t / "jax" / "venv" / "bin" / "old").exists())
        self.assertEqual(json.loads((self.t / "jax" / "setup.json").read_text())["lock_sha256"], lib.lock_sha256(changed))

    def test_a_changed_lock_removes_what_the_old_one_installed(self):
        self.setup("jax")
        n = len([c for c in self.calls if "pip" in c])
        (self.t / "changed.lock").write_text((ML / "locks/jax_cu13.lock.txt").read_text() + "\n# changed\n")
        with mock.patch.object(mr, "lock_path", lambda *a: self.t / "changed.lock"):
            self.setup("jax")
        self.assertEqual(len([c for c in self.calls if "pip" in c]), n + 1)

    def test_vllm_fetches_the_model_after_pip_and_a_bad_model_fails_the_setup(self):
        origin = Model()
        good, _ = origin.fake_origin()
        st, why = self.setup("vllm", get=good)
        self.assertEqual(st, "ok", why)
        self.assertTrue((self.t / "vllm" / "model" / "model.json").is_file())
        self.assertEqual(json.loads((self.t / "vllm" / "setup.json").read_text())["model"][:1], "8")
        bad, _ = origin.fake_origin(tamper="model.safetensors")
        __import__("shutil").rmtree(self.t / "vllm")
        st, why = self.setup("vllm", get=bad)
        self.assertEqual(st, lib.FAIL)
        self.assertTrue(why.startswith("model_fetch_failed: hash_mismatch"), why)
        self.assertFalse((self.t / "vllm" / "setup.json").exists())


# ------------------------------------------------------------------------------------------------ host against guest
def judged(row, status=lib.PASS, **result):
    base = {"vllm": vllm_result, "tensorrt": trt_result, "jax": jax_result}[row]()
    base.update(result)
    return {"status": status, "reason": "ok" if status == lib.PASS else "x", "result": base}


class Compare(unittest.TestCase):
    def kinds(self, row, host, guest):
        return [f["kind"] for f in lib.compare_rows(row, host, guest)]

    def test_identical_results_have_no_candidate(self):
        for row in lib.ROWS:
            self.assertEqual(self.kinds(row, judged(row), judged(row)), [], row)

    def test_different_greedy_tokens_are_a_candidate_naming_the_first_prompt(self):
        g = judged("vllm")
        ids = [list(t) for t in g["result"]["token_ids"]]
        ids[3][1] += 1
        g["result"]["token_ids"], g["result"]["token_sha256"] = ids, lib.canon_sha256(ids)
        f = lib.compare_rows("vllm", judged("vllm"), g)
        self.assertEqual([x["kind"] for x in f], ["CANDIDATE"])
        self.assertIn("first differing prompt: 3", f[0]["what"])

    def test_graph_size_count_and_total_memory(self):
        f = lib.compare_rows("vllm", judged("vllm"), judged("vllm", cudagraph_n_sizes=35, total_mib=3800))
        self.assertEqual(sorted(x["kind"] for x in f), ["CANDIDATE", "INFO"])

    def test_a_guest_failure_after_a_host_pass_is_a_candidate_and_a_not_run_is_only_a_note(self):
        self.assertEqual(self.kinds("jax", judged("jax"), judged("jax", lib.FAIL)), ["CANDIDATE"])
        self.assertEqual(self.kinds("jax", judged("jax"), judged("jax", lib.NOT_RUN)), ["NOTE"])
        self.assertEqual(self.kinds("jax", judged("jax", lib.FAIL), judged("jax")), ["NOTE"])
        self.assertEqual(self.kinds("jax", judged("jax", lib.FAIL), judged("jax", lib.FAIL)), [])

    def test_a_missing_side_is_a_note_never_a_candidate(self):
        self.assertEqual(self.kinds("jax", None, judged("jax")), ["NOTE"])
        self.assertEqual(self.kinds("jax", judged("jax"), None), ["NOTE"])

    def test_jax_preallocation_fraction_and_autotuner(self):
        self.assertEqual(self.kinds("jax", judged("jax"), judged("jax", bytes_limit_mib=12000.0)), ["CANDIDATE"])
        self.assertEqual(self.kinds("jax", judged("jax"), judged("jax", autotune_bytes=0)), ["CANDIDATE"])
        self.assertEqual(self.kinds("jax", judged("jax"), judged("jax", total_mib=3800, bytes_limit_mib=3800 * 0.75)), ["INFO"])

    def test_tensorrt_engine_size_and_the_ort_provider(self):
        b = dict(trt_result()["builder"], engine_bytes=2_000_000)
        self.assertEqual(self.kinds("tensorrt", judged("tensorrt"), judged("tensorrt", builder=b)), ["CANDIDATE"])
        self.assertEqual(self.kinds("tensorrt", judged("tensorrt"), judged("tensorrt", ort_trt={"status": "NOT_RUN"})), ["CANDIDATE"])
        self.assertEqual(self.kinds("tensorrt", judged("tensorrt"), judged("tensorrt", onnx_sha256="c" * 64)), ["NOTE"])

    def test_a_not_run_guest_alone_does_not_fail_the_comparison(self):
        t = Path(tempfile.mkdtemp())
        for name, j in (("h.log", judged("jax")), ("g.log", judged("jax", lib.NOT_RUN))):
            (t / name).write_text(lib.judged_line("jax", j["status"], j["reason"], "cu13", [], j["result"]) + "\n")
        buf = io.StringIO()
        with mock.patch("sys.stdout", buf):
            rc = mr.main(["compare", "jax", str(t / "h.log"), str(t / "g.log")])
        self.assertEqual(rc, 0)
        self.assertIn("ML_COMPARE_NOTE jax: host PASS, guest NOT_RUN", buf.getvalue())

    def test_the_autotuner_loss_alone_is_a_candidate(self):
        self.assertEqual(self.kinds("jax", judged("jax"), judged("jax", autotune_bytes=0)), ["CANDIDATE"])

    def test_cmd_compare_sees_different_tokens_in_two_logs(self):
        t = Path(tempfile.mkdtemp())
        h, g = judged("vllm"), judged("vllm")
        ids = [list(x) for x in g["result"]["token_ids"]]
        ids[0][0] += 1
        g["result"]["token_ids"], g["result"]["token_sha256"] = ids, lib.canon_sha256(ids)
        for name, j in (("h.log", h), ("g.log", g)):
            (t / name).write_text(lib.judged_line("vllm", j["status"], j["reason"], "cu13", [], j["result"]) + "\n")
        buf = io.StringIO()
        with mock.patch("sys.stdout", buf):
            rc = mr.main(["compare", "vllm", str(t / "h.log"), str(t / "g.log")])
        self.assertEqual(rc, 1)
        self.assertIn("greedy token ids differ", buf.getvalue())

    def test_a_forged_status_in_a_log_is_not_a_result(self):
        self.assertIsNone(lib.extract_judged(lib.RESULT_PREFIX + '{"row":"jax","status":"GREAT"}\n', "jax"))
        self.assertIsNone(lib.extract_judged(lib.RESULT_PREFIX + '{"row":"jax","status":"pass"}\n', None))

    def test_a_log_with_another_row_s_result_gives_nothing(self):
        text = lib.judged_line("vllm", lib.PASS, "ok", "cu13", [], {}) + "\n"
        self.assertIsNone(lib.extract_judged(text, "jax"))
        self.assertIsNotNone(lib.extract_judged(text, None))

    def test_cmd_compare_reads_two_logs_and_exits_one_on_a_candidate(self):
        t = Path(tempfile.mkdtemp())
        h, g = judged("jax"), judged("jax", lib.FAIL)
        for name, j in (("h.log", h), ("g.log", g)):
            (t / name).write_text("noise\n" + lib.judged_line("jax", j["status"], j["reason"], "cu13", [], j["result"]) + "\n")
        buf = io.StringIO()
        with mock.patch("sys.stdout", buf):
            rc = mr.main(["compare", "jax", str(t / "h.log"), str(t / "g.log")])
        self.assertEqual(rc, 1)
        self.assertIn("ML_COMPARE_CANDIDATE jax: host PASS, guest FAIL", buf.getvalue())
        self.assertIn("ML_COMPARE_SUMMARY row=jax host=PASS guest=FAIL candidates=1", buf.getvalue())

    def test_extract_ignores_a_forged_line_with_a_bad_status_and_other_rows(self):
        text = lib.RESULT_PREFIX + '{"row":"jax","status":"GREAT"}\n' + lib.judged_line("vllm", lib.PASS, "ok", "cu13", [], {}) + "\n"
        self.assertIsNone(lib.extract_judged(text, "jax"))
        self.assertIsNotNone(lib.extract_judged(text, "vllm"))
        self.assertIsNone(lib.extract_judged(lib.RESULT_PREFIX + "{not json", None))

    def test_extract_result_takes_the_last_dict_line_and_ignores_garbage(self):
        text = lib.JSON_PREFIX + '{"a":1}\n' + lib.JSON_PREFIX + "[1]\n" + lib.JSON_PREFIX + "{bad\n" + lib.JSON_PREFIX + '{"a":2}\n'
        self.assertEqual(lib.extract_result(text), {"a": 2})
        self.assertIsNone(lib.extract_result("MLROW_JSON"))


# ------------------------------------------------------------------------------------------------ pushing into a guest
class Push(unittest.TestCase):
    def test_the_tool_dir_and_the_staged_rows_go_through_the_transport_as_tar(self):
        import tarfile
        t = Path(tempfile.mkdtemp())
        root = t / "root"
        (root / "jax/venv/bin").mkdir(parents=True)
        (root / "jax/venv/bin/python").write_text("py")
        (root / "jax/out").mkdir()
        (root / "jax/out/big.log").write_text("x")
        (root / "jax/pip.log").write_text("x")
        (root / "jax/setup.json").write_text("{}")
        sink = t / "sink"
        # the transport runs `sh -c 'cat > SINK' <remote command>`: the remote command is the last argument and lands in $0
        transport = ["sh", "-c", "cat > $(mktemp %s_XXXXXX)" % sink]
        buf = io.StringIO()
        with mock.patch("sys.stdout", buf):
            rc = mr.main(["push", "--root", str(root), "--rows", "jax,vllm", "--transport"] + transport)
        self.assertEqual(rc, 0, buf.getvalue())
        self.assertEqual(buf.getvalue().count("MLROWS_PUSH what="), 2)            # the tool dir and jax (vllm was not staged: skipped, not failed)
        self.assertIn("what=mlrows_tool rc=0", buf.getvalue())
        names = []
        for f in t.glob("sink_*"):
            with tarfile.open(f) as tf:
                names += tf.getnames()
        self.assertIn("mlrows.py", " ".join(names))
        self.assertTrue(any(n.endswith("jax/venv/bin/python") for n in names))
        self.assertFalse(any("big.log" in n or "pip.log" in n for n in names))
        self.assertFalse(any("__pycache__" in n for n in names))

    def test_a_failing_transport_fails_the_push(self):
        t = Path(tempfile.mkdtemp())
        buf = io.StringIO()
        with mock.patch("sys.stdout", buf):
            rc = mr.main(["push", "--root", str(t), "--rows", "jax", "--transport", "sh", "-c", "cat >/dev/null; exit 3"])
        self.assertNotEqual(rc, 0)
        self.assertIn("what=mlrows_tool rc=3", buf.getvalue())


# ------------------------------------------------------------------------------------------------ the matrix wiring and the GPU-use judge
RUN_APPS = (ROOT / "scripts/apps/run_apps.sh").read_text()
tg = load("test_gpu_use_for_ml", HERE / "test_gpu_use.py")


class MatrixWiring(unittest.TestCase):
    def bash(self, args, root, env=None):
        e = dict(os.environ, APPS_ROOT=str(root))
        e.update(env or {})
        return subprocess.run(["bash", str(ROOT / "scripts/apps/run_apps.sh")] + args, capture_output=True, text=True, env=e, timeout=60)

    def test_the_rows_are_listed_but_not_in_listall_and_in_no_gate_list(self):
        with tempfile.TemporaryDirectory() as t:
            lst = self.bash(["x", "list"], t).stdout.split()
            allr = self.bash(["x", "listall"], t).stdout.split()
        for row in lib.ROWS:
            self.assertIn(row, lst)
            self.assertNotIn(row, allr)
        self.assertIn("nvidia_smi", allr)
        for f in ("gate_list.txt", "gate_list_middle.txt", "expected_rows.txt"):
            names = set((ROOT / "scripts/apps" / f).read_text().split())
            for row in lib.ROWS:
                self.assertNotIn(row, names, f)

    def test_each_row_runs_mlrows_with_its_own_name_root_and_out_and_a_timeout_above_the_row_s(self):
        for row in lib.ROWS:
            m = re.search(r"^%s\|(\d+)\|(\w+_SUMMARY status=PASS)\|(.*)$" % row, RUN_APPS, re.M)
            self.assertIsNotNone(m, row)
            self.assertGreater(int(m.group(1)), PINS["rows"][row]["timeout_s"] + 60, row)
            self.assertEqual(m.group(2), {"vllm": "VLLM_SUMMARY", "tensorrt": "TENSORRT_SUMMARY", "jax": "JAX_SUMMARY"}[row] + " status=PASS")
            self.assertEqual(m.group(3), "python3 -B -I $B/mlrows_tool/mlrows.py run %s --root $B/mlrows --out $O/%s" % (row, row))

    def fake_tool(self, root, body):
        d = root / "bundle" / "mlrows_tool"
        d.mkdir(parents=True)
        (d / "mlrows.py").write_text(body)

    def row_verdict(self, body):
        with tempfile.TemporaryDirectory() as t:
            self.fake_tool(Path(t), body)
            out = self.bash(["host", "vllm"], t).stdout
        return re.search(r"APPRES side=host app=vllm verdict=(\w+) rc=(\d+) .* note=(.*)$", out, re.M)

    def test_a_pass_summary_is_a_pass(self):
        m = self.row_verdict("print('VLLM_SUMMARY status=PASS flavour=cu13')\n")
        self.assertEqual(m.group(1), "PASS")

    def test_the_rows_own_note_line_is_the_note_of_a_pass(self):
        m = self.row_verdict("print('VLLM_SUMMARY status=PASS')\nprint('APPNOTE PASS on cu13: ok')\nprint('(MainProcess) INFO shutdown timeout=default')\n")
        self.assertEqual((m.group(1), m.group(3)), ("PASS", "APPNOTE PASS on cu13: ok".replace("APPNOTE ", "")))

    def test_a_not_run_is_notrun_with_its_reason_and_never_a_pass_or_a_fail(self):
        m = self.row_verdict("print('VLLM_SUMMARY status=NOT_RUN reason=x')\nprint('APPNOTRUN driver_cuda_below_every_wheel: cu13: driver supports CUDA 12.9 < 13.0')\n")
        self.assertEqual(m.group(1), "NOTRUN")
        self.assertIn("not-run:driver_cuda_below_every_wheel", m.group(3))

    def test_a_crash_that_printed_appnotrun_is_still_a_fail(self):
        m = self.row_verdict("print('APPNOTRUN x')\nraise SystemExit(1)\n")
        self.assertEqual(m.group(1), "FAIL")

    def test_a_fail_summary_is_a_fail_even_with_exit_zero(self):
        m = self.row_verdict("print('VLLM_SUMMARY status=FAIL reason=x')\n")
        self.assertEqual(m.group(1), "FAIL")

    def test_a_nonzero_exit_that_printed_appnotrun_is_still_a_fail(self):
        m = self.row_verdict("print('APPNOTRUN x')\nraise SystemExit(2)\n")
        self.assertEqual((m.group(1), m.group(2)), ("FAIL", "2"))

    def test_the_real_tool_with_no_gpu_is_notrun_through_run_apps(self):
        with tempfile.TemporaryDirectory() as t:
            (Path(t) / "bundle").mkdir()
            os.symlink(ML, Path(t) / "bundle" / "mlrows_tool")
            fake = Path(t) / "fakebin"
            fake.mkdir()
            (fake / "nvidia-smi").write_text("#!/bin/sh\nexit 9\n")
            (fake / "nvidia-smi").chmod(0o755)
            out = self.bash(["host", "vllm"], t, {"PATH": "%s:%s" % (fake, os.environ["PATH"])}).stdout
        m = re.search(r"APPRES side=host app=vllm verdict=(\w+) rc=(\d+) .* note=(.*)$", out, re.M)
        self.assertEqual(m.group(1), "NOTRUN", out)
        self.assertIn("not-run:no_gpu_or_no_nvidia_smi", m.group(3))

    def test_list_is_listall_plus_exactly_the_three_rows(self):
        with tempfile.TemporaryDirectory() as t:
            lst = self.bash(["x", "list"], t).stdout.split()
            allr = self.bash(["x", "listall"], t).stdout.split()
        self.assertEqual(sorted(set(lst) - set(allr)), sorted([*lib.ROWS, "vkcts"]))   # the union: the Vulkan CTS row is the fourth discovery row
        self.assertLessEqual(set(allr), set(lst))

    def test_all_and_listall_leave_the_discovery_rows_out(self):
        self.assertIn('[ "${1:-}" = all ] && set -- $(echo "$APPS" | cut', RUN_APPS)
        self.assertIn('[ "${1:-}" = listall ] && { echo "$APPS" | cut', RUN_APPS)

    def test_all_runs_no_discovery_row(self):
        with tempfile.TemporaryDirectory() as t:
            out = self.bash(["host", "all"], t, {"PATH": "/usr/bin:/bin"}).stdout
            n = len(self.bash(["x", "listall"], t).stdout.split())
        apps = re.findall(r"^APPRES side=host app=(\w+) ", out, re.M)
        self.assertEqual(len(apps), n)
        self.assertFalse(set(apps) & set(lib.ROWS))

    def test_a_discovery_row_never_reaches_the_gate_list(self):
        mt = load("kf_matrix_table_ml2", ROOT / "scripts/apps/matrix_table.py")
        for row in lib.ROWS:
            self.assertNotEqual(mt.classify(row, {"verdict": "PASS"}, {"verdict": "PASS"}, None)[0], "GATE", row)

    def test_the_hook_pushes_the_rows_it_was_asked_for_before_the_guest_runs_anything(self):
        hook = (ROOT / "scripts/apps/apps_hook.sh").read_text()
        push = hook.index('mlrows/mlrows.py" push')
        self.assertLess(push, hook.index("sudo rm -rf /opt/apps/out/guest"))
        self.assertLess(push, hook.index("for app in $APPS; do"))
        self.assertIn("case $a in vllm|tensorrt|jax)", hook)

    def test_the_host_run_sets_the_rows_up_before_it_takes_the_gpu_lock(self):
        matrix = (ROOT / "scripts/apps/apps_matrix.sh").read_text()
        self.assertLess(matrix.index('mlrows/mlrows.py" setup'), matrix.index("flock 9   # the host GPU is the bench GPU"))

    def test_the_matrix_compares_host_and_guest_for_each_row_after_the_guest_runs(self):
        matrix = (ROOT / "scripts/apps/apps_matrix.sh").read_text()
        self.assertIn("for row in vllm tensorrt jax; do", matrix)
        self.assertLess(matrix.index('mlrows/mlrows.py" compare'), matrix.index('say "GUEST_DONE batched:'))
        self.assertGreater(matrix.index('mlrows/mlrows.py" compare'), matrix.index("phase 2"))

    def test_the_matrix_table_classes_them_discovery_and_the_gpu_judge_has_a_class_for_each(self):
        mt = load("kf_matrix_table_ml", ROOT / "scripts/apps/matrix_table.py")
        for row in lib.ROWS:
            self.assertEqual(mt.classify(row, {"verdict": "PASS"}, {"verdict": "FAIL"}, None)[0], "DISCOVERY")
            self.assertIn(row, gur.ROWS)
        self.assertEqual({r: gur.row_class(r) for r in lib.ROWS}, {"vllm": "a+proof", "tensorrt": "b", "jax": "b"})

    def test_the_shell_glue_pushes_sets_up_and_compares_the_rows(self):
        hook = (ROOT / "scripts/apps/apps_hook.sh").read_text()
        self.assertIn('mlrows/mlrows.py" push --root', hook)
        self.assertIn('--transport "$G"', hook)
        self.assertIn("case $a in vllm|tensorrt|jax)", hook)
        matrix = (ROOT / "scripts/apps/apps_matrix.sh").read_text()
        self.assertIn('mlrows/mlrows.py" setup --root', matrix)
        self.assertIn('mlrows/mlrows.py" compare "$row"', matrix)
        self.assertIn("listall", matrix)

    def test_the_hook_and_matrix_glue_stay_short_shell(self):
        for f in ("apps_hook.sh", "apps_matrix.sh"):
            self.assertLessEqual(len((ROOT / "scripts/apps" / f).read_text().splitlines()), 140, f)


class HookPush(unittest.TestCase):
    """The two shell lines of apps_hook.sh that copy the rows into the guest, run for real with a fake transport (they are the only part of the hook these rows add)."""
    hook = (ROOT / "scripts/apps/apps_hook.sh").read_text()

    def lines(self):
        a = self.hook.index("MLR=$(for a in $APPS")
        b = self.hook.index("$G 'sudo rm -rf /opt/apps/out/guest'")
        return self.hook[a:b]

    def run_lines(self, apps, staged):
        t = Path(tempfile.mkdtemp())
        for row in staged:
            (t / "root" / "mlrows" / row / "venv" / "bin").mkdir(parents=True)
            (t / "root" / "mlrows" / row / "venv" / "bin" / "python").write_text("py")
        sink = t / "sink"
        sink.mkdir()
        (t / "g").write_text("#!/bin/sh\ncat > \"$(mktemp %s/XXXXXX)\"\n" % sink)
        (t / "g").chmod(0o755)
        script = 'HERE="%s"; APPS="%s"; G="%s"; set -u\n%s' % (ROOT / "scripts/apps", apps, t / "g", self.lines())
        done = subprocess.run(["bash", "-c", script], capture_output=True, text=True, env=dict(os.environ, KF_APPS_ROOT=str(t / "root")), timeout=60)
        return done, sorted(sink.iterdir())

    def test_the_rows_asked_for_and_staged_are_pushed_with_the_tool_dir(self):
        done, sent = self.run_lines("nvidia_smi jax vllm", ["jax", "vllm"])
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(done.stdout.count("MLROWS_PUSH what="), 3)
        for what in ("mlrows_tool", "jax", "vllm"):
            self.assertIn("what=%s rc=0" % what, done.stdout)
        self.assertEqual(len(sent), 3)

    def test_a_run_without_ml_rows_pushes_nothing(self):
        done, sent = self.run_lines("nvidia_smi vectorAdd", ["jax"])
        self.assertEqual((done.stdout, sent), ("", []))                     # the hook has no `set -e`: the false `[ -n ]` test is harmless there

    def test_a_row_that_was_not_staged_is_left_out_and_the_tool_dir_still_goes(self):
        done, sent = self.run_lines("tensorrt", [])
        self.assertIn("what=mlrows_tool rc=0", done.stdout)
        self.assertNotIn("what=tensorrt", done.stdout)


class GpuUseJudge(unittest.TestCase):
    """The shared judge (PR #475) over these rows: a GPU run passes, and every CPU form the row can take is judged FAIL by the host census and by the app proof alone."""
    base = tg.fx("kf3_base.txt").strip()
    window = tg.fx("kf3_window_gpu.log").splitlines()

    def log(self, row, res, f=None):
        line = lib.proof_for(row, res, f or facts())
        return "\n".join(["=== x side=guest start=now", "log line", line or ""])

    def evaluate(self, row, text, busy=True, kf3=True):
        kf3_lines = self.window if kf3 else tg.fx("kf3_window_idle.log").splitlines()
        base = self.base if kf3 else tg.fx("kf3_base_end.txt").strip()
        return gur.evaluate(row, "guest", text, kf3_lines, base, tg.smi_for(row, busy))

    def good(self, row):
        return {"vllm": vllm_result(), "tensorrt": trt_result(), "jax": jax_result()}[row]

    def test_the_gpu_run_of_every_row_passes_with_the_census_and_the_engines(self):
        for row in lib.ROWS:
            rec = self.evaluate(row, self.log(row, self.good(row)))
            self.assertEqual(rec["verdict"], gu.PASS, (row, rec["reasons"]))
            self.assertEqual(rec["authority"], "host")

    def test_the_cpu_forms_are_judged_fail_by_the_app_proof_alone(self):
        for row, res in (("jax", jax_result(platform="cpu", mem_growth_mib=0)), ("tensorrt", trt_result(platform="cpu", mem_growth_mib=0)), ("vllm", None)):
            rec = gur.evaluate(row, "host", self.log(row, res), None, None, None)
            self.assertEqual(rec["verdict"], gu.FAIL, (row, rec["reasons"]))

    def test_the_cpu_forms_are_judged_fail_by_the_host_alone_when_the_guest_lies(self):
        for row in lib.ROWS:
            rec = self.evaluate(row, self.log(row, self.good(row)), busy=False, kf3=False)       # the guest printed a perfect GPU proof; the host saw no work
            self.assertEqual(rec["verdict"], gu.FAIL, (row, rec["reasons"]))
            self.assertTrue(any("[host-" in r and "FAIL" in r for r in rec["reasons"]), rec["reasons"])

    def test_a_jax_run_the_host_saw_idle_is_a_fail(self):
        rec = gur.evaluate("jax", "guest", self.log("jax", jax_result()), self.window, self.base, tg.smi_for("jax", False))
        self.assertEqual(rec["verdict"], gu.FAIL, rec["reasons"])
        self.assertTrue(any("[host-smi]" in r and "FAIL" in r for r in rec["reasons"]), rec["reasons"])

    def test_an_idle_host_engine_fails_every_row_whose_census_is_fine(self):
        for row in lib.ROWS:
            rec = gur.evaluate(row, "guest", self.log(row, self.good(row)), self.window, self.base, tg.smi_for(row, False))
            self.assertEqual(rec["verdict"], gu.FAIL, (row, rec["reasons"]))

    def test_a_cpu_run_with_plenty_of_memory_growth_is_still_judged_fail_by_the_app_proof(self):
        rec = gur.evaluate("jax", "host", "\n".join(["log", lib.proof_for("jax", dict(jax_result(), platform="cpu", mem_growth_mib=9000), facts())]), None, None, None)
        self.assertEqual(rec["verdict"], gu.FAIL, rec["reasons"])

    def test_a_row_that_dropped_its_proof_is_a_fail_not_a_pass(self):
        for row in lib.ROWS:
            self.assertEqual(self.evaluate(row, "log line only")["verdict"], gu.FAIL, row)

    def test_too_little_device_memory_is_a_fail(self):
        for row in lib.ROWS:
            res = dict(self.good(row), mem_growth_mib=gur.policy(row)["params"]["min_mib"] - 1)
            self.assertEqual(self.evaluate(row, self.log(row, res))["verdict"], gu.FAIL, row)

    def test_a_pass_of_the_row_is_rewritten_to_fail_when_the_proof_says_cpu(self):
        line = "APPRES side=guest app=jax verdict=PASS rc=0 secs=3 quiet=0 note=-"
        new, rec = gur.annotate_line(line, "host", self.log("jax", jax_result(platform="cpu", mem_growth_mib=0)))
        self.assertIn(" verdict=FAIL ", new)
        self.assertIn("gpu-use:FAIL", new)

    def test_the_negative_control_of_each_row_is_control_ok(self):
        for row, res in (("jax", jax_result(platform="cpu", mem_growth_mib=0)), ("tensorrt", trt_result(platform="cpu", mem_growth_mib=0)), ("vllm", None)):
            line = "APPRES side=host app=%s verdict=PASS rc=0 secs=3 quiet=0 note=-" % row
            _, rec = gur.annotate_line(line, "host", self.log(row, res), negctl=True)
            self.assertTrue(gur.control_line(row, rec).startswith("NEGCTL app=%s control=CONTROL_OK" % row), gur.control_line(row, rec))


class Standalone(unittest.TestCase):
    """run_standalone.sh with a fake tool next to it, a fake apt-get and a fake id: what it runs, in which order, and how it ends."""
    script = ML / "run_standalone.sh"

    def run_it(self, tool_body, uid="0", ensure_rc=0):
        t = Path(tempfile.mkdtemp())
        (t / "tool").mkdir()
        shutil_copy = __import__("shutil").copyfile
        shutil_copy(self.script, t / "tool" / "run_standalone.sh")
        (t / "tool" / "mlrows.py").write_text(tool_body)
        (t / "tool" / "private_python.py").write_text("import sys\nprint('PRIVATE_PYTHON ' + ' '.join(sys.argv[1:]), flush=True)\nsys.exit(" + str(ensure_rc) + ")\n")
        fake = t / "bin"
        fake.mkdir()
        (fake / "apt-get").write_text("#!/bin/sh\necho \"apt-get $*\" >> %s\n" % (t / "calls"))
        (fake / "id").write_text("#!/bin/sh\necho %s\n" % uid)
        for f in fake.iterdir():
            f.chmod(0o755)
        env = dict(os.environ, PATH="%s:%s" % (fake, os.environ["PATH"]), DEBIAN_FRONTEND="")
        done = subprocess.run(["bash", str(t / "tool" / "run_standalone.sh"), str(t / "out")], capture_output=True, text=True, env=env, timeout=60)
        calls = (t / "calls").read_text() if (t / "calls").exists() else ""
        return done, calls, t

    TOOL = ("import sys\nprint('MLROWS %s' % ' '.join(sys.argv[1:]), flush=True)\n"
            "row = sys.argv[2] if sys.argv[1] == 'run' else None\n"
            "if row: print({'jax': 'JAX_SUMMARY status=PASS', 'tensorrt': 'TENSORRT_SUMMARY status=FAIL reason=x', 'vllm': 'APPNOTRUN driver_cuda_below_every_wheel'}[row])\n"
            "sys.exit(1 if row == 'tensorrt' else 0)\n")

    def test_it_installs_the_prerequisites_as_root_checks_sets_up_then_runs_every_row(self):
        done, calls, t = self.run_it(self.TOOL)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn("apt-get install -y -qq python3 python3-venv python3-dev gcc ca-certificates", calls)
        order = [l.split()[1] for l in done.stdout.splitlines() if l.startswith("MLROWS ")]
        self.assertEqual(order, ["check", "setup"])
        for row in ("jax", "tensorrt", "vllm"):
            self.assertIn("MLROWS run %s --root %s --out %s" % (row, t / "out" / "root", t / "out" / row), (t / "out" / (row + ".log")).read_text())
        self.assertIn("JAX_SUMMARY status=PASS", done.stdout)
        self.assertIn("TENSORRT_SUMMARY status=FAIL", done.stdout)               # a failing row does not stop the others
        self.assertIn("APPNOTRUN driver_cuda_below_every_wheel", done.stdout)

    def test_a_host_that_needs_it_gets_the_private_python_before_check_and_setup_and_a_failed_build_does_not_stop_the_run(self):
        for rc in (0, 3):
            done, _, t = self.run_it(self.TOOL, ensure_rc=rc)
            lines = [l for l in done.stdout.splitlines() if l.startswith(("PRIVATE_PYTHON", "MLROWS "))]
            self.assertEqual(lines[0], "PRIVATE_PYTHON ensure --root %s --if-needed" % (t / "out" / "root"), rc)
            self.assertEqual([l.split()[1] for l in lines[1:]], ["check", "setup"], rc)
            self.assertEqual(done.returncode, 0, done.stderr)

    def test_a_non_root_user_gets_no_apt_call(self):
        done, calls, _ = self.run_it(self.TOOL, uid="1000")
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(calls, "")

    def test_a_failed_setup_stops_the_run(self):
        done, _, _ = self.run_it("import sys\nprint('MLROWS', sys.argv[1])\nsys.exit(3 if sys.argv[1] == 'setup' else 0)\n")
        self.assertEqual(done.returncode, 3)
        self.assertNotIn("MLROWS run", done.stdout)

    def test_a_row_that_printed_no_summary_is_named(self):
        done, _, t = self.run_it("import sys\nsys.exit(0)\n")
        self.assertIn("jax: no summary line", done.stdout)

    def test_it_is_short_linear_shell(self):
        text = self.script.read_text()
        self.assertIn("set -euo pipefail", text)
        self.assertLessEqual(len(text.splitlines()), 30)
        for bad in ("curl", "wget", "pip install", "git clone", "sudo"):
            self.assertNotIn(bad, text)


# ------------------------------------------------------------------------------------------------ the real runs (fixtures captured on a Vast container, RTX 4060 Ti, 2026-11-17)
REAL = {row: fx("%s_run_rtx4060ti_580.log" % row) for row in lib.ROWS}
REAL_NEG = {row: fx("%s_negctl_rtx4060ti_580.log" % row) for row in lib.ROWS}
REAL_FACTS = facts(name="NVIDIA GeForce RTX 4060 Ti", driver="580.178.04", cc=(8, 9), total_mib=16380)


class RealRuns(unittest.TestCase):
    """The lines a native run printed, through today's judges: format drift between the runner and its logs shows here, and the judges are tested on real numbers."""

    def test_every_real_run_is_a_pass_and_re_judging_its_measurements_gives_the_same_verdict(self):
        for row in lib.ROWS:
            judged_line = lib.extract_judged(REAL[row], row)
            self.assertEqual(judged_line["status"], lib.PASS, row)
            res = lib.extract_result(REAL[row])
            st, why, checks = lib.JUDGES[row](res)
            self.assertEqual((st, why), (lib.PASS, "ok"), (row, why))
            self.assertEqual([c["name"] for c in checks], [c["name"] for c in judged_line["checks"]], row)

    def test_the_proof_line_the_run_printed_is_the_one_the_runner_would_print_now(self):
        for row in lib.ROWS:
            res = lib.extract_judged(REAL[row], row)["result"]
            printed = [l for l in REAL[row].splitlines() if l.startswith("GPU_USE_PROOF")]
            self.assertEqual(printed, [lib.proof_for(row, res, REAL_FACTS)], row)

    def test_the_real_proof_lines_pass_the_judge_with_the_floors_of_the_class_table(self):
        for row in lib.ROWS:
            p = gur.policy(row)
            proof = gu.PROOFS[p["proof"]](REAL[row], **p["params"])
            self.assertIs(proof["ok"], True, (row, proof))

    def test_the_summary_line_carries_the_status_and_for_vllm_the_token_hash(self):
        vl = [l for l in REAL["vllm"].splitlines() if l.startswith("VLLM_SUMMARY ")][0]
        res = lib.extract_result(REAL["vllm"])
        self.assertIn("status=PASS", vl)
        self.assertIn("token_sha256=%s" % res["token_sha256"], vl)
        self.assertEqual(res["token_sha256"], lib.canon_sha256(res["token_ids"]))

    def test_the_real_tokens_pass_the_committed_hash_through_the_runner_and_made_up_ones_do_not(self):
        t = Path(tempfile.mkdtemp())
        (t / "root" / "vllm" / "venv" / "bin").mkdir(parents=True)
        (t / "root" / "vllm" / "venv" / "bin" / "python").write_text("")
        (t / "root" / "vllm" / "setup.json").write_text(json.dumps({"status": "ok", "flavour": "cu13"}))
        res = lib.extract_result(REAL["vllm"])
        for k in ("mem_base_mib", "mem_peak_mib", "mem_growth_mib", "total_mib", "secs", "negctl", "log_facts"):
            res.pop(k, None)
        with mock.patch.object(mr, "Sampler", FakeSampler), mock.patch.object(mr.time, "sleep", lambda s: None):
            ok = mr.run_row("vllm", t / "root", t / "out", REAL_FACTS, PINS, popen=lambda argv, **kw: FakeProc([lib.JSON_PREFIX + json.dumps(res)]))
            fake = dict(res, token_ids=[[1, 2, 3]] * 8)
            fake["token_sha256"] = lib.canon_sha256(fake["token_ids"])
            bad = mr.run_row("vllm", t / "root", t / "out2", REAL_FACTS, PINS, popen=lambda argv, **kw: FakeProc([lib.JSON_PREFIX + json.dumps(fake)]))
        self.assertEqual((ok[0], ok[1]), (lib.PASS, "ok"), ok[1])
        self.assertEqual(bad[0], lib.FAIL)
        self.assertTrue(bad[1].startswith("tokens_equal_committed_hash"), bad[1])

    def test_the_committed_token_hash_if_any_is_what_the_real_run_produced(self):
        want = PINS["rows"]["vllm"].get("expected_token_sha256")
        if want:
            self.assertEqual(lib.extract_result(REAL["vllm"])["token_sha256"], want)

    def test_the_engine_log_facts_come_out_of_a_real_log(self):
        facts_ = lib.extract_result(REAL["vllm"])["log_facts"]
        self.assertEqual(facts_["graphs_captured"], facts_["graphs_planned"])
        self.assertGreater(facts_["kv_cache_tokens"], 100000)
        self.assertGreater(facts_["graph_capture_gib"], 0)
        again = lib.vllm_log_facts(REAL["vllm"])
        self.assertEqual({k: again[k] for k in ("kv_cache_tokens", "graphs_captured", "model_load_gib")}, {k: facts_[k] for k in ("kv_cache_tokens", "graphs_captured", "model_load_gib")})

    def test_a_real_run_compared_with_itself_has_no_finding(self):
        for row in lib.ROWS:
            j = lib.extract_judged(REAL[row], row)
            self.assertEqual(lib.compare_rows(row, j, j), [], row)

    def test_the_real_negative_controls_are_fails_and_the_gpu_use_judge_agrees(self):
        for row in lib.ROWS:
            j = lib.extract_judged(REAL_NEG[row], row)
            self.assertEqual(j["status"], lib.FAIL, row)
            p = gur.policy(row)
            rec = gur.evaluate(row, "host", REAL_NEG[row], None, None, None)
            self.assertEqual(rec["verdict"], gu.FAIL, (row, rec["reasons"]))

    def test_the_vllm_control_has_no_proof_line_because_nothing_ran(self):
        self.assertFalse([l for l in REAL_NEG["vllm"].splitlines() if l.startswith("GPU_USE_PROOF")])
        self.assertIn("RuntimeError", REAL_NEG["vllm"])

    def test_the_jax_and_tensorrt_controls_print_a_cpu_device(self):
        for row in ("jax", "tensorrt"):
            self.assertIn("GPU_USE_PROOF %s device=cpu:0 name=cpu peak_mib=0" % row, REAL_NEG[row])

    def test_the_real_results_hold_the_numbers_the_design_doc_quotes(self):
        t = lib.extract_result(REAL["tensorrt"])
        self.assertEqual((t["trt"], t["builder"]["fp16"], t["ort_trt"]["first_provider"]), ("10.16.1.11", True, "TensorrtExecutionProvider"))
        self.assertGreater(t["builder"]["tactic_lines"], 1000)
        j = lib.extract_result(REAL["jax"])
        self.assertAlmostEqual(j["bytes_limit_mib"] / j["total_mib"], 0.73, places=2)
        self.assertEqual(j["jax"], "0.12.0.1")

    def test_a_real_run_with_one_property_broken_fails_by_that_check(self):
        res = lib.extract_result(REAL["vllm"])
        res["repeat_identical"] = False
        st, why, _ = lib.judge_vllm(res)
        self.assertEqual(st, lib.FAIL)
        self.assertTrue(why.startswith("greedy_repeat_identical"))
        res = lib.extract_result(REAL["jax"])
        res["bytes_limit_mib"] = res["total_mib"] * 0.5
        self.assertTrue(lib.judge_jax(res)[1].startswith("preallocation_is_75_percent"))


# ------------------------------------------------------------------------------------------------ the trust rules, as text pins
PY_FILES = sorted(ML.glob("*.py"))


class TrustPins(unittest.TestCase):
    """Pins: they prove the line is in the file, not that it works (the install and the model checks above are behavioural)."""

    def src(self, name):
        return (ML / name).read_text()

    def test_no_row_code_shells_out_to_a_downloader_or_runs_a_shell(self):
        for p in PY_FILES:
            text = p.read_text()
            for bad in ("shell=True", "os.system(", "os.popen(", "curl ", "wget ", "git clone", "easy_install", "--trusted-host", "--no-verify", "ssl._create_unverified"):
                self.assertNotIn(bad, text, "%s in %s" % (bad, p.name))
            if p.name != "gen_locks.py":                       # the lock generator is a developer tool: nothing at run time imports or runs it
                self.assertNotIn("uv pip", text, p.name)
        self.assertNotIn("gen_locks", self.src("mlrows.py"))

    def test_pip_is_run_in_exactly_one_place_and_always_hashed(self):
        hits = [p.name for p in PY_FILES if re.search(r'"pip"\s*,\s*"install"', p.read_text())]
        self.assertEqual(hits, ["mlrows.py"])
        self.assertIn('"--require-hashes"', self.src("mlrows.py"))
        self.assertIn('"--only-binary=:all:"', self.src("mlrows.py"))
        self.assertIn('"--no-deps"', self.src("mlrows.py"))

    def test_every_url_in_the_code_is_https_and_from_an_allowed_origin(self):
        allowed = ("https://huggingface.co", "https://pypi.org/simple", "https://pypi.nvidia.com/", "https://github.com/NVIDIA/TensorRT")
        for p in PY_FILES + [ML / "pins.json"]:
            for url in re.findall(r"https?://[A-Za-z0-9.-]+(?:/[^\s\"'),\]]*)?", p.read_text()):
                if url.startswith("https://pypi.nvidia.com") or url.startswith("https://pypi.org") or url.startswith("https://huggingface.co") or url.startswith("https://github.com/NVIDIA/TensorRT"):
                    continue
                self.assertTrue(url.startswith(allowed) or "%s" in url or url.endswith("/") and "example" in url, "%s: %s" % (p.name, url))

    def test_the_row_workloads_download_nothing(self):
        for name in ("row_vllm.py", "row_tensorrt.py", "row_jax.py"):
            text = self.src(name)
            for mod in ("urllib", "requests", "socket", "http.client", "huggingface_hub", "subprocess", "pip"):
                self.assertIsNone(re.search(r"^\s*(import|from)\s+%s\b" % re.escape(mod), text, re.M), "%s imports %s" % (name, mod))

    def test_each_row_workload_prints_one_result_line_and_decides_nothing(self):
        for name in ("row_vllm.py", "row_tensorrt.py", "row_jax.py"):
            text = self.src(name)
            self.assertEqual(text.count('"MLROW_JSON "'), 1, name)
            self.assertNotIn("sys.exit(1)", text)
            for verdict in ('"PASS"', "'PASS'", "verdict"):
                self.assertNotIn(verdict, text.replace('"status": "PASS"', "").replace('"status": "NOT_RUN"', "").replace('"status": "FAIL"', ""), "%s decides: %s" % (name, verdict))

    def test_the_model_origin_is_the_only_network_use_of_setup_and_it_is_bounded(self):
        text = self.src("mlrows.py")
        self.assertIn("def http_get(url, timeout=60, limit=1 << 30)", text)
        self.assertIn('startswith("https://")', text)
        self.assertEqual(len(re.findall(r"urllib\.request\.urlopen", text)), 1)

    def test_the_row_env_never_copies_the_whole_environment(self):
        text = self.src("mlrows.py")
        self.assertNotIn("dict(os.environ)", text)
        self.assertNotIn("env = base_env.copy()", text)
        self.assertIn("ENV_KEEP", text)

    def test_the_vllm_row_turns_telemetry_off(self):
        self.assertIn('"VLLM_NO_USAGE_STATS": "1"', self.src("mlrows.py"))
        self.assertIn('"DO_NOT_TRACK": "1"', self.src("mlrows.py"))



# ------------------------------------------------------------------------------------------------ 2026-11-20: a private Python 3.12 for the host (docs/design/V3_OPEN_ML_ROWS.md section 3a)
import private_python as pp                      # noqa: E402  the module mlrows.py imports (scripts/apps/mlrows is on sys.path)
import tarfile                                   # noqa: E402

sys.path.insert(0, str(HERE))
import kf_trust as kt                            # noqa: E402
import trust_lint as tl                          # noqa: E402

BUILD_PACKAGES = ["build-essential", "libbz2-dev", "libffi-dev", "liblzma-dev", "libsqlite3-dev", "libssl-dev", "zlib1g-dev"]
FAKE_SHA = "ab" * 32                             # a made-up digest for the manifest-shape tests: the real one needs a measurement on a machine with network


def source_tarball(path, top="Python-" + pp.PY_VERSION, extra=()):
    """A tiny stand-in for the python.org tarball: one top directory with a `configure`; `extra` are (name, TarInfo-type, linkname) members that make it hostile."""
    with tarfile.open(path, "w:gz") as tf:
        for name in ("%s/configure" % top, "%s/Makefile.pre.in" % top):
            info = tarfile.TarInfo(name)
            body = b"#!/bin/sh\n"
            info.size, info.mode = len(body), 0o755
            tf.addfile(info, __import__("io").BytesIO(body))
        for name, kind, link in extra:
            info = tarfile.TarInfo(name)
            info.type, info.linkname = kind, link
            tf.addfile(info)


class PrivateBox:
    """A fake machine for private_python.ensure: every process goes through .run (argv recorded, answered from a script), nothing real starts."""

    def __init__(self, test, installed=None, fetch="ok", fail=None, probe_missing="-", probe_version="3.12.10", policy="ok", tarball=None, timeout=None):
        self.t = Path(tempfile.mkdtemp(prefix="pp-test-"))
        test.addCleanup(__import__("shutil").rmtree, self.t, True)
        self.root = self.t / "mlrows"
        self.installed = set(BUILD_PACKAGES if installed is None else installed)
        self.fetch, self.fail, self.probe_missing, self.probe_version, self.policy, self.tarball, self.timeout = fetch, fail, probe_missing, probe_version, policy, tarball, timeout
        self.calls = []

    def names(self):
        """The calls as kinds: policy, apt, dpkg, fetch, configure, make, install, probe."""
        out = []
        for c in self.calls:
            a = c["argv"]
            out.append("apt" if a[:2] == ["bash", "-c"] else "policy" if a[0] == "bash" else "dpkg" if a[0] == "dpkg-query" else "fetch" if len(a) > 3 and a[2].endswith("kf_trust.py")
                       else "configure" if a[0] == "./configure" else "install" if a[:2] == ["make", "altinstall"] else "make" if a[0] == "make" else "probe")
        return out

    def run(self, argv, env=None, timeout=60, cwd=None):
        self.calls.append({"argv": list(argv), "env": env, "cwd": cwd, "timeout": timeout})
        a = list(argv)
        if a[0] == "bash" and a[1] == "-c":
            self.installed.update(a[4:])
            return 0, "", False
        if a[0] == "bash":
            return (0, "\n".join(BUILD_PACKAGES) + "\n", False) if self.policy == "ok" else (1, "", False)
        if a[0] == "dpkg-query":
            return (0, "install ok installed", False) if a[-1] in self.installed else (1, "", False)
        if len(a) > 3 and a[2].endswith("kf_trust.py"):
            if self.fetch == "ok":
                out = Path(a[a.index("--out") + 1])
                (self.tarball or (lambda o: source_tarball(o)))(out)
                return 0, "KF_TRUST ok\n", False
            return 4 if self.fetch == "TRUST_UNKNOWN_ARTIFACT" else 3, "%s %s: not in the trust manifest\n" % (self.fetch, pp.ARTIFACT), False
        step = "configure" if a[0] == "./configure" else ("install" if a[:2] == ["make", "altinstall"] else "make" if a[0] == "make" else None)
        if step:
            if self.fail == step:
                pp.prefix_of(self.root).mkdir(parents=True, exist_ok=True)
                (pp.prefix_of(self.root) / ("partial_" + step)).write_text("x")
                return (124 if self.timeout else 2), "checking for gcc... no\nconfigure: error: no acceptable C compiler found in $PATH\n", bool(self.timeout)
            pp.prefix_of(self.root).mkdir(parents=True, exist_ok=True)
            (pp.prefix_of(self.root) / ("partial_" + step)).write_text("x")           # files that a step leaves behind: a failure after it must not keep them
            if step == "install":
                exe = pp.interpreter_of(self.root)
                exe.parent.mkdir(parents=True, exist_ok=True)
                exe.write_text("#!/bin/sh\n")
                exe.chmod(0o755)
            return 0, "", False
        if a[0] == str(pp.interpreter_of(self.root)) and "-c" in a:
            self.marker_at_probe = (pp.prefix_of(self.root) / pp.MARKER).exists()
            return 0, "KF_PY version=%s include=%s missing=%s\n" % (self.probe_version, self.root / pp.PRIVATE_DIR / "include" / "python3.12", self.probe_missing), False
        raise AssertionError("an unexpected process: %r" % (a,))

    def ensure(self, **kw):
        kw.setdefault("root_user", True)
        kw.setdefault("environ", {"PATH": "/usr/bin", "HOME": "/root", "LD_PRELOAD": "x", "PYTHONPATH": "/evil", "GITHUB_TOKEN": "t", "KF_TRUST_EVIDENCE_DIR": "/ev"})
        return pp.ensure(self.root, run=self.run, **kw)

    def steps(self):
        return [c for c in self.calls if c["argv"][0] in ("./configure", "make")]


class PrivatePython(unittest.TestCase):
    def test_the_recipe_is_a_private_static_altinstall_with_the_prefix_derived_from_one_constant(self):
        root = Path("/srv/apps/mlrows")
        steps = pp.recipe(pp.prefix_of(root), "/src", 6)
        self.assertEqual([s[0] for s in steps], ["configure", "make", "install"])
        self.assertEqual(steps[0][1], ["./configure", "--prefix=/srv/apps/mlrows/" + pp.PRIVATE_DIR, "--with-ensurepip=no"])
        self.assertEqual(steps[1][1], ["make", "-j6"])
        self.assertEqual(steps[2][1], ["make", "altinstall"])
        self.assertTrue(all(cwd == "/src" for _, _, cwd in steps))
        flat = [a for _, argv, _ in steps for a in argv]
        for bad in ("--enable-shared", "--enable-optimizations", "--with-lto", "install", "--prefix=/usr", "--prefix=/usr/local"):
            self.assertNotIn(bad, flat)
        self.assertEqual(pp.PRIVATE_DIR, "python312")
        self.assertEqual(str(pp.interpreter_of(root)), "/srv/apps/mlrows/python312/bin/python3.12")

    def test_the_version_is_one_constant_inside_the_range_the_pins_allow(self):
        self.assertRegex(pp.PY_VERSION, r"^3\.12\.\d+$")
        self.assertEqual(pp.ARTIFACT, "python-%s-source" % pp.PY_VERSION)
        self.assertRegex(pp.ARTIFACT, kt.NAME.pattern)
        lo, hi = tuple(PINS["python"]["min"]), tuple(PINS["python"]["max_exclusive"])
        self.assertTrue(lo <= tuple(int(x) for x in pp.PY_VERSION.split("."))[:2] < hi)

    def test_a_build_runs_the_steps_in_order_and_writes_the_marker_last(self):
        b = PrivateBox(self)
        r = b.ensure(jobs=4)
        self.assertEqual((r["status"], r["built"], r["version"]), ("ok", True, pp.PY_VERSION), r)
        self.assertEqual(b.names(), ["policy", "fetch"] + ["dpkg"] * len(BUILD_PACKAGES) + ["configure", "make", "install", "probe"])
        self.assertFalse(b.marker_at_probe)                                               # the marker is written after the probe passed, never before
        configure = b.steps()[0]
        self.assertEqual(configure["argv"], ["./configure", "--prefix=" + str(pp.prefix_of(b.root)), "--with-ensurepip=no"])
        self.assertEqual(b.steps()[1]["argv"], ["make", "-j4"])
        self.assertEqual(b.steps()[2]["argv"], ["make", "altinstall"])
        marker = json.loads((pp.prefix_of(b.root) / pp.MARKER).read_text())
        self.assertEqual((marker["version"], marker["artifact"], marker["recipe"]), (pp.PY_VERSION, pp.ARTIFACT, pp.recipe_id(pp.prefix_of(b.root))))
        self.assertRegex(marker["source_sha256"], r"^[0-9a-f]{64}$")
        self.assertFalse((b.root / (pp.PRIVATE_DIR + ".build")).exists())           # the build tree is gone, the verified tarball stays for a second try

    def test_a_marker_that_is_written_before_the_probe_would_bless_a_broken_build(self):
        b = PrivateBox(self, probe_missing="ssl")
        r = b.ensure()
        self.assertEqual(r["status"], "FAIL")
        self.assertFalse((pp.prefix_of(b.root) / pp.MARKER).exists())
        self.assertFalse(pp.prefix_of(b.root).exists())

    def test_a_second_ensure_builds_nothing(self):
        b = PrivateBox(self)
        b.ensure()
        n = len(b.calls)
        r = b.ensure()
        self.assertEqual((r["status"], r["built"], r["reason"]), ("ok", False, "cached:" + pp.PY_VERSION))
        self.assertEqual(b.names()[n:], ["probe"])                                        # only the probe that proves the cached one still runs
        self.assertEqual(len(b.steps()), 3)

    def test_a_cached_build_makes_no_network_apt_or_policy_call(self):
        b = PrivateBox(self)
        b.ensure()
        b.calls.clear()
        b.ensure()
        self.assertEqual(set(b.names()), {"probe"})

    def test_a_marker_of_another_recipe_or_version_or_a_missing_interpreter_is_a_rebuild(self):
        for edit in (lambda m: m.update(version="3.12.1"), lambda m: m.update(recipe="0" * 64), lambda m: m.update(schema="x")):
            b = PrivateBox(self)
            b.ensure()
            f = pp.prefix_of(b.root) / pp.MARKER
            (pp.prefix_of(b.root) / "stale").write_text("x")
            m = json.loads(f.read_text())
            edit(m)
            f.write_text(json.dumps(m))
            n = len(b.steps())
            self.assertTrue(b.ensure()["built"])
            self.assertEqual(len(b.steps()), n + 3)
            self.assertFalse((pp.prefix_of(b.root) / "stale").exists())               # the other recipe's tree is replaced, not built over
        b = PrivateBox(self)
        b.ensure()
        pp.interpreter_of(b.root).unlink()
        self.assertTrue(b.ensure()["built"])

    def test_a_prefix_without_a_marker_is_never_used_and_is_replaced(self):
        b = PrivateBox(self)
        exe = pp.interpreter_of(b.root)
        exe.parent.mkdir(parents=True)
        exe.write_text("#!/bin/sh\n")
        (pp.prefix_of(b.root) / "stale").write_text("x")
        self.assertIsNone(pp.usable(b.root, b.run))
        self.assertTrue(b.ensure()["built"])
        self.assertFalse((pp.prefix_of(b.root) / "stale").exists())

    def test_a_cached_python_with_a_missing_module_or_the_wrong_version_is_not_usable(self):
        b = PrivateBox(self)
        b.ensure()
        for kw in ({"probe_missing": "ssl,zlib"}, {"probe_version": "3.12.1"}):
            for k, v in kw.items():
                setattr(b, k, v)
            self.assertIsNone(pp.usable(b.root, b.run), kw)
            b.probe_missing, b.probe_version = "-", pp.PY_VERSION

    def test_a_host_that_already_has_python_312_builds_nothing_with_if_needed(self):
        b = PrivateBox(self)
        r = b.ensure(if_needed=True, running=(3, 12, 3))
        self.assertEqual((r["status"], r["built"], r["reason"]), ("ok", False, "host_python_ok:3.12.3"))
        self.assertEqual(b.calls, [])
        r = b.ensure(if_needed=True, running=(3, 10, 12))
        self.assertTrue(r["built"])
        self.assertTrue(PrivateBox(self).ensure(if_needed=False, running=(3, 12, 3))["built"])           # without the flag it always ensures

    def test_a_host_python_newer_than_the_pinned_range_is_not_good_enough_either(self):
        self.assertTrue(PrivateBox(self).ensure(if_needed=True, running=(3, 15, 0))["built"])

    def test_the_tarball_comes_through_kf_trust_and_nothing_else(self):
        b = PrivateBox(self)
        b.ensure()
        fetch = next(c for c in b.calls if len(c["argv"]) > 3 and c["argv"][2].endswith("kf_trust.py"))
        self.assertEqual(fetch["argv"][:5], [sys.executable, "-I", str(pp.KF_TRUST), "fetch", pp.ARTIFACT])
        self.assertEqual(fetch["argv"][5], "--out")
        for bad in ("--trust-lift", "--user-source", "--expect-sha256", "--operator-pin", "--mode"):
            self.assertNotIn(bad, fetch["argv"])
        self.assertEqual(pp.KF_TRUST, ROOT / "scripts/ci/kf_trust.py")
        self.assertEqual(fetch["env"].get("KF_TRUST_EVIDENCE_DIR"), "/ev")                # the helper's own variables reach the helper (and only it)
        self.assertNotIn("KF_TRUST_EVIDENCE_DIR", b.steps()[0]["env"])
        text = (ML / "private_python.py").read_text()
        for bad in ("urlopen", "urllib.request", "http.client", "requests.", "wget", "git clone", "shell=True"):
            self.assertNotIn(bad, text)

    def test_an_artifact_the_manifest_does_not_anchor_is_refused_by_name_before_apt_runs(self):
        for code in ("TRUST_UNKNOWN_ARTIFACT", "TRUST_PIN_MISMATCH", "TRUST_NO_ANCHOR"):
            b = PrivateBox(self, fetch=code, installed=[])
            r = b.ensure()
            self.assertEqual((r["status"], r["reason"], r["built"]), ("NOT_RUN", "python_source_refused:" + code, False), code)
            self.assertNotIn("apt", b.names())
            self.assertEqual(b.steps(), [])
            self.assertFalse(pp.prefix_of(b.root).exists())

    def test_a_transfer_that_fails_without_a_trust_code_is_named_by_its_exit_code(self):
        b = PrivateBox(self)
        orig = b.run
        b.run = lambda argv, **kw: (7, "connection reset\n", False) if len(argv) > 3 and argv[2].endswith("kf_trust.py") else orig(argv, **kw)
        r = pp.ensure(b.root, run=b.run, root_user=True, environ={})
        self.assertEqual((r["status"], r["reason"]), ("NOT_RUN", "python_source_refused:rc7"))

    def test_build_dependencies_are_installed_through_bash_as_root_and_rechecked(self):
        b = PrivateBox(self, installed=["build-essential", "zlib1g-dev"])
        r = b.ensure()
        self.assertTrue(r["built"], r)
        apt = next(c for c in b.calls if c["argv"][:2] == ["bash", "-c"])
        self.assertIn("apt-get install -y", apt["argv"][2])
        self.assertIn("--no-install-recommends", apt["argv"][2])
        self.assertEqual(apt["env"]["DEBIAN_FRONTEND"], "noninteractive")
        self.assertEqual(apt["argv"][4:], ["libbz2-dev", "libffi-dev", "liblzma-dev", "libsqlite3-dev", "libssl-dev"])      # only what is missing
        self.assertLess(b.names().index("apt"), b.names().index("configure"))

    def test_the_apt_call_gets_the_payload_shim_variables_and_nothing_else_does(self):
        shim = {"PATH": "/usr/bin", "BASH_ENV": "/p/hooks.sh", "KF_PAYLOAD_ROOT": "/p", "KF_APT_CACHE_ROOT": "/c", "HOME": "/root", "GITHUB_TOKEN": "t"}
        b = PrivateBox(self, installed=["build-essential"])
        b.ensure(environ=shim)
        apt = next(c for c in b.calls if c["argv"][:2] == ["bash", "-c"])
        self.assertEqual(apt["env"], {"PATH": "/usr/bin", "BASH_ENV": "/p/hooks.sh", "KF_PAYLOAD_ROOT": "/p", "KF_APT_CACHE_ROOT": "/c", "DEBIAN_FRONTEND": "noninteractive", "LANG": "C", "LC_ALL": "C"})
        for c in b.steps():
            self.assertFalse({"BASH_ENV", "KF_PAYLOAD_ROOT", "KF_APT_CACHE_ROOT", "DEBIAN_FRONTEND"} & set(c["env"]))

    def test_build_dependencies_nobody_may_install_are_a_named_not_run_and_nothing_builds(self):
        b = PrivateBox(self, installed=["build-essential"])
        r = b.ensure(root_user=False)
        self.assertEqual(r["status"], "NOT_RUN")
        self.assertTrue(r["reason"].startswith("build_deps_missing_and_not_root:libbz2-dev,"), r)
        self.assertNotIn("apt", b.names())
        self.assertEqual(b.steps(), [])
        b = PrivateBox(self, installed=["build-essential"])
        b.run_orig = b.run
        b.run = lambda argv, **kw: (0, "", False) if argv[:2] == ["bash", "-c"] else b.run_orig(argv, **kw)      # an apt that installs nothing
        r = pp.ensure(b.root, run=b.run, root_user=True, environ={})
        self.assertEqual(r["status"], "NOT_RUN")
        self.assertTrue(r["reason"].startswith("build_deps_missing:libbz2-dev,"), r)
        self.assertEqual(b.steps(), [])

    def test_an_unreadable_package_policy_is_named_and_nothing_runs(self):
        b = PrivateBox(self, policy="bad")
        r = b.ensure()
        self.assertEqual((r["status"], r["reason"]), ("NOT_RUN", "package_policy_unreadable"))
        self.assertEqual(b.names(), ["policy"])

    def test_the_build_dependencies_come_from_the_one_policy_list(self):
        b = PrivateBox(self)
        b.ensure()
        policy = next(c for c in b.calls if c["argv"][0] == "bash" and c["argv"][1] != "-c")
        self.assertEqual(policy["argv"], ["bash", str(pp.PROVISION), "--packages", pp.PACKAGE_PROFILE])
        self.assertEqual(pp.PROVISION, ROOT / "scripts/bench/provision_box.sh")

    def test_a_failed_step_is_a_named_fail_that_leaves_no_prefix_and_no_marker(self):
        for step in ("configure", "make", "install"):
            b = PrivateBox(self, fail=step)
            r = b.ensure()
            self.assertEqual(r["status"], "FAIL", step)
            self.assertTrue(r["reason"].startswith(step + "_failed:"), r["reason"])
            self.assertIn("no_acceptable_C_compiler", r["reason"])
            self.assertFalse(pp.prefix_of(b.root).exists(), step)
            self.assertFalse((b.root / (pp.PRIVATE_DIR + ".build")).exists(), step)

    def test_a_timed_out_step_says_so(self):
        b = PrivateBox(self, fail="make", timeout=True)
        self.assertTrue(b.ensure()["reason"].startswith("make_timeout:"))
        self.assertFalse(pp.prefix_of(b.root).exists())

    def test_a_built_python_is_checked_for_its_version_and_for_every_module_a_missing_header_would_drop(self):
        for kw, why in (({"probe_missing": "ssl"}, "built_python_unusable:modules_missing:ssl"), ({"probe_missing": "ctypes,lzma"}, "built_python_unusable:modules_missing:ctypes,lzma"),
                        ({"probe_version": "3.12.9"}, "built_python_unusable:wrong_version:3.12.9")):
            b = PrivateBox(self, **kw)
            r = b.ensure()
            self.assertEqual((r["status"], r["reason"]), ("FAIL", why))
            self.assertFalse((pp.prefix_of(b.root) / pp.MARKER).exists())
        self.assertEqual(sorted(pp.REQUIRED_MODULES), sorted(["ssl", "zlib", "ctypes", "bz2", "lzma", "sqlite3", "venv", "ensurepip"]))

    def test_the_probe_reads_what_a_real_interpreter_prints(self):
        t = Path(tempfile.mkdtemp())
        self.addCleanup(__import__("shutil").rmtree, t, True)
        fake = t / "python3.12"
        fake.write_text("#!/bin/sh\necho 'KF_PY version=3.12.10 include=/p/include/python3.12 missing=ssl,zlib'\n")
        fake.chmod(0o755)
        got = pp.probe(fake, pp.run_cmd)
        self.assertEqual(got, {"version": (3, 12, 10), "include": "/p/include/python3.12", "missing": ["ssl", "zlib"]})
        fake.write_text("#!/bin/sh\necho nonsense\n")
        self.assertIsNone(pp.probe(fake, pp.run_cmd))
        self.assertIsNone(pp.probe(t / "absent", pp.run_cmd))
        fake.write_text("#!/bin/sh\necho 'KF_PY version=3.12.10 include=x missing=-'\nexit 1\n")
        self.assertIsNone(pp.probe(fake, pp.run_cmd))
        compile(pp.PROBE_CODE, "probe", "exec")                       # the program the interpreter runs is itself valid Python

    def test_the_build_runs_in_an_allowlisted_environment(self):
        b = PrivateBox(self)
        b.ensure()
        for c in b.steps():
            self.assertEqual(sorted(c["env"]), ["LANG", "LC_ALL", "PATH"], c["argv"])
            self.assertEqual((c["env"]["LANG"], c["env"]["LC_ALL"]), ("C", "C"))

    def test_a_source_tarball_that_leaves_its_directory_is_refused_and_extracts_nothing_outside(self):
        for extra, why in (([("Python-%s/../../escape" % pp.PY_VERSION, tarfile.REGTYPE, "")], "member_outside_the_tree"),
                           ([("/abs/path", tarfile.REGTYPE, "")], "member_outside_the_tree"),
                           ([("Python-%s/evil" % pp.PY_VERSION, tarfile.SYMTYPE, "/etc/passwd")], "link_outside_the_tree"),
                           ([("Python-%s/evil" % pp.PY_VERSION, tarfile.SYMTYPE, "../../outside")], "link_outside_the_tree"),
                           ([("Python-%s/hard" % pp.PY_VERSION, tarfile.LNKTYPE, "../x")], "link_outside_the_tree"),
                           ([("Python-%s/dev" % pp.PY_VERSION, tarfile.CHRTYPE, "")], "special_member")):
            t = Path(tempfile.mkdtemp())
            self.addCleanup(__import__("shutil").rmtree, t, True)
            tgz = t / "p.tgz"
            source_tarball(tgz, extra=extra)
            with self.assertRaises(ValueError) as cm:
                pp.extract_source(tgz, t / "build")
            self.assertTrue(str(cm.exception).startswith(why), (extra, str(cm.exception)))
            self.assertFalse((t / "escape").exists() or (t.parent / "escape").exists())

    def test_a_tarball_with_another_top_directory_or_two_is_refused_and_a_good_one_extracts(self):
        t = Path(tempfile.mkdtemp())
        self.addCleanup(__import__("shutil").rmtree, t, True)
        source_tarball(t / "a.tgz", top="Python-3.11.0")
        with self.assertRaises(ValueError):
            pp.extract_source(t / "a.tgz", t / "b1")
        source_tarball(t / "b.tgz", extra=[("other/file", tarfile.REGTYPE, "")])
        with self.assertRaises(ValueError):
            pp.extract_source(t / "b.tgz", t / "b2")
        (t / "c.tgz").write_bytes(b"not a tarball")
        with self.assertRaises(ValueError):
            pp.extract_source(t / "c.tgz", t / "b3")
        source_tarball(t / "d.tgz")
        src = pp.extract_source(t / "d.tgz", t / "b4")
        self.assertEqual(src, t / "b4" / ("Python-" + pp.PY_VERSION))
        self.assertTrue(os.access(src / "configure", os.X_OK))

    def test_a_refused_tarball_is_a_named_not_run_and_the_build_never_starts(self):
        b = PrivateBox(self, tarball=lambda out: source_tarball(out, extra=[("Python-%s/x" % pp.PY_VERSION, tarfile.SYMTYPE, "/etc/passwd")]))
        r = b.ensure()
        self.assertEqual(r["status"], "NOT_RUN")
        self.assertTrue(r["reason"].startswith("source_refused:link_outside_the_tree"), r)
        self.assertEqual(b.steps(), [])

    def test_it_only_ever_writes_below_its_own_prefix_and_never_replaces_the_system_python(self):
        b = PrivateBox(self)
        b.ensure()
        prefix = str(pp.prefix_of(b.root))
        for c in b.steps():
            for a in c["argv"]:
                if a.startswith("--prefix="):
                    self.assertTrue(a.split("=", 1)[1].startswith(str(b.root) + os.sep), a)
        self.assertTrue(prefix.endswith("/" + pp.PRIVATE_DIR))
        self.assertTrue((b.root / "python312.dl").is_dir() and (b.root / "python312").is_dir())

    def test_ensure_refuses_hostile_or_wrong_tarballs_before_building(self):
        for make, why in ((lambda out: source_tarball(out, extra=[("Python-%s/../../escape" % pp.PY_VERSION, tarfile.REGTYPE, "")]), "member_outside_the_tree"),
                          (lambda out: source_tarball(out, top="Python-3.11.0"), "unexpected_top_directory"),
                          (lambda out: source_tarball(out, extra=[("Python-%s/dev" % pp.PY_VERSION, tarfile.CHRTYPE, "")]), "special_member"),
                          (lambda out: source_tarball(out, extra=[("Python-%s/l" % pp.PY_VERSION, tarfile.SYMTYPE, "/etc")]), "link_outside_the_tree")):
            b = PrivateBox(self, tarball=make)
            r = b.ensure()
            self.assertEqual(r["status"], "NOT_RUN", r)
            self.assertTrue(r["reason"].startswith("source_refused:" + why), r)         # OUR check names it (a newer tarfile's own filter would refuse too, an older one does not have it)
            self.assertEqual(b.steps(), [])
            self.assertFalse((b.root.parent / "escape").exists() or (b.root / "escape").exists())

    def test_main_passes_the_flags_through(self):
        seen = []
        for argv, want in ((["ensure", "--root", "/r", "--if-needed", "--jobs", "3"], ("/r", True, 3)), (["ensure", "--root", "/r"], ("/r", False, None))):
            with mock.patch.object(pp, "ensure", lambda root, **k: seen.append((root, k["if_needed"], k["jobs"])) or pp.result("ok", "x")), mock.patch("sys.stdout", io.StringIO()):
                pp.main(argv)
            self.assertEqual(seen[-1], want)

    def test_main_on_a_host_that_has_python_312_builds_nothing_and_on_an_old_one_builds(self):
        for version, builds in (((3, 12, 3), False), ((3, 10, 12), True), ((3, 15, 0), True)):
            b = PrivateBox(self)
            out = io.StringIO()
            with mock.patch.object(pp, "run_cmd", b.run), mock.patch.object(pp, "host_version", lambda v=version: v), mock.patch("os.geteuid", lambda: 0), mock.patch("sys.stdout", out):
                self.assertEqual(pp.main(["ensure", "--root", str(b.root), "--if-needed"]), 0)
            self.assertEqual(bool(b.steps()), builds, version)
            self.assertIn("PRIVATE_PYTHON status=ok reason=" + ("built:" if builds else "host_python_ok:3.12.3"), out.getvalue())

    def test_main_prints_one_line_and_exits_zero_for_a_refusal_and_three_for_a_failed_build(self):
        for result, rc in ((pp.result("ok", "cached:x", Path("/r"), False, "3.12.10"), 0), (pp.result("NOT_RUN", "python_source_refused:TRUST_UNKNOWN_ARTIFACT"), 0), (pp.result("FAIL", "make_failed:x"), 3)):
            buf = io.StringIO()
            with mock.patch.object(pp, "ensure", lambda *a, **k: result), mock.patch("sys.stdout", buf):
                self.assertEqual(pp.main(["ensure", "--root", "/r", "--if-needed"]), rc)
            self.assertEqual(len(buf.getvalue().splitlines()), 1)
            self.assertTrue(buf.getvalue().startswith("PRIVATE_PYTHON status=%s reason=" % result["status"]), buf.getvalue())

    # ---- the anchor: recorded in git through kf_trust, or refused by name; it can never be a quiet fetch
    def manifest_with(self, entry):
        doc = json.loads(kt.MANIFEST.read_text())
        doc["artifacts"].update(entry)
        return doc

    def test_the_manifest_entry_pin_entry_prints_is_valid_for_kf_trust_and_the_lint(self):
        entry = pp.pin_entry(FAKE_SHA, 27_000_000, "2026-11-20")
        doc = self.manifest_with(entry)
        self.assertEqual(kt.validate_manifest(doc), [])
        t = Path(tempfile.mkdtemp())
        self.addCleanup(__import__("shutil").rmtree, t, True)
        (t / "m.json").write_text(json.dumps(doc))
        mine = [f for f in tl.lint(ROOT, t / "m.json") if pp.ARTIFACT in f[1] or "private_python" in f[1] or "private_python" in f[2]]      # the whole tree's findings are trust_lint.py's own run
        self.assertEqual(mine, [])
        self.assertIn("scripts/apps/mlrows/private_python.py", [c for c in doc["artifacts"][pp.ARTIFACT]["consumers"]])
        self.assertTrue((ROOT / "scripts/apps/mlrows/private_python.py").is_file())
        e = entry[pp.ARTIFACT]
        a = e["anchors"][0]
        self.assertEqual((a["class"], a["scope"], a["status"], a["pin_origin"]), ("upstream-pin", "source", "MEASURED", "measured-from-upstream"))
        self.assertEqual(kt.confirmation(a), "owner-confirmed")                       # a SOURCE pin: not one of the binaries that await the owner
        self.assertEqual((e["kind"], e["routing"], e["consumers"]), ("file", "helper", ["scripts/apps/mlrows/private_python.py"]))
        self.assertEqual(e["origin"], "https://www.python.org/ftp/python/%s/Python-%s.tgz" % (pp.PY_VERSION, pp.PY_VERSION))
        self.assertTrue(a["pin_source_url"].startswith("https://www.python.org/downloads/release/python-312"))
        self.assertFalse(kt.is_ours(e["origin"]) or kt.is_ours(a["pin_source_url"]))

    def test_the_lint_refuses_the_entry_without_a_hash_or_with_ours(self):
        for edit in (lambda a: a.pop("sha256"), lambda a: a.update(sha256="xyz"), lambda a: a.update(pin_source_url="https://r2.dev/x"), lambda a: a.update(scope="binary")):
            entry = pp.pin_entry(FAKE_SHA, 1, "2026-11-20")
            edit(entry[pp.ARTIFACT]["anchors"][0])
            self.assertNotEqual(kt.validate_manifest(self.manifest_with(entry)), [])

    def test_the_committed_manifest_anchors_the_tarball_exactly_as_pin_entry_prints_it(self):
        entry = json.loads(kt.MANIFEST.read_text())["artifacts"].get(pp.ARTIFACT)
        self.assertIsNotNone(entry, "the entry is the coordinator's (MEASURED 2026-11-20): without it the build refuses by name")
        a = entry["anchors"][0]
        self.assertEqual(entry, pp.pin_entry(a["sha256"], a["size"], a["measured"])[pp.ARTIFACT])
        self.assertEqual(entry["origin"], "https://www.python.org/ftp/python/%s/Python-%s.tgz" % (pp.PY_VERSION, pp.PY_VERSION))

    def test_without_the_entry_the_real_helper_refuses_by_name_and_downloads_nothing(self):
        doc = json.loads(kt.MANIFEST.read_text())
        doc["artifacts"].pop(pp.ARTIFACT, None)
        t = Path(tempfile.mkdtemp())
        self.addCleanup(__import__("shutil").rmtree, t, True)
        (t / "m.json").write_text(json.dumps(doc))                                    # a copy of the manifest, never the committed file
        err = io.StringIO()
        with mock.patch("sys.stderr", err):
            rc = kt.main(["fetch", pp.ARTIFACT, "--out", str(t / "Python.tgz"), "--manifest", str(t / "m.json")])
        self.assertEqual(rc, 4)                                                       # TRUST_UNKNOWN_ARTIFACT: no network was needed to refuse
        self.assertIn("TRUST_UNKNOWN_ARTIFACT", err.getvalue())
        self.assertFalse((t / "Python.tgz").exists())

    def test_pin_entry_hashes_the_file_and_cross_checks_the_release_pages_md5(self):
        t = Path(tempfile.mkdtemp())
        self.addCleanup(__import__("shutil").rmtree, t, True)
        (t / "p.tgz").write_bytes(b"python bytes")
        out = io.StringIO()
        with mock.patch("sys.stdout", out):
            rc = pp.main(["pin-entry", "--tarball", str(t / "p.tgz"), "--md5", hashlib.md5(b"python bytes").hexdigest(), "--measured", "2026-11-20"])
        self.assertEqual(rc, 0)
        a = json.loads(out.getvalue())[pp.ARTIFACT]["anchors"][0]
        self.assertEqual((a["sha256"], a["size"], a["measured"], a["scope"]), (hashlib.sha256(b"python bytes").hexdigest(), 12, "2026-11-20", "source"))
        out, err = io.StringIO(), io.StringIO()
        with mock.patch("sys.stdout", out), mock.patch("sys.stderr", err):
            rc = pp.main(["pin-entry", "--tarball", str(t / "p.tgz"), "--md5", "0" * 32])
        self.assertEqual(rc, 3)
        self.assertEqual(out.getvalue(), "")
        self.assertIn("PIN_ENTRY_MD5_MISMATCH", err.getvalue())


class Interpreter(unittest.TestCase):
    """Which Python makes the venvs: the one that runs mlrows.py when it is inside the pinned range, else the private one when it is complete, else the refusal stays."""

    def setUp(self):
        self.t = Path(tempfile.mkdtemp())
        self.addCleanup(__import__("shutil").rmtree, self.t, True)
        self.calls = []

    def own(self, version=(3, 10, 12)):
        return {"exe": "/usr/bin/python3", "version": version, "include": "/usr/include/python3.10", "private": False}

    def stage_private(self, probe="KF_PY version=3.12.10 include=%s missing=-", marker=True):
        exe = pp.interpreter_of(self.t)
        exe.parent.mkdir(parents=True, exist_ok=True)
        exe.write_text("#!/bin/sh\n")
        if marker:
            (pp.prefix_of(self.t) / pp.MARKER).write_text(json.dumps({"schema": pp.MARKER_SCHEMA, "version": pp.PY_VERSION, "recipe": pp.recipe_id(pp.prefix_of(self.t))}))
        inc = pp.prefix_of(self.t) / "include" / "python3.12"
        inc.mkdir(parents=True, exist_ok=True)
        self.probe_text = probe % inc
        return inc

    def run_cmd(self, argv, env=None, timeout=60, cwd=None, stdin=None):
        self.calls.append(list(argv))
        return 0, self.probe_text, False

    def choose(self, own=None):
        with mock.patch.object(mr, "run_cmd", self.run_cmd):
            return mr.choose_interpreter(self.t, PINS, own=own or self.own())

    def test_a_host_python_inside_the_range_is_used_and_nothing_is_probed(self):
        self.stage_private()
        got = self.choose(self.own((3, 12, 3)))
        self.assertEqual((got["exe"], got["private"], got["version"]), ("/usr/bin/python3", False, (3, 12, 3)))
        self.assertEqual(self.calls, [])

    def test_the_edges_of_the_pinned_range_are_inside_it(self):
        self.stage_private()
        for version in ((3, 12, 0), (3, 14, 2)):
            got = self.choose(self.own(version))
            self.assertEqual((got["exe"], got["private"], got["version"]), ("/usr/bin/python3", False, version))
        for version in ((3, 11, 9), (3, 15, 0)):
            self.assertTrue(self.choose(self.own(version))["private"], version)             # below the range, and above it (no wheel is pinned for 3.15)

    def test_an_old_host_python_takes_the_private_one_when_it_is_complete(self):
        inc = self.stage_private()
        got = self.choose()
        self.assertEqual((got["exe"], got["private"], got["version"], got["include"]), (str(pp.interpreter_of(self.t)), True, (3, 12, 10), str(inc)))
        self.assertEqual(self.calls[0][0], str(pp.interpreter_of(self.t)))

    def test_an_old_host_python_without_a_complete_private_one_stays_and_the_refusal_stays(self):
        self.assertFalse(self.choose()["private"])                                            # nothing built
        self.stage_private(marker=False)
        self.assertFalse(self.choose()["private"])                                            # a half-made prefix: no marker
        for probe in ("KF_PY version=3.12.10 include=%s missing=ssl", "KF_PY version=3.11.9 include=%s missing=-", "KF_PY version=3.12.1 include=%s missing=-", "garbage %s"):
            self.probe_text = probe % "/x"
            (pp.prefix_of(self.t) / pp.MARKER).write_text(json.dumps({"schema": pp.MARKER_SCHEMA, "version": pp.PY_VERSION, "recipe": pp.recipe_id(pp.prefix_of(self.t))}))
            self.assertFalse(self.choose()["private"], probe)

    def private_facts(self, headers):
        inc = self.stage_private()
        if headers:
            (inc / "Python.h").write_text("")
        else:
            (inc / "Python.h").unlink(missing_ok=True)
        host_inc = self.t / "host_include"
        host_inc.mkdir(exist_ok=True)
        (host_inc / "Python.h").write_text("")                  # the running Python HAS its headers here: only the chosen interpreter's directory may decide
        with mock.patch.object(mr, "run_cmd", lambda argv, **k: (0, self.probe_text, False) if "-c" in argv else (1, "", False)), \
                mock.patch.object(mr.sysconfig, "get_paths", lambda *a, **k: {"include": str(host_inc)}):
            return mr.collect_facts(str(self.t), interp=self.choose())

    def test_facts_take_the_python_version_of_the_interpreter_that_makes_the_venvs(self):
        f = self.private_facts(True)
        self.assertEqual(f["python"], (3, 12, 10))
        self.assertEqual(lib.choose_flavour("jax", dict(f, gpus=1, name="x", cc=(8, 6), total_mib=12000, driver_cuda=(13, 0)), PINS)["status"], "ok")      # no python_too_old

    def test_facts_take_python_h_from_the_include_directory_of_that_interpreter(self):
        self.assertTrue(self.private_facts(True)["python_h"])
        self.assertFalse(self.private_facts(False)["python_h"])

    def test_a_private_python_without_its_headers_is_not_enough_for_vllm(self):
        f = dict(self.private_facts(False), gpus=1, name="x", cc=(8, 6), total_mib=12000, driver_cuda=(13, 0), gcc=True, disk_free_gb=500.0)
        with mock.patch.object(mr, "run_cmd", lambda *a, **k: (0, "", False)):
            st, why = mr.setup_row("vllm", self.t, f, PINS)
        self.assertEqual(st, lib.NOT_RUN)
        self.assertIn("no_c_compiler_or_python_headers", why)

    def end_to_end_setup(self, private):
        """`mlrows.py setup` on a host whose Python is 3.10, with real nvidia-smi output, a fake pip and a fake venv; the private Python is staged or not."""
        if private:
            (self.stage_private() / "Python.h").write_text("")
        seen = []

        def run(argv, env=None, timeout=60, cwd=None, stdin=None):
            seen.append(list(argv))
            if argv == [mr.SMI]:
                return 0, fx("nvidia_smi_head_580_rtx3060.txt"), False
            if argv[0] == mr.SMI:
                return 0, fx("query_gpu_580_rtx3060.csv"), False
            if "-c" in argv and argv[0] == str(pp.interpreter_of(self.t)):
                return 0, self.probe_text, False
            if argv[1:3] == ["-m", "venv"]:
                (Path(argv[3]) / "bin").mkdir(parents=True)
                (Path(argv[3]) / "bin" / "python").write_text("")
            return 0, "ok\n", False

        out = io.StringIO()
        free = __import__("collections").namedtuple("usage", "total used free")(10**12, 0, 5 * 10**11)
        with mock.patch.object(mr, "run_cmd", run), mock.patch.object(mr, "running_interpreter", lambda: self.own()), mock.patch.object(mr.shutil, "disk_usage", lambda p: free), \
                mock.patch("sys.stdout", out):
            rc = mr.main(["setup", "--root", str(self.t), "--rows", "jax"])
        return rc, out.getvalue().splitlines(), seen

    def test_setup_end_to_end_on_an_old_host_makes_the_venv_with_the_private_python_and_records_it(self):
        rc, lines, seen = self.end_to_end_setup(True)
        self.assertEqual(rc, 0, lines)
        venv = next(c for c in seen if c[1:3] == ["-m", "venv"])
        self.assertEqual(venv[0], str(pp.interpreter_of(self.t)))
        self.assertTrue(any(l.startswith("MLROWS_SETUP row=jax status=ok ") for l in lines), lines)
        self.assertIn("MLROWS_PYTHON exe=%s version=3.12.10 private=1" % pp.interpreter_of(self.t), lines)
        self.assertFalse([l for l in lines if l.startswith("MLROWS_HINT")])
        state = json.loads((self.t / "jax" / "setup.json").read_text())
        self.assertEqual((state["python"], state["python_private"]), ("3.12.10", True))

    def test_setup_end_to_end_on_an_old_host_without_the_private_python_refuses_by_name_and_says_how_to_get_it(self):
        rc, lines, seen = self.end_to_end_setup(False)
        self.assertEqual(rc, 0, lines)
        self.assertIn("MLROWS_SETUP row=jax status=NOT_RUN reason=python_too_old:_3.10_<_3.12", lines)
        self.assertFalse([c for c in seen if c[1:3] == ["-m", "venv"]])
        hint = next(l for l in lines if l.startswith("MLROWS_HINT "))
        tool = hint.split("hint=python3_", 1)[1].split("_ensure_--root_")[0]
        self.assertTrue(Path(tool).is_file() and Path(tool).name == "private_python.py", tool)
        self.assertTrue(hint.endswith("_ensure_--root_%s" % self.t))

    def test_setup_makes_the_venv_with_the_chosen_interpreter_and_records_it(self):
        t = Path(tempfile.mkdtemp())
        self.addCleanup(__import__("shutil").rmtree, t, True)
        seen = []

        def run(argv, env=None, timeout=60, cwd=None, stdin=None):
            seen.append(list(argv))
            if argv[1:3] == ["-m", "venv"]:
                (Path(argv[3]) / "bin").mkdir(parents=True)
                (Path(argv[3]) / "bin" / "python").write_text("")
            return 0, "", False

        interp = {"exe": "/priv/python3.12", "version": (3, 12, 10), "include": "/priv/include", "private": True}
        with mock.patch.object(mr, "run_cmd", run):
            st, why = mr.setup_row("jax", t, facts(python=(3, 12, 10)), PINS, interp=interp)
            self.assertEqual(st, "ok", why)
            self.assertEqual(seen[0][:3], ["/priv/python3.12", "-m", "venv"])
            state = json.loads((t / "jax" / "setup.json").read_text())
            self.assertEqual((state["python"], state["python_private"]), ("3.12.10", True))
            seen.clear()
            mr.setup_row("tensorrt", t, facts(), PINS)
            self.assertEqual(seen[0][0], sys.executable)                                       # without a chosen interpreter: the one that runs mlrows.py, as before
            self.assertFalse(json.loads((t / "tensorrt" / "setup.json").read_text())["python_private"])

    def setup_main(self, interp, f, extra=()):
        buf = io.StringIO()
        with mock.patch.object(mr, "choose_interpreter", lambda *a, **k: interp), mock.patch.object(mr, "collect_facts", lambda *a, **k: f), \
                mock.patch.object(mr, "setup_row", lambda row, *a, **k: (lib.NOT_RUN, "python_too_old: 3.10 < 3.12") if f["python"] < (3, 12) else ("ok", "x")), mock.patch("sys.stdout", buf):
            rc = mr.main(["setup", "--root", str(self.t), "--rows", "jax,tensorrt"] + list(extra))
        return rc, buf.getvalue().splitlines()

    def test_a_refused_old_python_names_the_way_out_without_changing_the_setup_line(self):
        rc, lines = self.setup_main(self.own(), facts(python=(3, 10, 12)))
        self.assertEqual(rc, 0)
        self.assertIn("MLROWS_SETUP row=jax status=NOT_RUN reason=python_too_old:_3.10_<_3.12", lines)
        hint = [l for l in lines if l.startswith("MLROWS_HINT ")]
        self.assertEqual(len(hint), 2)
        self.assertIn("row=jax", hint[0])
        self.assertIn("private_python.py_ensure_--root_%s" % self.t, hint[0])
        rc, lines = self.setup_main(self.own((3, 12, 3)), facts(python=(3, 12, 3)))
        self.assertFalse([l for l in lines if l.startswith("MLROWS_HINT ")])

    def test_check_and_run_each_choose_the_interpreter_from_the_root(self):
        for argv in (["check", "--root", "{r}", "--rows", "jax"], ["run", "jax", "--root", "{r}", "--out", "{r}/o"]):
            got = []
            with mock.patch.object(mr, "choose_interpreter", lambda root, pins, **k: {"exe": "/priv", "version": (3, 12, 10), "include": "/i", "private": True}), \
                    mock.patch.object(mr, "collect_facts", lambda path=".", interp=None: got.append(interp) or facts()), mock.patch.object(mr, "run_row", lambda *a, **k: (lib.NOT_RUN, "x", None, [], None, [])), \
                    mock.patch("sys.stdout", io.StringIO()):
                mr.main([a.format(r=self.t) for a in argv])
            self.assertEqual([i["exe"] for i in got], ["/priv"], argv[0])

    def test_check_run_and_setup_all_choose_the_interpreter_from_the_root(self):
        seen = []

        def chosen(root, pins, **k):
            seen.append(str(root))
            return {"exe": "/priv/python3.12", "version": (3, 12, 10), "include": "/priv/include", "private": True}

        buf = io.StringIO()
        got, handed = [], []
        with mock.patch.object(mr, "choose_interpreter", chosen), mock.patch.object(mr, "collect_facts", lambda path=".", interp=None: got.append(interp) or facts()), \
                mock.patch.object(mr, "run_row", lambda *a, **k: (lib.NOT_RUN, "x", None, [], None, [])), mock.patch.object(mr, "setup_row", lambda *a, **k: handed.append(k.get("interp")) or ("ok", "x")), \
                mock.patch("sys.stdout", buf):
            mr.main(["check", "--root", str(self.t), "--rows", "jax"])
            mr.main(["run", "jax", "--root", str(self.t), "--out", str(self.t / "o")])
            mr.main(["setup", "--root", str(self.t), "--rows", "jax"])
        self.assertEqual(seen, [str(self.t)] * 3)
        self.assertTrue(all(i and i["private"] for i in got) and len(got) == 3)
        self.assertEqual([h["exe"] for h in handed], ["/priv/python3.12"])                      # and setup hands the same interpreter to the venv maker


class PrivatePush(unittest.TestCase):
    """The venvs link their interpreter into <root>/python312: the guest must find it at the same path, so push ships it with the rows and links the host's root to the bundle copy."""

    def push(self, private=True, root_is_bundle=False):
        t = Path(tempfile.mkdtemp())
        self.addCleanup(__import__("shutil").rmtree, t, True)
        root = t / "apps" / "mlrows"
        (root / "jax/venv/bin").mkdir(parents=True)
        (root / "jax/venv/bin/python").write_text("py")
        (root / "jax/setup.json").write_text(json.dumps({"status": "ok", "python_private": private}))
        (root / "python312/bin").mkdir(parents=True)
        (root / "python312/bin/python3.12").write_text("py")
        (root / "python312" / pp.MARKER).write_text(json.dumps({"schema": pp.MARKER_SCHEMA}))
        (root / "python312.dl").mkdir()
        (root / "python312.dl/Python.tgz").write_text("x")
        sink, log = t / "sink", t / "remote.log"
        transport = ["sh", "-c", 'echo "$0" >> %s; cat > $(mktemp %s_XXXXXX)' % (log, sink)]
        buf = io.StringIO()
        dest = str(root.parent) if root_is_bundle else "/opt/apps/bundle"
        with mock.patch("sys.stdout", buf):
            rc = mr.main(["push", "--root", str(root), "--rows", "jax", "--dest", dest, "--transport"] + transport)
        names = []
        for f in t.glob("sink_*"):
            try:
                with tarfile.open(f) as tf:
                    names += tf.getnames()
            except tarfile.TarError:
                pass
        return rc, buf.getvalue(), names, (log.read_text().splitlines() if log.exists() else []), root

    def test_the_private_python_travels_with_a_venv_that_needs_it_and_the_guest_gets_the_hosts_path(self):
        rc, out, names, remote, root = self.push()
        self.assertEqual(rc, 0, out)
        self.assertTrue(any(n.endswith("python312/bin/python3.12") for n in names))
        self.assertFalse(any(".dl" in n or "Python.tgz" in n for n in names))
        self.assertIn("what=python312 rc=0", out)
        link = [r for r in remote if "ln -s" in r]
        self.assertEqual(len(link), 1, remote)
        self.assertIn("ln -sfn /opt/apps/bundle/mlrows %s" % root, link[0])
        self.assertIn("mkdir -p %s" % root.parent, link[0])

    def test_a_venv_made_with_the_system_python_ships_no_interpreter_and_no_link(self):
        rc, out, names, remote, _ = self.push(private=False)
        self.assertEqual(rc, 0, out)
        self.assertFalse(any("python312" in n for n in names))
        self.assertFalse([r for r in remote if "ln -s" in r])
        self.assertNotIn("what=python312", out)

    def test_a_staged_row_without_a_setup_state_ships_no_interpreter(self):
        t = Path(tempfile.mkdtemp())
        self.addCleanup(__import__("shutil").rmtree, t, True)
        root = t / "apps" / "mlrows"
        (root / "jax/venv/bin").mkdir(parents=True)
        (root / "python312/bin").mkdir(parents=True)
        (root / "python312" / pp.MARKER).write_text(json.dumps({"schema": pp.MARKER_SCHEMA}))
        buf = io.StringIO()
        with mock.patch("sys.stdout", buf):
            rc = mr.main(["push", "--root", str(root), "--rows", "jax", "--transport", "sh", "-c", "cat >/dev/null"])
        self.assertEqual(rc, 0)
        self.assertNotIn("what=python312", buf.getvalue())

    def test_a_root_that_is_the_bundle_copy_already_needs_no_link(self):
        rc, out, names, remote, _ = self.push(root_is_bundle=True)
        self.assertEqual(rc, 0, out)
        self.assertFalse([r for r in remote if "ln -s" in r])

    def test_a_failing_link_fails_the_push(self):
        t = Path(tempfile.mkdtemp())
        self.addCleanup(__import__("shutil").rmtree, t, True)
        root = t / "apps" / "mlrows"
        (root / "jax/venv/bin").mkdir(parents=True)
        (root / "jax/setup.json").write_text(json.dumps({"status": "ok", "python_private": True}))
        (root / "python312/bin").mkdir(parents=True)
        (root / "python312" / pp.MARKER).write_text(json.dumps({"schema": pp.MARKER_SCHEMA}))
        buf = io.StringIO()
        with mock.patch("sys.stdout", buf):
            rc = mr.main(["push", "--root", str(root), "--rows", "jax", "--transport", "sh", "-c", 'case "$0" in *ln\\ -s*) exit 5;; esac; cat >/dev/null'])
        self.assertNotEqual(rc, 0)
        self.assertIn("what=python312_link rc=5", buf.getvalue())


if __name__ == "__main__":
    unittest.main()
