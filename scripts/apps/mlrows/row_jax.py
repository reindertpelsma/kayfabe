#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""JAX / XLA GPU row workload (runs in the row's venv; stdlib + numpy + jax). Measures, prints ONE `MLROW_JSON` line, decides nothing (mlrows_lib.judge_jax does).

STATUS: LIVE, 2026-11-17 (docs/design/V3_OPEN_ML_ROWS.md). What it asks of the driver: the default memory preallocation (XLA reserves 75 percent of the memory it is
told the device has, at the first operation), the XLA GEMM/convolution autotuner (candidate kernels timed on the device, results dumped to a file), jit of a matmul, a
convolution and a lax.scan, each compared with the same function on JAX's own CPU backend. `--negctl` is not a flag of this script: the runner sets JAX_PLATFORMS=cpu.
Nothing here sets XLA_PYTHON_CLIENT_PREALLOCATE or the memory fraction: the defaults are the experiment.
"""
import json
import os
import sys
import time

AUTOTUNE_FILE = os.path.join(os.getcwd(), "xla_autotune_results.textproto")
# the dump flag is part of the experiment: the autotuner's results are what proves candidate kernels were timed on the device
os.environ["XLA_FLAGS"] = (os.environ.get("XLA_FLAGS", "") + " --xla_gpu_dump_autotune_results_to=" + AUTOTUNE_FILE).strip()

import numpy as np  # noqa: E402


def emit(res):
    print("MLROW_JSON " + json.dumps(res, sort_keys=True, separators=(",", ":")), flush=True)


def main():
    t0 = time.time()
    import jax
    import jax.numpy as jnp
    from jax import lax
    import jaxlib
    res = {"row": "jax", "jax": jax.__version__, "jaxlib": jaxlib.__version__, "env_preallocate": os.environ.get("XLA_PYTHON_CLIENT_PREALLOCATE"),
           "env_mem_fraction": os.environ.get("XLA_PYTHON_CLIENT_MEM_FRACTION"), "jax_platforms": os.environ.get("JAX_PLATFORMS")}
    devs = jax.devices()
    res["default_backend"] = jax.default_backend()
    res["devices"] = [{"id": d.id, "platform": d.platform, "kind": getattr(d, "device_kind", None)} for d in devs]
    res["platform"] = devs[0].platform
    cpu = jax.devices("cpu")[0]
    rng = np.random.default_rng(1234)

    def both(name, f, args, precision):
        """f on the default device (the GPU) and on the CPU backend, same numpy inputs; numbers only."""
        t = time.time()
        gpu_args = [jax.device_put(a) for a in args]
        out = jax.jit(f)(*gpu_args)
        out.block_until_ready()
        first = time.time() - t
        t = time.time()
        for _ in range(5):
            out = jax.jit(f)(*gpu_args)
        out.block_until_ready()
        steady = (time.time() - t) / 5
        ref = np.asarray(jax.jit(f)(*[jax.device_put(a, cpu) for a in args]))
        got = np.asarray(out)
        return {"name": name, "precision": precision, "max_abs_err": float(np.max(np.abs(got - ref))), "ref_max_abs": float(np.max(np.abs(ref))),
                "on_gpu": list(out.devices())[0].platform in ("gpu", "cuda"), "first_call_s": round(first, 3), "steady_s": round(steady, 5), "shape": list(got.shape)}

    a = rng.standard_normal((2048, 2048), dtype=np.float32)
    b = rng.standard_normal((2048, 2048), dtype=np.float32)
    x = rng.standard_normal((8, 16, 64, 64), dtype=np.float32)
    w = (rng.standard_normal((32, 16, 3, 3), dtype=np.float32) * 0.1).astype(np.float32)
    wh = (rng.standard_normal((64, 64), dtype=np.float32) * (0.7 / 8.0)).astype(np.float32)
    xs = rng.standard_normal((256, 64), dtype=np.float32)

    def mm_hi(p, q):
        return jnp.matmul(p, q, precision=lax.Precision.HIGHEST)

    def mm_def(p, q):
        return jnp.matmul(p, q)

    def conv(p, k):
        return lax.conv_general_dilated(p, k, (1, 1), "SAME", dimension_numbers=("NCHW", "OIHW", "NCHW"), precision=lax.Precision.HIGHEST)

    def scan(wm, seq):
        def step(h, xt):
            h = jnp.tanh(jnp.matmul(h, wm, precision=lax.Precision.HIGHEST) + xt)
            return h, h
        _, hs = lax.scan(step, jnp.zeros((64,), jnp.float32), seq)
        return hs

    res["ops"] = [both("matmul_f32_highest", mm_hi, [a, b], "highest"), both("matmul_default", mm_def, [a, b], "default"), both("conv", conv, [x, w], "highest"),
                  both("scan", scan, [wh, xs], "highest")]
    st = devs[0].memory_stats() if hasattr(devs[0], "memory_stats") else None
    st = st or {}
    mib = 1 << 20
    res["bytes_limit_mib"] = round(st["bytes_limit"] / mib, 1) if "bytes_limit" in st else None
    res["peak_bytes_in_use_mib"] = round(st["peak_bytes_in_use"] / mib, 2) if "peak_bytes_in_use" in st else None
    res["pool_bytes_mib"] = round(st["pool_bytes"] / mib, 1) if "pool_bytes" in st else None
    # the autotuner's dump is written when the compilations are done; the file is small text
    try:
        res["autotune_bytes"] = os.path.getsize(AUTOTUNE_FILE)
        with open(AUTOTUNE_FILE, errors="replace") as f:
            res["autotune_entries"] = sum(1 for line in f if line.startswith("results {") or line.strip().startswith("results {"))
    except OSError:
        res["autotune_bytes"], res["autotune_entries"] = 0, 0
    # nvidia-smi's total comes from the runner (the sampler); the row prints what XLA itself says
    res["wall_s"] = round(time.time() - t0, 1)
    emit(res)


if __name__ == "__main__":
    main()
