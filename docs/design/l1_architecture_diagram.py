#!/usr/bin/env python3
"""Render the L1 concurrency architecture diagram (l1_concurrency.md §2) to PNG.

Read-only companion to l1_architecture_summary.md — regenerate with:
    python3 l1_architecture_diagram.py
"""
import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.patches import FancyBboxPatch, FancyArrowPatch, Circle

fig, ax = plt.subplots(figsize=(17.5, 12.5))
ax.set_xlim(0, 100)
ax.set_ylim(0, 100)
ax.axis("off")

# ---------- palette ----------
C_GUEST = ("#dbeafe", "#1d4ed8")
C_VCPU = ("#e0e7ff", "#3730a3")
C_DEV = ("#fef9c3", "#a16207")
C_PROC = ("#dcfce7", "#15803d")
C_ISO = ("#ffedd5", "#c2410c")
C_HOST = ("#e5e7eb", "#374151")
C_WATCH = ("#f3e8ff", "#7e22ce")
C_INBOX = ("#fce7f3", "#be185d")


def box(x, y, w, h, fc, ec, lw=1.4, ls="-", r=0.6, z=1):
    p = FancyBboxPatch(
        (x, y), w, h,
        boxstyle=f"round,pad=0,rounding_size={r}",
        facecolor=fc, edgecolor=ec, linewidth=lw, linestyle=ls, zorder=z,
    )
    ax.add_patch(p)
    return p


def txt(x, y, s, size=8.2, weight="normal", color="#111827", ha="center",
        va="center", z=5, family="sans-serif", style="normal"):
    ax.text(x, y, s, fontsize=size, fontweight=weight, color=color,
            ha=ha, va=va, zorder=z, family=family, style=style, linespacing=1.35)


def arrow(x1, y1, x2, y2, color="#111827", lw=1.6, style="-|>", con="arc3,rad=0",
          z=4, ls="-", shrinkA=2, shrinkB=2, ms=14):
    a = FancyArrowPatch(
        (x1, y1), (x2, y2), arrowstyle=style, connectionstyle=con,
        color=color, linewidth=lw, linestyle=ls, zorder=z,
        shrinkA=shrinkA, shrinkB=shrinkB, mutation_scale=ms,
    )
    ax.add_patch(a)


def step(x, y, n, color, z=8):
    c = Circle((x, y), 1.15, facecolor=color, edgecolor="white", lw=1.0, zorder=z)
    ax.add_patch(c)
    ax.text(x, y, str(n), fontsize=8.5, fontweight="bold", color="white",
            ha="center", va="center", zorder=z + 1)


RED = "#dc2626"     # doorbell route/act path
PUR = "#7e22ce"     # completion path

# ================= title =================
txt(50, 98.6, "kayfabe Mode-2 — PLANNED L1 concurrency architecture (l1_concurrency.md §2, HEAD 488117c)",
    size=13, weight="bold")
txt(50, 96.6, "four thread roles · two locks · one queue  —  lock order (total, one-way): device → proc → leaf; no lock across join/barrier",
    size=9.5, style="italic", color="#374151")

# ================= GUEST =================
box(3, 87.5, 56, 7.2, *C_GUEST)
txt(31, 93.1, "GUEST VM — unmodified guest, stock NVIDIA kernel driver", size=10, weight="bold", color=C_GUEST[1])
txt(31, 90.2, "vCPU 0 … vCPU N-1 — doorbell writes · GSP RPC queue · rare RM control traps\n"
              "(pushbuffers + completion semaphores are PASSTHROUGH pages: guest polls its own RAM, ~zero traps steady-state)",
    size=7.8)

# ================= vCPU trap threads =================
box(3, 79.0, 56, 6.0, *C_VCPU)
txt(31, 83.5, "vCPU TRAP THREADS  (N, owned by the VMM — KVM exit is a synchronous upcall)", size=9.5, weight="bold", color=C_VCPU[1])
txt(31, 81.0, "Device::mmio_read/write → decode trap (pure) → take the locks below → compose GSP reply → resume vCPU", size=7.8)

arrow(24, 87.5, 24, 85.0, lw=1.8)
txt(23.2, 86.3, "MMIO trap (KVM exit)", size=7.5, ha="right")
arrow(45, 85.0, 45, 87.5, lw=1.8, color="#15803d")
txt(45.8, 86.3, "reply / resume", size=7.5, ha="left", color="#15803d")

# ================= DEVICE LOCK domain =================
box(3, 60.5, 56, 15.5, C_DEV[0], C_DEV[1], lw=2.2, ls="--")
txt(31, 74.4, "DEVICE LOCK — one RwLock over the pure spine", size=10, weight="bold", color="#854d0e")
txt(31, 72.3, "R1: NO host I/O ever under this lock — pure core logic + at most Vmm::raise_irq (non-blocking eventfd)",
    size=7.8, style="italic", color="#854d0e")

box(5, 62.0, 25.5, 8.6, "#fefce8", C_DEV[1], lw=1.2)
txt(17.75, 69.2, "READ (shared) — route / resolve", size=8.5, weight="bold", color="#854d0e")
txt(17.75, 65.5, "route_doorbell: token → (Proc, Chan, GPU)\nvia by_vchid[(gpu, vchid)]\nresolve via by_pdb[(gpu, pdb)] · routing lookups\nheld for the DURATION of any proc op (R2)",
    size=7.3)

box(32.5, 62.0, 24.5, 8.6, "#fefce8", C_DEV[1], lw=1.2)
txt(44.75, 69.2, "WRITE (exclusive) — mutate / pump", size=8.5, weight="bold", color="#854d0e")
txt(44.75, 65.5, "Gpu::apply (RmGraph, transactional)\nprojection refresh · target minting\nisolate spawn = RESERVE only (two-phase)\ndeliver_completions pump  (+ raise_irq)",
    size=7.3)

arrow(20, 79.0, 17.75, 70.6, lw=1.6)
arrow(42, 79.0, 44.75, 70.6, lw=1.6)
txt(40.2, 77.6, "rare RM ops", size=7.2, ha="right", color="#374151")

# ================= PER-PROC domain =================
box(3, 38.5, 56, 20.0, C_PROC[0], C_PROC[1], lw=2.2, ls="--")
txt(31, 56.9, "PER-PROC LOCKS — one Mutex<Proc> per guest process (the #14 blast-radius boundary reused)", size=9.6, weight="bold", color="#166534")
txt(31, 55.0, "R2: held WITH the device READ lock · R3: never acquire device while holding proc · blocking is allowed HERE ONLY (confined)",
    size=7.6, style="italic", color="#166534")

for (x0, name, extra) in [
    (5,  "Proc A", "act phase of the doorbell:\ngate_working_set (MISS=FAULT)\n→ materialize-on-first-touch\n→ schedule → ring"),
    (23.5, "Proc B", "publish_backing (&mut Proc)\nCompletionQueue::observe\n(parallel with Proc A —\nno shared lock)"),
    (42, "system Proc", "kernel / CeUtils traffic\n⚠ B3 residual: a stall here\nstalls kernel forwarding\n(watchdog + #73 interrupt)"),
]:
    w = 16.5 if x0 < 42 else 15
    box(x0, 39.7, w, 13.6, "#f0fdf4", C_PROC[1], lw=1.2)
    txt(x0 + w / 2, 51.9, name, size=8.8, weight="bold", color="#166534")
    txt(x0 + w / 2, 49.9, "Vas+AddressTable · Channels+ExecPlane\nGPA arena · CompletionQueue · isolates[gpu]" if x0 < 42
        else "Vas+AddressTable · Channels\narena · queue · isolates[gpu]", size=6.7, color="#374151")
    txt(x0 + w / 2, 44.2, extra, size=7.0)

arrow(14, 62.0, 13.25, 53.3, lw=1.6)
arrow(31, 60.5, 31.75, 53.3, lw=1.6)
txt(27.2, 59.3, "drop to owning proc's lock", size=7.2, ha="right", color="#374151")

# ================= ISOLATE workers =================
box(3, 24.0, 56, 11.0, C_ISO[0], C_ISO[1], lw=1.8)
txt(31, 33.4, "ISOLATE WORKER PROCESSES — one sandboxed process per (Proc, GpuId)", size=9.6, weight="bold", color="#9a3412")
txt(31, 31.5, "namespaces · pivot_root · seccomp · unprivileged uid (Mode-1 stub posture) — its handle namespace dies with it",
    size=7.4, style="italic", color="#9a3412")
for (x0, name) in [(5, "worker (A, GPU0)"), (23.5, "worker (B, GPU0)"), (42, "worker (sys, GPU0)")]:
    w = 16.5 if x0 < 42 else 15
    box(x0, 25.0, w, 5.2, "#fff7ed", C_ISO[1], lw=1.1)
    txt(x0 + w / 2, 28.8, name, size=8.2, weight="bold", color="#9a3412")
    txt(x0 + w / 2, 26.7, "single-threaded verb loop · blocking\nreal RM ioctl · EINTR interrupt (#73)", size=6.6)

for xa in (13.25, 31.75, 49.5):
    arrow(xa, 39.7, xa, 30.2, lw=1.5, style="<|-|>", color="#9a3412")
txt(31, 37.0, "socket: sync, strictly 1-deep request/reply — RmBackend verbs (alloc / map / schedule / ring / control) — caller MAY BLOCK",
    size=7.2, color="#9a3412")

# ================= HOST =================
box(3, 15.5, 56, 6.2, *C_HOST)
txt(31, 20.2, "HOST — kernel RM (/dev/nvidia*) → real GPU(s)", size=9.5, weight="bold", color="#1f2937")
txt(31, 17.6, "GPU writes completion semaphores INTO passthrough guest pages (guest sees them with no L1 thread involved)\nand signals host os-event fds on interrupt-shaped completions",
    size=7.3)
for xa in (13.25, 31.75, 49.5):
    arrow(xa, 25.0, xa, 21.7, lw=1.4, color="#374151")
txt(31, 23.3, "real RM ioctls (unprivileged)", size=7.0, color="#374151")

# ================= RIGHT COLUMN: watcher / inbox / executor =================
box(63, 60.5, 34, 15.5, C_WATCH[0], C_WATCH[1], lw=1.8)
txt(80, 74.3, "WATCHER THREAD (1)", size=10, weight="bold", color=C_WATCH[1])
txt(80, 72.4, "one blocking epoll_wait — ZERO busy-poll (F1)", size=7.9, style="italic", color=C_WATCH[1])
txt(80, 66.6, "fds watched:\n· isolate sockets (HUP = worker death)\n· host os-event fds (armed at registration)\n· timerfd (earliest Vmm::defer deadline)\n\nconverts readiness → CoreEvent; touches NO core\nstate (it holds no device reference at all)",
    size=7.3)

box(66, 51.5, 28, 5.2, C_INBOX[0], C_INBOX[1], lw=1.6)
txt(80, 54.9, "CoreEvent inbox (std::sync::mpsc)", size=8.8, weight="bold", color=C_INBOX[1])
txt(80, 52.9, "the ONE concurrent L1 structure · producers: watcher + Vmm::defer", size=7.0)

box(63, 33.5, 34, 14.5, C_WATCH[0], C_WATCH[1], lw=1.8)
txt(80, 46.4, "EXECUTOR (1, serialized — the Vmm::defer target)", size=9.4, weight="bold", color=C_WATCH[1])
txt(80, 39.9, "drains the inbox in order; runs Device::event under the\nSAME two locks as any vCPU thread:\n· IsolateComplete{session, cookie} → observe + pump\n· Deferred(DeferredReap) → Gpu::reap_retired (quiesce, L10)\n· Deferred(CompletionRedeliver) → bounded backstop\n  (armed only while completions outstanding — never periodic)\n· LockedRegionFault → update → restore",
    size=7.2)

arrow(80, 60.5, 80, 56.7, lw=1.6, color=C_WATCH[1])
arrow(80, 51.5, 80, 48.0, lw=1.6, color=C_WATCH[1])
txt(81.2, 58.5, "push, wake", size=7.0, ha="left", color=C_WATCH[1])
txt(81.2, 49.7, "condvar drain", size=7.0, ha="left", color=C_WATCH[1])

# host → watcher (os-event): route AROUND the right edge (no box crossings)
ax.plot([59, 98.8], [17.5, 17.5], color=PUR, lw=1.6, zorder=4)
ax.plot([98.8, 98.8], [17.5, 68.0], color=PUR, lw=1.6, zorder=4)
arrow(98.8, 68.0, 97.2, 68.0, lw=1.6, color=PUR, shrinkA=0, shrinkB=0)
t = ax.text(99.9, 42.0, "host RM signals os-event fd", fontsize=7.2, color=PUR,
            ha="center", va="center", rotation=90, zorder=5)
# isolate socket HUP → watcher (label lives inside the watcher's fd list)
arrow(59, 29.5, 63.5, 63.0, lw=1.2, color="#9a3412", ls=":", con="arc3,rad=-0.1")

# executor → per-proc (observe) and → device write (pump)
arrow(63, 41.5, 59, 47.0, lw=1.6, color=PUR, con="arc3,rad=0.12")
txt(60.0, 42.6, "observe\n(proc lock)", size=6.9, color=PUR, ha="left")
arrow(63, 44.5, 57, 66.0, lw=1.6, color=PUR, con="arc3,rad=0.25")
txt(61.5, 56.5, "pump (device\nWRITE lock)", size=6.9, color=PUR, ha="left")

# pump → guest IRQ (SWGEN0): from device write box up the left edge to guest
arrow(1.8, 66.0, 1.8, 89.0, lw=1.8, color=PUR, con="arc3,rad=0.0", shrinkA=0, shrinkB=0)
arrow(5.0, 68.0, 1.9, 69.5, lw=1.8, color=PUR, con="arc3,rad=0.2")
arrow(1.85, 89.0, 6.0, 90.5, lw=1.8, color=PUR, con="arc3,rad=0.2")
txt(0.9, 78.0, "Vmm::raise_irq(SWGEN0) → guest IRQ", size=7.3, color=PUR, ha="center", va="center")
ax.texts[-1].set_rotation(90)

# ================= numbered paths =================
# doorbell (red): 1 guest write, 2 route (device read), 3 act (proc lock), 4 ring verb, 5 reply
step(9.5, 91.5, 1, RED)
step(7.0, 69.2, 2, RED)
step(7.0, 51.9, 3, RED)
step(13.25, 38.5, 4, RED)
step(53.5, 86.3, 5, RED)
# completion (purple): 1 host sema/os-event, 2 watcher, 3 inbox->executor, 4 observe, 5 pump, 6 IRQ
step(52.5, 17.5, 1, PUR)
step(65.2, 74.3, 2, PUR)
step(77.0, 49.7, 3, PUR)
step(60.5, 45.2, 4, PUR)
step(58.2, 64.5, 5, PUR)
step(5.3, 88.6, 6, PUR)

# ================= legend =================
box(63, 14.0, 34, 16.5, "#f9fafb", "#6b7280", lw=1.2)
txt(80, 29.2, "THE TWO HOT PATHS (numbered)", size=8.8, weight="bold", color="#111827")
ax.text(64.2, 27.9, "DOORBELL (route/act split, R4):\n"
        "① guest rings → trap   ② ROUTE under device READ lock\n"
        "③ ACT under proc Mutex: gate → materialize → schedule\n"
        "④ ring_doorbell verb over socket (blocking, confined)\n"
        "⑤ reply → resume vCPU",
        fontsize=6.9, ha="left", va="top", color=RED, zorder=5, linespacing=1.3)
ax.text(64.2, 21.0, "COMPLETION (edge-driven, F2/F3):\n"
        "① host signals os-event   ② watcher epoll   ③ inbox → executor\n"
        "④ observe (proc lock, per-proc — never cross-proc gated)\n"
        "⑤ pump (device WRITE lock, pure + raise_irq)   ⑥ guest IRQ\n"
        "re-pump edges: guest's OWN poll · IRQSCLR · defer backstop",
        fontsize=6.9, ha="left", va="top", color=PUR, zorder=5, linespacing=1.3)

plt.savefig("/workspace/nvkvm-rs/docs/design/l1_architecture_diagram.png",
            dpi=170, bbox_inches="tight", facecolor="white")
print("written")
