"""Child probe: prints one JSON line with torch/CUDA facts. Imports torch (the only place besides tests)."""
import importlib.util
import json
import sys


def main():
    d = {"modules": {m: importlib.util.find_spec(m) is not None for m in ("triton", "torchvision", "numpy")}}
    try:
        import torch
    except Exception as e:  # noqa
        d["error"] = "import torch failed: %s" % e
        print(json.dumps(d))
        return
    d["torch"] = torch.__version__
    d["cuda"] = torch.version.cuda
    try:
        d["cudnn"] = str(torch.backends.cudnn.version())
    except Exception:
        d["cudnn"] = None
    try:
        import triton
        d["triton"] = triton.__version__
    except Exception:
        d["triton"] = None
    try:
        torch.cuda.init()
        ok = torch.cuda.is_available()
    except Exception as e:  # noqa
        ok = False
        d["cuda_error"] = str(e)
    d["cuda_available"] = ok
    if ok:
        p = torch.cuda.get_device_properties(0)
        free, total = torch.cuda.mem_get_info(0)
        d.update(gpu_name=p.name, cc=[p.major, p.minor], vram_total_gb=round(total / 2**30, 2),
                 vram_free_gb=round(free / 2**30, 2))
    elif "cuda_error" not in d:
        d["cuda_error"] = "torch.cuda.is_available() is False"
    print(json.dumps(d))


if __name__ == "__main__":
    main()
