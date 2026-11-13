import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from kfml.env import Env  # noqa: E402


def fake_env(**kw):
    d = dict(python="3.12.0", gpu_visible=True, gpu_name="Fake RTX", cc=(8, 9), vram_total_gb=24.0,
             vram_free_gb=23.0, driver="999.1", torch="2.7.1+cu128", cuda="12.8", cudnn="90701",
             triton="3.3.1", cuda_available=True, nvcc="12.8", nvcc_path="/x/nvcc", cxx=True, python_h=True,
             disk_free_gb=100.0, lock_sha256="ab" * 32)
    d.update(kw)
    return Env(**d)
