#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""TensorRT row workload (runs in the row's venv; stdlib + numpy + onnx + onnxruntime + the TensorRT bindings). Measures, prints ONE `MLROW_JSON` line.

STATUS: LIVE, 2026-11-17 (docs/design/V3_OPEN_ML_ROWS.md). A small convolutional network is GENERATED here with onnx.helper from a seeded numpy generator (no download, no
torch), run once by ONNX Runtime on the CPU (the reference), then built by TensorRT's own builder with FP16 and a dynamic batch (the builder times candidate kernels on the
device: the autotuner of GAPS 19), the serialized engine is deserialized and run through the CUDA runtime, and its output is compared with the reference. The same model is
also run through ONNX Runtime's TensorRT execution provider when the installed onnxruntime wheel has one (the CUDA 13 flavour); the provider's own profile says which
provider ran each node, because ONNX Runtime falls back to the CPU silently. `trtexec` is not in any pip wheel (checked 2026-11-17, design doc section 3): the row records
that as NOT_RUN with the reason; its builder code path (ONNX parser, builder, FP16, optimization profile, timing) is the one this script drives through the Python API.
`--negctl`: the CPU reference only, no TensorRT.
"""
import argparse
import ctypes
import glob
import hashlib
import json
import os
import sys
import time

import numpy as np

BATCH_MIN, BATCH_OPT, BATCH_MAX = 1, 8, 16


def emit(res):
    print("MLROW_JSON " + json.dumps(res, sort_keys=True, separators=(",", ":"), default=str), flush=True)


def make_model(seed=2026):
    """-> (onnx bytes, parameter count). Conv-Relu stacks with a residual Add, a MaxPool, GlobalAveragePool and a Gemm; opset 17, IR 8, dynamic batch."""
    from onnx import TensorProto, checker, helper, numpy_helper
    rng = np.random.default_rng(seed)
    inits, nodes, params = [], [], [0]

    def tensor(name, arr):
        params[0] += int(arr.size)
        inits.append(numpy_helper.from_array(arr.astype(np.float32), name))

    def conv(src, dst, cin, cout, stride=1):
        tensor(dst + "_w", rng.standard_normal((cout, cin, 3, 3)) * np.sqrt(2.0 / (cin * 9)))
        tensor(dst + "_b", rng.standard_normal(cout) * 0.05)
        nodes.append(helper.make_node("Conv", [src, dst + "_w", dst + "_b"], [dst + "_c"], kernel_shape=[3, 3], pads=[1, 1, 1, 1], strides=[stride, stride]))
        nodes.append(helper.make_node("Relu", [dst + "_c"], [dst]))

    conv("input", "a1", 3, 32)
    conv("a1", "a2", 32, 64, 2)
    conv("a2", "a3", 64, 64)
    conv("a3", "a4", 64, 64)
    nodes.append(helper.make_node("Add", ["a2", "a4"], ["res"]))
    nodes.append(helper.make_node("MaxPool", ["res"], ["pooled"], kernel_shape=[2, 2], strides=[2, 2]))
    conv("pooled", "a5", 64, 128, 2)
    conv("a5", "a6", 128, 128)
    nodes.append(helper.make_node("GlobalAveragePool", ["a6"], ["gap"]))
    nodes.append(helper.make_node("Flatten", ["gap"], ["flat"], axis=1))
    tensor("fc_w", rng.standard_normal((10, 128)) * np.sqrt(1.0 / 128))
    tensor("fc_b", rng.standard_normal(10) * 0.1)
    nodes.append(helper.make_node("Gemm", ["flat", "fc_w", "fc_b"], ["logits"], transB=1))
    graph = helper.make_graph(nodes, "kf_trt_net", [helper.make_tensor_value_info("input", TensorProto.FLOAT, ["N", 3, 128, 128])],
                              [helper.make_tensor_value_info("logits", TensorProto.FLOAT, ["N", 10])], inits)
    model = helper.make_model(graph, opset_imports=[helper.make_opsetid("", 17)], ir_version=8)
    checker.check_model(model)
    return model.SerializeToString(), params[0]


class Cudart:
    """The five CUDA runtime calls the engine run needs, over ctypes (no CUDA Python package): the runtime comes from the pip nvidia wheel the lock installed."""

    def __init__(self):
        names = sorted({os.path.basename(p) for d in os.environ.get("LD_LIBRARY_PATH", "").split(":") if d for p in glob.glob(os.path.join(d, "libcudart.so.*"))}, reverse=True)
        last = None
        for n in names + ["libcudart.so.13", "libcudart.so.12"]:
            try:
                self.lib = ctypes.CDLL(n)
                self.name = n
                return
            except OSError as exc:
                last = exc
        raise OSError("no libcudart: %s" % last)

    def check(self, rc, what):
        if rc != 0:
            raise RuntimeError("%s failed: CUDA error %d" % (what, rc))

    def malloc(self, n):
        p = ctypes.c_void_p()
        self.check(self.lib.cudaMalloc(ctypes.byref(p), ctypes.c_size_t(n)), "cudaMalloc")
        return p

    def h2d(self, dst, arr):
        self.check(self.lib.cudaMemcpy(dst, arr.ctypes.data_as(ctypes.c_void_p), ctypes.c_size_t(arr.nbytes), 1), "cudaMemcpy H2D")

    def d2h(self, arr, src):
        self.check(self.lib.cudaMemcpy(arr.ctypes.data_as(ctypes.c_void_p), src, ctypes.c_size_t(arr.nbytes), 2), "cudaMemcpy D2H")

    def stream(self):
        s = ctypes.c_void_p()
        self.check(self.lib.cudaStreamCreate(ctypes.byref(s)), "cudaStreamCreate")
        return s

    def sync(self, s):
        self.check(self.lib.cudaStreamSynchronize(s), "cudaStreamSynchronize")

    def free(self, p):
        self.lib.cudaFree(p)


def reference(onnx_bytes, x):
    import onnxruntime as ort
    sess = ort.InferenceSession(onnx_bytes, providers=["CPUExecutionProvider"])
    return sess.run(None, {"input": x})[0], ort.__version__


def build_engine(onnx_bytes, trt, free_mib):
    """TensorRT 10 builder, FP16, dynamic batch, an EMPTY timing cache (so the builder has to time its candidate kernels), verbose logger counting tactic lines."""
    class Logger(trt.ILogger):
        def __init__(self):
            trt.ILogger.__init__(self)
            self.lines, self.tactic_lines, self.errors = 0, 0, []

        def log(self, severity, msg):
            self.lines += 1
            if "actic" in msg:
                self.tactic_lines += 1
            if severity <= trt.ILogger.Severity.ERROR and len(self.errors) < 5:
                self.errors.append(msg[:200])

    lg = Logger()
    b = {"built": False, "fp16": False}
    builder = trt.Builder(lg)
    network = builder.create_network(0)
    parser = trt.OnnxParser(network, lg)
    if not parser.parse(onnx_bytes):
        b["error"] = "onnx parse: " + "; ".join(str(parser.get_error(i)) for i in range(parser.num_errors))[:300]
        return b, None, lg
    config = builder.create_builder_config()
    config.set_flag(trt.BuilderFlag.FP16)
    b["fp16"] = bool(config.get_flag(trt.BuilderFlag.FP16))
    config.set_memory_pool_limit(trt.MemoryPoolType.WORKSPACE, int(min(2048, max(256, free_mib // 4))) << 20)
    cache = config.create_timing_cache(b"")
    config.set_timing_cache(cache, False)
    prof = builder.create_optimization_profile()
    prof.set_shape("input", (BATCH_MIN, 3, 128, 128), (BATCH_OPT, 3, 128, 128), (BATCH_MAX, 3, 128, 128))
    config.add_optimization_profile(prof)
    t = time.time()
    blob = builder.build_serialized_network(network, config)
    b["build_s"] = round(time.time() - t, 2)
    if blob is None:
        b["error"] = "build_serialized_network returned None: " + "; ".join(lg.errors)[:300]
        return b, None, lg
    data = bytes(blob)
    b.update({"built": True, "engine_bytes": len(data), "timing_cache_bytes": len(bytes(cache.serialize())), "tactic_lines": lg.tactic_lines, "log_lines": lg.lines,
              "layers_in_network": network.num_layers})
    return b, data, lg


def run_engine(engine_bytes, trt, x, ref, free_mib):
    cu = Cudart()
    r = {"ran": False, "cudart": cu.name}
    runtime = trt.Runtime(trt.Logger(trt.Logger.WARNING))
    engine = runtime.deserialize_cuda_engine(engine_bytes)
    if engine is None:
        r["error"] = "deserialize_cuda_engine returned None"
        return r
    ctx = engine.create_execution_context()
    ctx.set_input_shape("input", tuple(x.shape))
    out = np.zeros(ref.shape, dtype=np.float32)
    d_in, d_out = cu.malloc(x.nbytes), cu.malloc(out.nbytes)
    s = cu.stream()
    outs = []
    try:
        cu.h2d(d_in, np.ascontiguousarray(x))
        ctx.set_tensor_address("input", d_in.value)
        ctx.set_tensor_address("logits", d_out.value)
        for _ in range(2):
            if not ctx.execute_async_v3(s.value):
                r["error"] = "execute_async_v3 returned False"
                return r
            cu.sync(s)
            got = np.zeros(ref.shape, dtype=np.float32)
            cu.d2h(got, d_out)
            outs.append(got)
        t = time.time()
        for _ in range(50):
            ctx.execute_async_v3(s.value)
        cu.sync(s)
        r["engine_ms"] = round((time.time() - t) / 50 * 1000, 3)
    finally:
        cu.free(d_in)
        cu.free(d_out)
    r.update({"ran": True, "max_abs_err": float(np.max(np.abs(outs[0] - ref))), "ref_max_abs": float(np.max(np.abs(ref))), "repeat_identical": bool(np.array_equal(outs[0], outs[1])),
              "out_sha256": hashlib.sha256(outs[0].tobytes()).hexdigest(), "io_tensors": int(engine.num_io_tensors), "device_memory_mib": round(engine.device_memory_size_v2 / (1 << 20), 2)
              if hasattr(engine, "device_memory_size_v2") else None})
    return r


def ort_trt(onnx_bytes, x, ref, work):
    """ONNX Runtime with the TensorRT execution provider; the profile (JSON) says which provider ran each node."""
    import onnxruntime as ort
    if "TensorrtExecutionProvider" not in ort.get_available_providers():
        return {"status": "NOT_RUN", "reason": "onnxruntime_wheel_has_no_tensorrt_ep: the installed wheel lists %s (the CUDA 13 flavour installs onnxruntime-gpu; PyPI's onnxruntime-gpu 1.32 is built for CUDA 13)" % ",".join(ort.get_available_providers())}
    so = ort.SessionOptions()
    so.enable_profiling = True
    so.profile_file_prefix = os.path.join(work, "ort_profile")
    opts = {"trt_fp16_enable": True, "trt_max_workspace_size": 1 << 30, "trt_engine_cache_enable": False, "trt_timing_cache_enable": False, "trt_min_subgraph_size": 1}
    t = time.time()
    try:
        sess = ort.InferenceSession(onnx_bytes, so, providers=[("TensorrtExecutionProvider", opts), "CPUExecutionProvider"])
        out = sess.run(None, {"input": x})[0]
        out2 = sess.run(None, {"input": x})[0]
    except Exception as exc:  # noqa: BLE001 - the row reports the provider's own error
        return {"status": "FAIL", "reason": ("ort session/run raised: %s" % exc)[:300]}
    profile = sess.end_profiling()
    events = {}
    try:
        for ev in json.load(open(profile)):
            if ev.get("cat") == "Node":
                p = (ev.get("args") or {}).get("provider")
                events[p] = events.get(p, 0) + 1
    except (OSError, ValueError):
        pass
    return {"status": "PASS", "first_provider": sess.get_providers()[0], "providers": sess.get_providers(), "node_events": events, "trt_node_events": events.get("TensorrtExecutionProvider", 0),
            "cpu_node_events": events.get("CPUExecutionProvider", 0), "max_abs_err": float(np.max(np.abs(out - ref))), "ref_max_abs": float(np.max(np.abs(ref))),
            "repeat_identical": bool(np.array_equal(out, out2)), "secs": round(time.time() - t, 2), "ort": ort.__version__}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--work", required=True)
    ap.add_argument("--negctl", action="store_true")
    a = ap.parse_args()
    os.makedirs(a.work, exist_ok=True)
    t0 = time.time()
    onnx_bytes, nparams = make_model()
    rng = np.random.default_rng(7)
    x = rng.standard_normal((BATCH_OPT, 3, 128, 128), dtype=np.float32)
    ref, ort_ver = reference(onnx_bytes, x)
    res = {"row": "tensorrt", "onnx_sha256": hashlib.sha256(onnx_bytes).hexdigest(), "onnx_bytes": len(onnx_bytes), "onnx_params": nparams, "ref_sha256": hashlib.sha256(ref.tobytes()).hexdigest(),
           "ref_max_abs": float(np.max(np.abs(ref))), "ort_cpu": ort_ver, "batch": [BATCH_MIN, BATCH_OPT, BATCH_MAX]}
    res["trtexec"] = {"status": "NOT_RUN", "reason": "trtexec_is_not_in_the_pypi_wheels"}
    if a.negctl:
        res.update({"platform": "cpu", "builder": {"built": False, "fp16": False}, "engine": {"ran": False}, "ort_trt": {"status": "NOT_RUN", "reason": "negctl: the CPU form of the row"}, "wall_s": round(time.time() - t0, 1)})
        emit(res)
        return
    import tensorrt_bindings as trt
    res["trt"] = trt.__version__
    free_mib = int(os.environ.get("KF_TRT_FREE_MIB", "8192"))
    builder, engine_bytes, lg = build_engine(onnx_bytes, trt, free_mib)
    res["builder"] = builder
    res["engine"] = run_engine(engine_bytes, trt, x, ref, free_mib) if engine_bytes else {"ran": False, "error": "no engine"}
    res["ort_trt"] = ort_trt(onnx_bytes, x, ref, a.work)
    res["wall_s"] = round(time.time() - t0, 1)
    emit(res)


if __name__ == "__main__":
    main()
