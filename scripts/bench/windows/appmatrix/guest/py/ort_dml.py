#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""ort_dml.py -- ONNX Runtime with the DirectML execution provider: a Direct3D 12 compute path on the NVIDIA
adapter. Builds a small Conv+MatMul+Relu model with onnx.helper, runs it on DmlExecutionProvider (device_id =
the DXGI index of the NVIDIA adapter from C:\\kf\\adapters.json) and on the CPU provider and compares.
Prints `CHECK dml_* ok|FAIL`, `ORT_PROVIDERS ...`, `ORT_DML_DONE`."""
import json
import sys

import numpy as np
import onnx
from onnx import TensorProto, helper
import onnxruntime as ort

print("ORT_PROVIDERS", ort.get_available_providers(), flush=True)
dev = 0
try:
    ad = json.load(open(r"C:\kf\adapters.json", encoding="utf-8-sig"))["adapters"]
    nv = [a["index"] for a in ad if a["vendor"] == 4318]
    dev = nv[0] if nv else 0
    print("ORT_DML_DEVICE index", dev, "of", [(a["index"], a["name"]) for a in ad], flush=True)
except Exception as e:
    print("adapters.json unreadable:", e, flush=True)
rng = np.random.default_rng(5)
x = helper.make_tensor_value_info("x", TensorProto.FLOAT, [4, 16, 32, 32])
w = helper.make_tensor("w", TensorProto.FLOAT, [32, 16, 3, 3], rng.standard_normal(32 * 16 * 9).astype(np.float32).tolist())
m = helper.make_tensor("m", TensorProto.FLOAT, [32, 64], rng.standard_normal(32 * 64).astype(np.float32).tolist())
nodes = [helper.make_node("Conv", ["x", "w"], ["c"], pads=[1, 1, 1, 1]), helper.make_node("Relu", ["c"], ["r"]),
         helper.make_node("GlobalAveragePool", ["r"], ["p"]), helper.make_node("Flatten", ["p"], ["f"]),
         helper.make_node("MatMul", ["f", "m"], ["y"])]
y = helper.make_tensor_value_info("y", TensorProto.FLOAT, [4, 64])
model = helper.make_model(helper.make_graph(nodes, "kf", [x], [y], [w, m]), opset_imports=[helper.make_opsetid("", 17)])
model.ir_version = 8
data = rng.standard_normal((4, 16, 32, 32)).astype(np.float32)
cpu = ort.InferenceSession(model.SerializeToString(), providers=["CPUExecutionProvider"]).run(None, {"x": data})[0]
if "DmlExecutionProvider" not in ort.get_available_providers():
    print("CHECK dml_provider FAIL DmlExecutionProvider not available"); sys.exit(1)
so = ort.SessionOptions(); so.enable_mem_pattern = False; so.execution_mode = ort.ExecutionMode.ORT_SEQUENTIAL
s = ort.InferenceSession(model.SerializeToString(), so, providers=[("DmlExecutionProvider", {"device_id": dev}), "CPUExecutionProvider"])
print("ORT_SESSION_PROVIDERS", s.get_providers(), flush=True)
print("CHECK dml_provider_active", "ok" if s.get_providers()[0] == "DmlExecutionProvider" else "FAIL", flush=True)
bad = 0
for i in range(30):
    out = s.run(None, {"x": data})[0]
    if not np.allclose(out, cpu, rtol=1e-3, atol=1e-3):
        bad += 1
print("CHECK dml_matches_cpu", "ok" if bad == 0 else f"FAIL {bad}/30 runs differ", flush=True)
print("ORT_DML_DONE", flush=True)
sys.exit(0 if bad == 0 and s.get_providers()[0] == "DmlExecutionProvider" else 1)
