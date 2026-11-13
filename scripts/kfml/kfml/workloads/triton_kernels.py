"""triton_kernels: tutorial-style Triton kernels (vector add, fused softmax, layernorm fwd, tiled matmul
with autotune) vs torch references, on shapes that include non-multiples of the block sizes."""
from __future__ import annotations

from ..spec import Spec, Upstream

TRITON_TUTORIALS = Upstream(
    name="triton-tutorials", repo_url="https://github.com/triton-lang/triton",
    commit="d654e0f2d91f07496454e0fcbec2a9b97df37d47",  # tag v3.3.1 (the pinned triton version)
    license="MIT", role="derived")

SPEC = Spec(
    name="triton_kernels", module="kfml.workloads.triton_kernels",
    description="triton add/softmax/layernorm/matmul(autotune) vs torch, odd shapes",
    tier="mid", budget_s=900, launch_bound=False, min_vram_gb=8.0, min_cc=(7, 5), min_torch="2.7",
    needs_triton=True, needs_compiler=True, upstreams=(TRITON_TUTORIALS,),
    download_note="first run downloads several GB of torch/CUDA wheels (setup)")

ADD_SIZES = [1, 7, 1000, 98432, 2**20 + 3]
SOFTMAX_SHAPES = [(1, 1), (7, 33), (130, 781), (1823, 1000), (257, 4097), (64, 8191)]
LN_SHAPES = [(1, 5), (9, 33), (130, 781), (513, 1024), (257, 4093)]
MATMUL_SHAPES = [(17, 33, 19), (127, 129, 65), (512, 512, 512), (1000, 777, 333), (2049, 513, 1025)]


def _np2(n):
    return 1 << max(0, (int(n) - 1).bit_length())


def run(ctx):
    import time
    import torch
    from . import _common as C
    from . import _triton_kernels as K
    import triton

    dev = "cuda"
    g = torch.Generator(device="cpu").manual_seed(ctx.seed)
    torch.backends.cuda.matmul.allow_tf32 = False
    metrics, checked = {}, 0

    def rnd(*shape, dtype=torch.float32, scale=1.0):
        return (torch.randn(*shape, generator=g) * scale).to(dtype).to(dev)

    # scaled large vector add: up to ~6% of free VRAM per operand, capped
    big = int(min(2**27, max(2**22, (ctx.vram_free_gb or 8) * 2**30 / 4 / 16)))
    add_sizes = ADD_SIZES + [big + 5]
    worst = 0.0
    for n in add_sizes:
        x, y = rnd(n), rnd(n)
        out = torch.empty_like(x)
        K.add_kernel[(triton.cdiv(n, 1024),)](x, y, out, n, BLOCK=1024)
        torch.testing.assert_close(out, x + y, rtol=0, atol=0)  # fp32 add is exact
        checked += 1
    metrics["add_sizes"] = add_sizes

    for rows, cols in SOFTMAX_SHAPES:
        x = rnd(rows, cols, scale=3.0)
        out = torch.empty_like(x)
        blk = _np2(cols)
        K.softmax_kernel[(rows,)](out, x, x.stride(0), out.stride(0), cols, BLOCK=blk, num_warps=min(16, max(1, blk // 256)))
        ref = torch.softmax(x, dim=1)
        err = float((out - ref).abs().max())
        worst = max(worst, err)
        C.check(err < 1e-5, "softmax %s max abs err %.3g" % ((rows, cols), err))
        C.check(float((out.sum(1) - 1).abs().max()) < 1e-4, "softmax rows do not sum to 1 for %s" % ((rows, cols),))
        checked += 1
    metrics["softmax_max_abs_err"] = worst

    ln_worst = 0.0
    for rows, cols in LN_SHAPES:
        for dt, tol in ((torch.float32, 2e-4), (torch.float16, 3e-3)):
            x = rnd(rows, cols, dtype=dt, scale=2.0)
            w = rnd(cols, dtype=dt)
            b = rnd(cols, dtype=dt)
            y = torch.empty_like(x)
            mean = torch.empty(rows, dtype=torch.float32, device=dev)
            rstd = torch.empty(rows, dtype=torch.float32, device=dev)
            blk = _np2(cols)
            K.layernorm_fwd_kernel[(rows,)](x, y, w, b, mean, rstd, x.stride(0), cols, 1e-5, BLOCK=blk,
                                            num_warps=min(16, max(1, blk // 256)))
            ref = torch.nn.functional.layer_norm(x.float(), (cols,), w.float(), b.float(), 1e-5)
            err = float((y.float() - ref).abs().max())
            ln_worst = max(ln_worst, err)
            C.check(err < tol * max(1.0, float(ref.abs().max())), "layernorm %s %s max abs err %.3g" % ((rows, cols), dt, err))
            rm = x.float().mean(1)
            C.check(float((mean - rm).abs().max()) < 1e-3 * max(1.0, float(rm.abs().max())), "layernorm mean mismatch %s" % ((rows, cols),))
            checked += 1
    metrics["layernorm_max_abs_err"] = ln_worst

    mm_worst = 0.0
    scale = ctx.vram_free_gb and ctx.vram_free_gb >= 16
    shapes = MATMUL_SHAPES + ([(4096, 4096, 2048)] if scale else [])
    t_mm = {}
    for M, N, Kd in shapes:
        a = rnd(M, Kd, dtype=torch.float16, scale=0.5)
        b = rnd(Kd, N, dtype=torch.float16, scale=0.5)
        c = torch.empty((M, N), dtype=torch.float32, device=dev)
        grid = lambda meta: (triton.cdiv(M, meta["BLOCK_M"]) * triton.cdiv(N, meta["BLOCK_N"]),)  # noqa: E731
        K.matmul_kernel[grid](a, b, c, M, N, Kd, a.stride(0), a.stride(1), b.stride(0), b.stride(1),
                              c.stride(0), c.stride(1))
        ref = a.float() @ b.float()
        e = float((c - ref).abs().max() / (ref.abs().max() + 1e-6))
        mm_worst = max(mm_worst, e)
        C.check(e < 2e-3, "matmul %s normalized max err %.3g" % ((M, N, Kd), e))
        checked += 1
        t_mm["%dx%dx%d" % (M, N, Kd)] = dict(rel_err=e, best_config=str(K.matmul_kernel.best_config))
    metrics["matmul"] = t_mm
    metrics["matmul_worst_rel_err"] = mm_worst
    # determinism: same inputs twice give the same bytes
    a = rnd(300, 200, dtype=torch.float16, scale=0.5)
    b = rnd(200, 100, dtype=torch.float16, scale=0.5)
    outs = []
    for _ in range(2):
        c = torch.empty((300, 100), dtype=torch.float32, device=dev)
        grid = lambda meta: (triton.cdiv(300, meta["BLOCK_M"]) * triton.cdiv(100, meta["BLOCK_N"]),)  # noqa: E731
        K.matmul_kernel[grid](a, b, c, 300, 100, 200, a.stride(0), a.stride(1), b.stride(0), b.stride(1), c.stride(0), c.stride(1))
        outs.append(c)
    C.check(torch.equal(outs[0], outs[1]), "triton matmul is not deterministic across two launches")
    metrics["cases_checked"] = checked + 1
    metrics["triton"] = triton.__version__
    return {"metrics": metrics}
