"""stress_alloc: allocator and runtime stress. Allocate/free to ~70% of free VRAM in varied sizes with content
verification, fragmentation churn, many streams and events with cross-stream dependencies, CUDA graph
capture + replay of a small training step vs eager, and pinned async copies round trip."""
from __future__ import annotations

from ..spec import Spec

SPEC = Spec(
    name="stress_alloc", module="kfml.workloads.stress_alloc",
    description="alloc to 70% VRAM, streams/events, CUDA graphs, async copies", tier="mid", budget_s=900,
    launch_bound=False, min_vram_gb=8.0, min_cc=(7, 5), min_torch="2.7",
    download_note="first run downloads several GB of torch/CUDA wheels (setup)")

TARGET_FRACTION = 0.70
MAX_TOTAL_GB = 48.0  # cap so the test stays inside its budget on very large cards


def _fill_check(torch, C, bufs):
    for i, t in bufs:
        v = float(i % 251)
        C.check(bool((t == v).all()), "buffer %d content corrupted (expected %s)" % (i, v))


def run(ctx):
    import random
    import time
    import torch
    from . import _common as C

    dev = "cuda"
    rng = random.Random(ctx.seed)
    metrics = {}
    torch.cuda.empty_cache()
    free0, total = torch.cuda.mem_get_info()
    target = int(min(free0 * TARGET_FRACTION, MAX_TOTAL_GB * 2**30))
    metrics.update(free_start_gb=free0 / 2**30, target_gb=target / 2**30)

    # 1. varied-size allocate/fill to target, verify, free half in random order, refill, verify, free all
    bufs, used, i = [], 0, 0
    while used < target and not ctx.expired():
        sz = int(min(target - used, 2 ** rng.uniform(16, 29)))  # 64 KiB .. 512 MiB
        n = max(1, sz // 4)
        t = torch.empty(n, dtype=torch.float32, device=dev)
        t.fill_(float(i % 251))
        bufs.append((i, t))
        used += n * 4
        i += 1
    torch.cuda.synchronize()
    _fill_check(torch, C, bufs)
    metrics.update(buffers=len(bufs), allocated_gb=used / 2**30)
    rng.shuffle(bufs)
    keep, drop = bufs[::2], bufs[1::2]
    bufs.clear()
    drop.clear()
    torch.cuda.synchronize()
    j = 10_000
    refill = []
    for _ in range(len(keep)):
        n = max(1, int(2 ** rng.uniform(14, 26)) // 4)
        t = torch.empty(n, dtype=torch.float32, device=dev)
        t.fill_(float(j % 251))
        refill.append((j, t))
        j += 1
    torch.cuda.synchronize()
    _fill_check(torch, C, keep)
    _fill_check(torch, C, refill)
    keep.clear()
    refill.clear()
    torch.cuda.synchronize()
    C.check(torch.cuda.memory_allocated() < 64 * 2**20, "memory_allocated not released: %d" % torch.cuda.memory_allocated())
    torch.cuda.empty_cache()
    free1, _ = torch.cuda.mem_get_info()
    C.check(free1 >= free0 - 512 * 2**20, "free VRAM after release %.2f GB < start %.2f GB - 0.5" % (free1 / 2**30, free0 / 2**30))
    metrics["free_end_gb"] = free1 / 2**30

    # 2. fragmentation churn
    live = {}
    for it in range(3000):
        k = rng.randrange(64)
        if k in live and rng.random() < 0.6:
            del live[k]
        else:
            live[k] = torch.full((rng.randrange(1, 1 << 20),), float(k), device=dev)
    for k, t in live.items():
        C.check(bool((t == float(k)).all()), "churn buffer corrupted")
    live.clear()

    # 3. many streams + events: independent matmul chains, plus a cross-stream dependency chain
    torch.backends.cuda.matmul.allow_tf32 = False
    ns = 16
    g = torch.Generator(device="cpu").manual_seed(ctx.seed)
    a = [torch.randn(256, 256, generator=g).to(dev) * 0.05 for _ in range(ns)]
    ref = []
    for k in range(ns):
        x = a[k]
        for _ in range(8):
            x = x @ a[(k + 1) % ns] + 0.01
        ref.append(x)
    torch.cuda.synchronize()
    streams = [torch.cuda.Stream() for _ in range(ns)]
    outs = [None] * ns
    events = [torch.cuda.Event() for _ in range(ns)]
    for k in range(ns):
        with torch.cuda.stream(streams[k]):
            x = a[k]
            for _ in range(8):
                x = x @ a[(k + 1) % ns] + 0.01
            outs[k] = x
            events[k].record(streams[k])
    chain = torch.zeros(1, device=dev)
    for k in range(ns):  # stream k+1 waits for stream k via events
        s = streams[(k + 1) % ns]
        s.wait_event(events[k])
        with torch.cuda.stream(s):
            chain = chain + outs[k].sum().reshape(1)
            outs[k].record_stream(s)
    torch.cuda.synchronize()
    for k in range(ns):
        torch.testing.assert_close(outs[k], ref[k], rtol=1e-4, atol=1e-5)
    C.check(torch.isfinite(chain).all().item(), "stream chain produced non-finite value")
    metrics["streams"] = ns

    # 4. CUDA graph capture + replay of a small training step vs eager
    def make():
        C.seed_all(torch, ctx.seed, dev)
        m = torch.nn.Sequential(torch.nn.Linear(64, 128), torch.nn.ReLU(), torch.nn.Linear(128, 10)).to(dev)
        return m, torch.optim.SGD(m.parameters(), lr=0.05, momentum=0.9)

    xs = torch.randn(32, 64, generator=g).to(dev)
    ys = torch.randint(0, 10, (32,), generator=g).to(dev)
    m_e, o_e = make()
    for _ in range(40):
        o_e.zero_grad(set_to_none=True)
        torch.nn.functional.cross_entropy(m_e(xs), ys).backward()
        o_e.step()
    m_g, o_g = make()
    m0, _ = make()  # pristine init to restore after warmup/capture
    sx, sy = xs.clone(), ys.clone()
    side = torch.cuda.Stream()
    side.wait_stream(torch.cuda.current_stream())
    with torch.cuda.stream(side):  # warmup so optimizer state exists before capture
        for _ in range(3):
            o_g.zero_grad(set_to_none=True)
            torch.nn.functional.cross_entropy(m_g(sx), sy).backward()
            o_g.step()
    torch.cuda.current_stream().wait_stream(side)
    torch.cuda.synchronize()
    graph = torch.cuda.CUDAGraph()
    o_g.zero_grad(set_to_none=True)
    with torch.cuda.graph(graph):
        lg = torch.nn.functional.cross_entropy(m_g(sx), sy)
        lg.backward()
        o_g.step()
    # capture does not execute: restore the pristine init and zero the momentum buffers
    # (a zero buffer makes step 1 identical to eager's first step), then replay 40 steps
    with torch.no_grad():
        for p, q in zip(m_g.parameters(), m0.parameters()):
            p.copy_(q)
        for st in o_g.state.values():
            for v in st.values():
                if torch.is_tensor(v):
                    v.zero_()
    for _ in range(40):
        graph.replay()
    torch.cuda.synchronize()
    for p, q in zip(m_g.parameters(), m_e.parameters()):
        torch.testing.assert_close(p, q, rtol=1e-3, atol=1e-4)
    metrics["graph_replays"] = 40

    # 5. pinned async copies round trip on a copy stream with events
    n = 16 * 2**20
    host = torch.empty(n, dtype=torch.float32).pin_memory()
    host.copy_(torch.randn(n, generator=g))
    back = torch.empty_like(host).pin_memory()
    cs = torch.cuda.Stream()
    for _ in range(4):
        with torch.cuda.stream(cs):
            d = host.to(dev, non_blocking=True)
            d2 = d * 2.0
            back.copy_(d2, non_blocking=True)
            ev = torch.cuda.Event()
            ev.record(cs)
        ev.synchronize()
        C.check(torch.equal(back, host * 2.0), "async pinned round trip mismatch")
    metrics["async_copy_mb"] = n * 4 / 2**20
    return {"metrics": metrics}
