#!/usr/bin/env python3
# torch_correct.py — NUMERICAL correctness of PyTorch on the GPU, checked against the CPU on the
# same machine (ai_bench.py's checks are only isfinite()). Seeded, so the GPU digests are also
# comparable host vs guest. Emits "CHECK <key> ok|FAIL" and "DIGEST <key> <hex>".
# ★ 2026-10-03 (release §I, row torch_expseg): with PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True
# the caching allocator backs its segments with the CUDA virtual-memory API (cuMemCreate/cuMemMap)
# and grows/shrinks them by mapping and unmapping pages at reserved VAs. After the usual checks the
# script then GROWS a segment, frees part of it, empty_cache()s (unmap), reallocates (remap), writes
# by kernel, verifies every element on the CPU, prints a DIGEST, and prints
# "CHECK expandable_segments ok|FAIL" LAST — the row's regex, so a run that stops early cannot pass.
import hashlib, os, sys, torch, torch.nn as nn

def chk(k, ok, extra=""):
    print(f"CHECK {k} {'ok' if ok else 'FAIL'} {extra}", flush=True)

def dig(k, t):
    h = hashlib.sha256(t.detach().float().cpu().numpy().round(3).tobytes()).hexdigest()[:16]
    print(f"DIGEST {k} {h}", flush=True)

assert torch.cuda.is_available(), "no CUDA"
print("[tc] device", torch.cuda.get_device_name(0), "torch", torch.__version__, file=sys.stderr)
torch.backends.cuda.matmul.allow_tf32 = False
torch.backends.cudnn.allow_tf32 = False
g = torch.Generator().manual_seed(1234)

# 1. matmul fp32 1024^2 vs CPU
a = torch.randn(1024, 1024, generator=g); b = torch.randn(1024, 1024, generator=g)
c = (a.cuda() @ b.cuda()).cpu(); ref = a @ b
err = (c - ref).abs().max().item(); chk("matmul_fp32", err < 1e-2, f"maxerr={err:.2e}")

# 2. fp16 tensor-core matmul vs fp32 CPU
c16 = (a.cuda().half() @ b.cuda().half()).float().cpu()
rel = ((c16 - ref).norm() / ref.norm()).item(); chk("matmul_fp16", rel < 5e-3, f"relerr={rel:.2e}")

# 3. conv2d (cuDNN) vs CPU
x = torch.randn(8, 16, 64, 64, generator=g); w = torch.randn(32, 16, 3, 3, generator=g)
y = torch.nn.functional.conv2d(x.cuda(), w.cuda(), padding=1).cpu(); yr = torch.nn.functional.conv2d(x, w, padding=1)
err = (y - yr).abs().max().item(); chk("conv2d_cudnn", err < 1e-2, f"maxerr={err:.2e}")

# 4. a small CNN forward + one SGD step, GPU vs CPU (same init)
torch.manual_seed(7)
net = nn.Sequential(nn.Conv2d(3, 16, 3, padding=1), nn.BatchNorm2d(16), nn.ReLU(), nn.MaxPool2d(2),
                    nn.Conv2d(16, 32, 3, padding=1), nn.ReLU(), nn.AdaptiveAvgPool2d(1), nn.Flatten(), nn.Linear(32, 10))
import copy
net_c = copy.deepcopy(net); net_g = copy.deepcopy(net).cuda()
xb = torch.randn(16, 3, 32, 32, generator=g); yb = torch.randint(0, 10, (16,), generator=g)
for n_, dev in ((net_c, "cpu"), (net_g, "cuda")):
    opt = torch.optim.SGD(n_.parameters(), lr=0.1)
    loss = nn.functional.cross_entropy(n_(xb.to(dev)), yb.to(dev)); opt.zero_grad(); loss.backward(); opt.step()
wc = net_c[8].weight; wg = net_g[8].weight.cpu()
err = (wc - wg).abs().max().item(); chk("cnn_train_step", err < 1e-3, f"maxerr={err:.2e}")
dig("cnn_train_step", wg)

# 5. large alloc + memcpy round trip byte-exact (1 GiB)
t = torch.arange(256 * 1024 * 1024, dtype=torch.int32)
back = t.cuda().cpu(); chk("htod_dtoh_1GiB", torch.equal(t, back))

# 6. reductions / sort / scan vs CPU
v = torch.randn(1 << 22, generator=g)
s_ok = torch.allclose(v.cuda().sort().values.cpu(), v.sort().values)
cs = v.double().cuda().cumsum(0).cpu(); cs_ok = torch.allclose(cs, v.double().cumsum(0), atol=1e-6)
chk("sort_cumsum", s_ok and cs_ok)

# 7. multiple streams + events
st = [torch.cuda.Stream() for _ in range(4)]; outs = []
for i, s_ in enumerate(st):
    with torch.cuda.stream(s_):
        outs.append((torch.full((1 << 20,), float(i), device="cuda") * 2).sum())
torch.cuda.synchronize(); vals = [o.item() for o in outs]
chk("multi_stream", vals == [2.0 * i * (1 << 20) for i in range(4)])
print("TORCH_CORRECT_DONE", flush=True)

# 8. ★ expandable segments (only when the allocator was configured for them — row torch_expseg)
if "expandable_segments:True" in os.environ.get("PYTORCH_CUDA_ALLOC_CONF", ""):
    chunk = 64 << 20  # int32 elements = 256 MiB per tensor; (i + 1) * arange stays below 2**31
    def write(i, t):
        torch.arange(chunk, dtype=torch.int32, device="cuda", out=t)  # a kernel writes it
        t.mul_(i + 1)
    torch.cuda.synchronize(); torch.cuda.empty_cache()
    ts = [torch.empty(chunk, dtype=torch.int32, device="cuda") for _ in range(4)]  # the segment grows
    for i, t in enumerate(ts):
        write(i, t)
    torch.cuda.synchronize()
    before = [t.data_ptr() for t in ts]
    del ts[1:3]  # free the middle two ...
    torch.cuda.synchronize(); torch.cuda.empty_cache()  # ... and UNMAP their pages (the VA stays reserved)
    ts.insert(1, torch.empty(chunk, dtype=torch.int32, device="cuda"))  # remap fresh pages
    ts.insert(2, torch.empty(chunk, dtype=torch.int32, device="cuda"))
    for i in (1, 2):
        write(i, ts[i])
    torch.cuda.synchronize()
    after = [t.data_ptr() for t in ts]
    ref = torch.arange(chunk, dtype=torch.int32)
    bad = sum(int((t.cpu() != ref * (i + 1)).sum().item()) for i, t in enumerate(ts))  # every element
    dig("expandable_segments", torch.cat([t[::4096] for t in ts]))
    segs = torch.cuda.memory._snapshot().get("segments", [])
    field = bool(segs) and all("is_expandable" in sg for sg in segs)  # FAIL if the field is missing
    expandable = field and any(sg["is_expandable"] for sg in segs)
    reused = sum(a == b for a, b in zip(before, after))
    chk("expandable_segments", field and expandable and bad == 0,
        f"bad={bad} segments={len(segs)} is_expandable_field={field} expandable={expandable} "
        f"same_va_after_remap={reused}/4")
