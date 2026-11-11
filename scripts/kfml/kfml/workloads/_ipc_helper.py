"""Spawned-process side of the CUDA IPC check (must be importable by `spawn`)."""


def bump(t, done):
    t.add_(1.0)
    import torch
    torch.cuda.synchronize()
    done.set()
