"""managed_oversub (not part of the gate): cudaMallocManaged oversubscription through a load_inline tensor.
Known unsupported on a kayfabe guest today: with KF_ML_GUEST=1 it reports NOT_RUN 'managed-memory paging
not supported' without running; KF_ML_PROBE_TRY=1 attempts it anyway (failure -> NOT_RUN, success -> PASS).
On a native host it must PASS."""
from __future__ import annotations

from ..spec import Spec

REASON = "managed-memory paging not supported"


def _precheck(env, environ):
    if environ.get("KF_ML_GUEST") == "1" and environ.get("KF_ML_PROBE_TRY") != "1":
        return REASON
    return None


SPEC = Spec(
    name="managed_oversub", module="kfml.workloads.managed_oversub",
    description="cudaMallocManaged oversubscription probe (known unsupported in the guest)", tier="large",
    budget_s=900, launch_bound=False, min_vram_gb=8.0, min_cc=(7, 5), min_torch="2.7", needs_nvcc=True,
    needs_compiler=True, gate=False, precheck=_precheck,
    download_note="needs nvcc matching torch.version.cuda; several GB of CUDA wheels on first setup")

CPP = "torch::Tensor managed_empty(int64_t numel);"
CUDA = r"""
#include <cuda_runtime.h>
torch::Tensor managed_empty(int64_t numel) {
  void* p = nullptr;
  cudaError_t e = cudaMallocManaged(&p, numel * sizeof(float));
  TORCH_CHECK(e == cudaSuccess, "cudaMallocManaged failed: ", cudaGetErrorString(e));
  auto deleter = [](void* q) { cudaFree(q); };
  int dev = 0; cudaGetDevice(&dev);
  return torch::from_blob(p, {numel}, deleter, torch::dtype(torch::kFloat32).device(torch::kCUDA, dev));
}
"""


def _host_avail_gb():
    try:
        for line in open("/proc/meminfo"):
            if line.startswith("MemAvailable:"):
                return int(line.split()[1]) / 2**20
    except Exception:
        pass
    return 0.0


def run(ctx):
    import os
    import torch
    from torch.utils import cpp_extension
    from ..errors import NotRun
    from . import _common as C

    attempt_guest = ctx.guest
    try:
        cc = "%d.%d" % tuple(ctx.cc)
        os.environ["TORCH_CUDA_ARCH_LIST"] = cc + "+PTX"
        build = os.path.join(os.environ.get("TORCH_EXTENSIONS_DIR", ctx.data_dir), "kfml_managed_" + cc.replace(".", "_"))
        os.makedirs(build, exist_ok=True)
        ext = cpp_extension.load_inline(name="kfml_managed_" + cc.replace(".", "_"), cpp_sources=CPP, cuda_sources=CUDA,
                                        functions=["managed_empty"], build_directory=build)
        total = torch.cuda.get_device_properties(0).total_memory
        want = int(total * 1.25)
        if want / 2**30 > _host_avail_gb() * 0.6:
            raise NotRun("host RAM too small to oversubscribe VRAM by 25%")
        chunk = 256 * 2**20 // 4
        tensors, got = [], 0
        while got < want:
            t = ext.managed_empty(chunk)
            t.fill_(float(len(tensors) % 97))
            tensors.append(t)
            got += chunk * 4
        torch.cuda.synchronize()
        for k, t in enumerate(tensors):  # touch everything again after the whole set was resident once
            C.check(bool((t == float(k % 97)).all()), "managed chunk %d corrupted" % k)
        return {"metrics": {"oversub_gb": got / 2**30, "vram_gb": total / 2**30, "chunks": len(tensors)}}
    except NotRun:
        raise
    except Exception as e:  # noqa
        if attempt_guest:
            raise NotRun("%s (%s: %s)" % (REASON, type(e).__name__, str(e)[:200]))
        raise
