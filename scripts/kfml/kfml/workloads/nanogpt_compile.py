"""nanogpt_compile: char-level Shakespeare GPT (vendored nanoGPT model.py), trained for a few hundred
steps three ways (fp32 eager, autocast eager, autocast torch.compile) from identical init and batches.
Self-checks: loss threshold, compiled-vs-eager curve tolerance, compiled-vs-eager forward agreement,
optional recorded native-host reference."""
from __future__ import annotations

from ..spec import Spec, Upstream

NANOGPT = Upstream(name="nanoGPT", repo_url="https://github.com/karpathy/nanoGPT",
                   commit="3adf61e154c3fe3fca428ad6bc3818b27a3b8291", license="MIT", role="vendored",
                   vendor_dir="nanogpt")

SPEC = Spec(
    name="nanogpt_compile", module="kfml.workloads.nanogpt_compile",
    description="small GPT, torch.compile + autocast + SDPA, vs eager and fp32 references",
    tier="mid", budget_s=1500, launch_bound=True, min_vram_gb=8.0, min_cc=(7, 5), min_torch="2.7",
    needs_triton=True, needs_compiler=True, datasets=("tinyshakespeare",), upstreams=(NANOGPT,),
    download_note="first run downloads several GB of torch/CUDA wheels (setup) plus a 1.1 MB dataset")

CFG = dict(n_layer=4, n_head=4, n_embd=128, block_size=128, dropout=0.0, bias=False)
BATCH, STEPS, LR_MAX, LR_MIN, WARMUP = 32, 400, 2e-3, 2e-4, 20
WINDOW = 25
FINAL_LOSS_MAX = 2.7  # initial ~ln(65)=4.17; 400 steps of this config reaches roughly 2.2-2.5 (UNVERIFIED on hardware)
MIN_DROP = 1.0
TOL_COMPILED_VS_EAGER = 0.15  # nats, on window means
TOL_AMP_VS_FP32 = 0.25
TOL_REFERENCE = 0.25
TOL_FORWARD_REL = 0.05


def _load_model_module(path):
    import importlib.util
    import os
    spec = importlib.util.spec_from_file_location("kfml_nanogpt_model", os.path.join(path, "model.py"))
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def _lr(step, steps):
    import math
    if step < WARMUP:
        return LR_MAX * (step + 1) / WARMUP
    p = (step - WARMUP) / max(1, steps - WARMUP)
    return LR_MIN + 0.5 * (1 + math.cos(math.pi * p)) * (LR_MAX - LR_MIN)


def _windows(curve, steps):
    idx = [i for i in (100, 200, 300, steps) if i <= steps and i >= WINDOW]
    return {str(i): sum(curve[i - WINDOW:i]) / WINDOW for i in idx}


def run(ctx):
    import time
    import torch
    import torch._dynamo  # noqa: F401
    from .. import datasets
    from . import _common as C

    dev = ctx.device
    steps = 30 if ctx.dev_cpu else STEPS
    mm = _load_model_module(ctx.upstream_paths["nanoGPT"])
    ds = datasets.fetch("tinyshakespeare", ctx.data_dir)
    text = open(ds["path"], "rb").read().decode("utf-8")
    chars = sorted(set(text))
    stoi = {c: i for i, c in enumerate(chars)}
    data = torch.tensor([stoi[c] for c in text], dtype=torch.long)
    n_train = int(0.9 * len(data))
    train_data, val_data = data[:n_train], data[n_train:]
    T = CFG["block_size"]
    dtype = C.amp_dtype(torch, ctx.cc) if not ctx.dev_cpu else torch.bfloat16
    use_scaler = dtype == torch.float16 and dev == "cuda"
    torch.backends.cuda.matmul.allow_tf32 = False
    torch.backends.cudnn.allow_tf32 = False

    def batch(gen, src):
        ix = torch.randint(len(src) - T - 1, (BATCH,), generator=gen)
        x = torch.stack([src[i:i + T] for i in ix])
        y = torch.stack([src[i + 1:i + T + 1] for i in ix])
        return x.to(dev), y.to(dev)

    def build():
        C.seed_all(torch, ctx.seed, dev)
        m = mm.GPT(mm.GPTConfig(vocab_size=len(chars), **CFG))
        return m.to(dev)

    def train(mode):
        m = build()
        opt = m.configure_optimizers(0.1, LR_MAX, (0.9, 0.99), dev)
        model = m
        amp = mode != "fp32_eager"
        t_compile = 0.0
        if mode == "amp_compiled":
            torch._dynamo.reset()
            model = torch.compile(m)
        scaler = torch.amp.GradScaler("cuda", enabled=use_scaler and amp)
        gen = torch.Generator().manual_seed(ctx.seed)
        curve, t_start = [], None
        for step in range(steps):
            for g in opt.param_groups:
                g["lr"] = _lr(step, steps)
            x, y = batch(gen, train_data)
            t_step = time.time()
            with torch.autocast(device_type=dev, dtype=dtype, enabled=amp):
                _, loss = model(x, y)
            scaler.scale(loss).backward()
            scaler.unscale_(opt)
            torch.nn.utils.clip_grad_norm_(m.parameters(), 1.0)
            scaler.step(opt)
            scaler.update()
            opt.zero_grad(set_to_none=True)
            curve.append(loss.detach().float())
            if step == 0:
                C.sync(torch, dev)
                t_compile = time.time() - t_step
                t_start = time.time()
            if ctx.expired():
                raise C.CheckFailed("soft wall-clock budget exhausted in %s at step %d" % (mode, step))
        C.sync(torch, dev)
        dt = time.time() - t_start
        vals = [float(v) for v in torch.stack(curve).cpu()]
        return m, model, vals, {"first_step_s": t_compile, "steps_per_s": (steps - 1) / max(dt, 1e-9)}

    out, curves, models = {}, {}, {}
    for mode in ("fp32_eager", "amp_eager", "amp_compiled"):
        m, model, vals, tm = train(mode)
        C.check(all(v == v and abs(v) != float("inf") for v in vals), "%s: non-finite loss" % mode)
        curves[mode] = vals
        models[mode] = (m, model)
        w = _windows(vals, steps)
        out[mode] = dict(initial=sum(vals[:10]) / 10, final=sum(vals[-WINDOW:]) / WINDOW, windows=w, **tm)
        if not ctx.dev_cpu:
            C.check(out[mode]["final"] < FINAL_LOSS_MAX, "%s final loss %.3f >= %.2f" % (mode, out[mode]["final"], FINAL_LOSS_MAX))
        C.check(out[mode]["initial"] - out[mode]["final"] > (MIN_DROP if not ctx.dev_cpu else 0.2),
                "%s: loss dropped only %.3f" % (mode, out[mode]["initial"] - out[mode]["final"]))

    def maxdiff(a, b):
        wa, wb = out[a]["windows"], out[b]["windows"]
        return max(abs(wa[k] - wb[k]) for k in wa)

    d_ce, d_af = maxdiff("amp_compiled", "amp_eager"), maxdiff("amp_eager", "fp32_eager")
    tolc, tola = (TOL_COMPILED_VS_EAGER, TOL_AMP_VS_FP32) if not ctx.dev_cpu else (1.0, 1.0)
    C.check(d_ce <= tolc, "compiled vs eager curve differ by %.3f > %.2f nats" % (d_ce, tolc))
    C.check(d_af <= tola, "amp vs fp32 curve differ by %.3f > %.2f nats" % (d_af, tola))

    # forward agreement: same trained weights, eager module vs its compiled wrapper
    m, cm = models["amp_compiled"]
    gen = torch.Generator().manual_seed(ctx.seed + 1)
    x, y = batch(gen, val_data)
    with torch.no_grad(), torch.autocast(device_type=dev, dtype=dtype):
        m.eval()
        cm.eval()
        le, _ = m(x, y)
        lc, _ = cm(x, y)
        le32 = le.float()
        lc32 = lc.float()
    fr = C.rel_err(torch, lc32, le32)
    C.check(fr <= (TOL_FORWARD_REL if not ctx.dev_cpu else 0.2), "compiled forward differs from eager: rel %.4f" % fr)

    cfgkey = dict(CFG, batch=BATCH, steps=steps, lr=[LR_MAX, LR_MIN, WARMUP], seed=ctx.seed)
    ref_cmp = None
    if ctx.reference and ctx.reference.get("config") == cfgkey:
        rw = ctx.reference["windows"]
        cw = out["amp_compiled"]["windows"]
        ref_cmp = max(abs(rw[k] - cw[k]) for k in rw if k in cw)
        C.check(ref_cmp <= TOL_REFERENCE, "differs from recorded native reference by %.3f nats" % ref_cmp)
    metrics = {"amp_dtype": str(dtype), "grad_scaler": use_scaler, "params": sum(p.numel() for p in m.parameters()),
               "modes": out, "compiled_vs_eager_max_window_diff": d_ce, "amp_vs_fp32_max_window_diff": d_af,
               "forward_rel_err": fr, "reference_compared": ref_cmp is not None, "reference_diff": ref_cmp,
               "reference_config_mismatch": bool(ctx.reference) and ref_cmp is None}
    return {"metrics": metrics, "dataset_sha256": {"tinyshakespeare": ds["sha256"]},
            "reference": {"config": cfgkey, "windows": out["amp_compiled"]["windows"]}}
