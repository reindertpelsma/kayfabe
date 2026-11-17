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

    def test_a_foreign_or_plain_http_index_is_refused(self):
        h = "foo==1.0 \\\n    --hash=sha256:" + "a" * 64 + "\n"
        self.assertTrue(lib.lock_problems("--index-url https://pypi.org/simple\n--extra-index-url https://evil.example/simple\n" + h))
        self.assertTrue(lib.lock_problems("--index-url http://pypi.org/simple\n" + h))
        self.assertTrue(lib.lock_problems(h))                               # no index line: pip would use the machine's
        self.assertTrue(lib.lock_problems("--index-url https://pypi.org/simple\n" + h, require_nvidia_index=True))

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
            files[tamper] = files[tamper] + b"!"
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


class JudgeVllm(unittest.TestCase):
    def test_a_good_result_passes(self):
        st, why, checks = lib.judge_vllm(vllm_result())
        self.assertEqual((st, why), (lib.PASS, "ok"))
        self.assertTrue(all(c["ok"] for c in checks))

    def test_each_missing_property_fails_by_its_own_name(self):
        for what, kw, name in (
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
        ):
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


class JudgeTensorrt(unittest.TestCase):
    def test_a_good_result_passes_and_a_not_run_ort_ep_does_not_fail_it(self):
        self.assertEqual(lib.judge_tensorrt(trt_result())[0], lib.PASS)
        self.assertEqual(lib.judge_tensorrt(trt_result(ort_trt={"status": "NOT_RUN", "reason": "cpu wheel"}))[0], lib.PASS)

    def test_each_missing_property_fails_by_its_own_name(self):
        b, e, o = trt_result()["builder"], trt_result()["engine"], trt_result()["ort_trt"]
        for what, kw, name in (
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
        ):
            st, why, _ = lib.judge_tensorrt(trt_result(**kw))
            self.assertEqual(st, lib.FAIL, what)
            self.assertTrue(why.startswith(name + ":"), "%s: %s" % (what, why))

    def test_the_tolerance_boundary_is_a_fraction_of_the_reference(self):
        tol = lib.TRT_REL_TOL * 3.0
        ok = trt_result(engine={"ran": True, "max_abs_err": tol, "ref_max_abs": 3.0, "repeat_identical": True})
        bad = trt_result(engine={"ran": True, "max_abs_err": tol * 1.001, "ref_max_abs": 3.0, "repeat_identical": True})
        self.assertEqual(lib.judge_tensorrt(ok)[0], lib.PASS)
        self.assertEqual(lib.judge_tensorrt(bad)[0], lib.FAIL)

    def test_a_zero_reference_cannot_pass(self):
        self.assertEqual(lib.judge_tensorrt(trt_result(engine={"ran": True, "max_abs_err": 0.0, "ref_max_abs": 0.0, "repeat_identical": True}))[0], lib.FAIL)


class JudgeJax(unittest.TestCase):
    def test_a_good_result_passes(self):
        self.assertEqual(lib.judge_jax(jax_result())[0], lib.PASS)

    def test_each_missing_property_fails_by_its_own_name(self):
        ops = jax_result()["ops"]
        for what, kw, name in (
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
        ):
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
    used, total = [300, 300, 300, 6000, 9300, 9300, 400], 12288

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
        return mr.run_row(row, self.root, self.out, kw.pop("facts", facts()), kw.pop("pins", PINS), popen=self.popen(lines, kw.pop("rc", 0), kw.pop("hold", None)), **kw)

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

    def test_hardware_and_environment_preconditions_are_named_not_runs_and_nothing_runs(self):
        for f, frag in ((facts(driver_cuda=(11, 8)), "driver_cuda_below"), (facts(cc=(6, 1)), "gpu_older_than_turing"), (facts(gcc=False), "no_c_compiler"), (facts(python_h=False), "no_c_compiler"),
                        (facts(disk_free_gb=3.0), "disk_free_below_need")):
            self.calls.clear()
            st, why = self.setup("vllm", f)
            self.assertEqual(st, lib.NOT_RUN, frag)
            self.assertIn(frag, why)
            self.assertEqual(self.calls, [], frag)

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

    def test_vllm_fetches_the_model_after_pip_and_a_bad_model_fails_the_setup(self):
        get, _ = Model().fake_origin(tamper="model.safetensors") if False else (None, None)
        m = Model()
        m.tree = Model.tree
        good, _ = m.fake_origin()
        st, why = self.setup("vllm", get=good)
        self.assertEqual(st, "ok", why)
        self.assertTrue((self.t / "vllm" / "model" / "model.json").is_file())
        bad, _ = m.fake_origin(tamper="model.safetensors")
        st, why = self.setup("vllm", facts(driver_cuda=(13, 1)), get=bad) if False else (None, None)
        shutil_rm = __import__("shutil").rmtree
        shutil_rm(self.t / "vllm")
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
        matrix = (ROOT / "scripts/apps/apps_matrix.sh").read_text()
        self.assertIn('mlrows/mlrows.py" setup --root', matrix)
        self.assertIn('mlrows/mlrows.py" compare "$row"', matrix)
        self.assertIn("listall", matrix)

    def test_the_hook_and_matrix_glue_stay_short_shell(self):
        for f in ("apps_hook.sh", "apps_matrix.sh"):
            self.assertLessEqual(len((ROOT / "scripts/apps" / f).read_text().splitlines()), 140, f)


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


if __name__ == "__main__":
    unittest.main()
