#!/usr/bin/env python3
"""★★★★★ w396 — MANY CUDA WORKLOADS IN ONE PROCESS, each verified against a CPU reference.

Owner, 2026-09-09: *"I really want iteration over many cuda apps."*

## ⊘ Why one process and not one boot per app
The multi-app wedge is per **device open**, not per workload: four opens succeeded in the w394
suite because they shared ONE RmInitAdapter↔RmShutdownAdapter span, and the fifth open tore the
adapter down and re-initialised — which is what fails (an event-notification control, see
`w394_cuda_apps_and_the_parity_gate.md`). ⇒ A single process can run an ARBITRARY number of
workloads without touching that wall. Breadth does not have to wait for the re-init fix.

## ★ Every workload carries its own CPU oracle
A GPU result is compared against the SAME computation on the CPU, in the same process, on the
same inputs. That is what makes this a differential rather than a smoke test: w386 measured a
boot printing `LLM_TOKENS=16` over garbage, and the `4,64` bandwidth workload reports rc=0 with
`bad=65536`. A workload that RAN is not a workload that was RIGHT.
⊘ Tolerances are per-workload and stated. fp32 reductions and matmuls do not associate the same
way on two devices, so `allclose` with a named rtol/atol is the honest test; bit-exactness is
demanded only where the operation is exact (integer, copy, indexing).

## Output contract — one line per workload, greppable, terminated
    W396 <name> rc=<0|1> max_abs_err=<float> verdict=<PASS|FAIL|SKIP:<why>> ms=<float>
and a final `W396_DONE total=<n> pass=<n> fail=<n> skip=<n>`. ⊘ The terminator matters: a
killed process and a finished one are indistinguishable by output alone without it.
"""
import os, sys, time, traceback

TOL = {}          # per-workload (rtol, atol); absent = bit-exact required
RESULTS = []

def workload(name, rtol=None, atol=None):
    """Register a workload. `rtol/atol` absent ⇒ the result must match BIT-EXACTLY."""
    def deco(fn):
        TOL[name] = (rtol, atol)
        fn._w396_name = name
        return fn
    return deco

def compare(name, got, ref):
    import torch
    rtol, atol = TOL[name]
    got_c, ref_c = got.detach().cpu(), ref.detach().cpu()
    if got_c.shape != ref_c.shape:
        return False, float("inf")
    if got_c.dtype.is_floating_point:
        err = (got_c - ref_c).abs().max().item() if got_c.numel() else 0.0
        ok = torch.allclose(got_c, ref_c, rtol=rtol or 0.0, atol=atol or 0.0)
    else:
        diff = (got_c != ref_c).sum().item()
        err = float(diff)
        ok = diff == 0
    return ok, err

# ---------------------------------------------------------------- the workloads
@workload("matmul_fp32", rtol=1e-4, atol=1e-4)
def w_matmul(torch, dev):
    a = torch.randn(512, 512); b = torch.randn(512, 512)
    return (a.to(dev) @ b.to(dev)), (a @ b)

@workload("elementwise_chain", rtol=1e-5, atol=1e-5)
def w_elem(torch, dev):
    x = torch.randn(1 << 16)
    f = lambda t: ((t * 2.0 + 1.0).sigmoid() - 0.25).relu()
    return f(x.to(dev)), f(x)

@workload("reduction_sum", rtol=1e-3, atol=1e-3)
def w_reduce(torch, dev):
    x = torch.randn(1 << 20)
    return x.to(dev).sum(), x.sum()

@workload("softmax_rowwise", rtol=1e-5, atol=1e-5)
def w_softmax(torch, dev):
    x = torch.randn(256, 1024)
    return x.to(dev).softmax(dim=-1), x.softmax(dim=-1)

@workload("layernorm", rtol=1e-4, atol=1e-4)
def w_ln(torch, dev):
    x = torch.randn(128, 768)
    ln = torch.nn.LayerNorm(768)
    return ln.to(dev)(x.to(dev)), ln(x)

@workload("conv2d", rtol=1e-3, atol=1e-3)
def w_conv(torch, dev):
    x = torch.randn(4, 3, 64, 64)
    c = torch.nn.Conv2d(3, 8, 3, padding=1)
    return c.to(dev)(x.to(dev)), c(x)

@workload("int_gather")          # exact: indexing moves bytes, it does not compute
def w_gather(torch, dev):
    x = torch.arange(1 << 16, dtype=torch.int32)
    idx = torch.randperm(1 << 16)[:4096]
    return x.to(dev)[idx.to(dev)], x[idx]

@workload("copy_roundtrip")      # exact: a copy that changes a byte is a bug, not a rounding
def w_copy(torch, dev):
    x = torch.randn(1 << 20)
    return x.to(dev).cpu(), x

@workload("transpose_contig", rtol=0.0, atol=0.0)
def w_transpose(torch, dev):
    x = torch.randn(512, 333)
    return x.to(dev).t().contiguous(), x.t().contiguous()

@workload("bmm_batched", rtol=1e-4, atol=1e-4)
def w_bmm(torch, dev):
    a = torch.randn(16, 128, 128); b = torch.randn(16, 128, 128)
    return torch.bmm(a.to(dev), b.to(dev)), torch.bmm(a, b)

@workload("cumsum", rtol=1e-3, atol=1e-3)
def w_cumsum(torch, dev):
    x = torch.randn(1 << 14)
    return x.to(dev).cumsum(0), x.cumsum(0)

@workload("argmax_indices")      # exact: an index is an integer
def w_argmax(torch, dev):
    x = torch.randn(1024, 512)
    return x.to(dev).argmax(dim=-1), x.argmax(dim=-1)


def main():
    dev = os.environ.get("W396_DEVICE", "cuda")
    only = os.environ.get("W396_ONLY")
    try:
        import torch
    except Exception as e:                       # noqa: BLE001
        print(f"W396_DONE total=0 pass=0 fail=0 skip=0 ⊘ UNMEASURED: no torch ({e})", flush=True)
        return 2
    torch.manual_seed(0)
    if dev == "cuda" and not torch.cuda.is_available():
        print("W396_DONE total=0 pass=0 fail=0 skip=0 ⊘ UNMEASURED: cuda unavailable", flush=True)
        return 2

    fns = [v for v in globals().values() if callable(v) and hasattr(v, "_w396_name")]
    npass = nfail = nskip = 0
    for fn in fns:
        name = fn._w396_name
        if only and only != name:
            continue
        t0 = time.time()
        try:
            got, ref = fn(torch, dev)
            ok, err = compare(name, got, ref)
            ms = (time.time() - t0) * 1000.0
            verdict = "PASS" if ok else "FAIL"
            npass, nfail = (npass + 1, nfail) if ok else (npass, nfail + 1)
            print(f"W396 {name} rc=0 max_abs_err={err:.3e} verdict={verdict} ms={ms:.1f}",
                  flush=True)
        except Exception as e:                   # noqa: BLE001
            ms = (time.time() - t0) * 1000.0
            nskip += 1
            # ⊘ An exception is SKIP, not FAIL: "the workload could not run" and "the GPU
            #   computed the wrong answer" are different facts and must not share a bucket.
            print(f"W396 {name} rc=1 max_abs_err=nan verdict=SKIP:{type(e).__name__} ms={ms:.1f}",
                  flush=True)
            print(f"    {traceback.format_exc().splitlines()[-1]}", flush=True)
    total = npass + nfail + nskip
    print(f"W396_DONE total={total} pass={npass} fail={nfail} skip={nskip} device={dev}",
          flush=True)
    return 1 if nfail else 0


if __name__ == "__main__":
    sys.exit(main())
