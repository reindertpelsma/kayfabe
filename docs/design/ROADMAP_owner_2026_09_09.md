# ★★★★★ THE ROADMAP — owner, 2026-09-09

**STATUS: LIVE.** The owner's stated order. Everything below is his framing, kept in his order,
with the reasoning that came with it — the reasoning is the part that survives re-derivation.

## The scoping insight that sets the order
> *"if the LLM passes then the rest of the cuda workloads will go very rapid that's not UVM.
> **UVM needs its own treatment.** Before we do UVM I think it's then better to port to driver archs
> and GPU archs."*

⇒ The LLM passing (`W392_OUTCOME=(P)`, text byte-identical to the same-boot CPU oracle, 2026-09-09)
is the gate that makes the **non-UVM** CUDA surface cheap. UVM is a separate lane and is
deliberately deferred **behind** the porting work.

## The order

| # | lane | why it sits here |
|---|---|---|
| 1 | **CUDA apps work** | unlocked by the LLM pass; the non-UVM surface should go fast |
| 2 | **Parity** | tok/s and throughput against a native baseline on the same box |
| 3 | **Driver versions + GPU archs** — ★ **GPU MODELS HAVE HIGHER PRIORITY** | ⇒ *"a driver version is preventable by installing another; an arch is literally it doesn't work without any solution, so that must be really well supported."* A user can downgrade a driver. A user cannot change the silicon in their machine. |
| 4 | **UVM** | its own treatment, deliberately after the porting work |
| 5 | **Graphics — headless, no display** | compute-shaped; no display plane needed |
| 6 | **Display — a real VGA output** | see the shape below |
| 7 | **Windows** 🪟 | the end goal |

## The display shape (owner's design, recorded before it is built)
> *"kayfabe first presents the GPU as a **VGA device with a linear buffer** support (before the GPU
> driver is loaded) and after the GPU driver is loaded **it takes over, just like on real NVIDIA
> hardware**, where it draws on a **nvidia-native zero-copy KMS plane** like `nvkvm-pv` (which has
> great docs about it). This also means there is **1 display, not a separate nvkvm display** like
> `nvkvm-pv` has."*

★ Two requirements hide in that paragraph and both are load-bearing:
1. **The handover must mirror real hardware** — VGA linear framebuffer pre-driver, NVIDIA KMS
   post-driver, on the *same* device. Not two devices, not a mode switch the guest can observe as
   anything other than what silicon does.
2. **ONE display.** `nvkvm-pv` ships a *separate* nvkvm display; that is explicitly **not** what is
   wanted here. Its KMS-plane docs are the reference for the zero-copy path, not for the topology.

## What is already true (measured, 2026-09-09)
- raw client `W392D_OUTCOME=(P)` — P1/P2/P3/STALE RACE ✔ and **THREADS 4 of 4**, falsifier honest.
- LLM `W392_OUTCOME=(P)` — 16 tokens, byte-identical to the CPU oracle, **0 Xids**.
- BAR1 trap census: **~87,700 accesses per raw-client run**; demand-install mirror in flight on
  branch `w393-bar-passthrough`.
