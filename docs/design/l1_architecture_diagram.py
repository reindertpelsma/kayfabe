#!/usr/bin/env python3
"""Render the architecture-review diagrams for `l1_architecture_summary.md`.

Three figures, all matplotlib. Only this script is committed; the PNGs it
writes are gitignored and regenerated on demand:

  1. l1_diagram_system.png   — the whole planned system + the L0..L3 layer model
  2. l1_diagram_runtime.png  — the L1 thread / lock architecture, as BUILT
  3. l1_diagram_dataflow.png — RmGraph -> projections -> the four planes

Regenerate with:  python3 docs/design/l1_architecture_diagram.py

Layout convention: every multi-line body block is placed with `body()`, which
anchors at the TOP of the block. Vertical centring of variable-length text
inside a fixed box is what made the previous revision overlap its own titles.
"""

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.patches import FancyBboxPatch, FancyArrowPatch

OUT = "/workspace/nvkvm-rs/docs/design"

# ---------------------------------------------------------------- palette ---
GUEST = ("#dbeafe", "#1d4ed8")
EMUL = ("#ede9fe", "#6d28d9")
CORE = ("#dcfce7", "#15803d")
SHELL = ("#fef9c3", "#a16207")
ISO = ("#ffedd5", "#c2410c")
HOST = ("#e5e7eb", "#374151")
GREY = ("#f9fafb", "#6b7280")

DONE = "#15803d"
PART = "#a16207"
TODO = "#b91c1c"
PUR = "#7e22ce"


def canvas(w, h):
    fig, ax = plt.subplots(figsize=(w, h))
    ax.set_xlim(0, 100)
    ax.set_ylim(0, 100)
    ax.axis("off")
    fig.subplots_adjust(left=0, right=1, top=1, bottom=0)
    return fig, ax


def box(ax, x, y, w, h, fc, ec, lw=1.4, ls="-", r=0.7, z=1):
    ax.add_patch(FancyBboxPatch(
        (x, y), w, h, boxstyle=f"round,pad=0,rounding_size={r}",
        facecolor=fc, edgecolor=ec, linewidth=lw, linestyle=ls, zorder=z))


def txt(ax, x, y, s, size=9, weight="normal", color="#111827", ha="center",
        va="center", z=5, style="normal", rot=0):
    ax.text(x, y, s, fontsize=size, fontweight=weight, color=color,
            ha=ha, va=va, zorder=z, style=style, linespacing=1.45, rotation=rot)


def body(ax, x, ytop, s, size=7.4, color="#111827", ha="center", z=5,
         style="normal"):
    """Multi-line block anchored at its TOP edge — never centred."""
    ax.text(x, ytop, s, fontsize=size, color=color, ha=ha, va="top",
            zorder=z, linespacing=1.5, style=style)


def arrow(ax, x1, y1, x2, y2, color="#111827", lw=1.8, style="-|>",
          con="arc3,rad=0", z=4, ls="-", ms=15, sa=2, sb=2):
    ax.add_patch(FancyArrowPatch(
        (x1, y1), (x2, y2), arrowstyle=style, connectionstyle=con,
        color=color, linewidth=lw, linestyle=ls, zorder=z,
        shrinkA=sa, shrinkB=sb, mutation_scale=ms))


def badge(ax, cx, cy, label, color, w=13.0):
    box(ax, cx - w / 2, cy - 1.35, w, 2.7, "white", color, lw=1.5, r=1.3, z=6)
    txt(ax, cx, cy, label, size=7.8, weight="bold", color=color, z=7)


# ===========================================================================
# FIGURE 1 — the whole planned system, and the layer model
# ===========================================================================
def figure_system():
    fig, ax = canvas(18.0, 14.6)

    txt(ax, 50, 98.4, "kayfabe — the whole planned system: an unmodified guest, an "
        "emulated GPU, and real compute on a real host GPU",
        size=15.0, weight="bold")
    txt(ax, 50, 96.3,
        "North star:  cuCtxCreate → first compute → matmul, numerically correct   ·   "
        "correctness = observable end-states only, never step-by-step replay",
        size=9.8, style="italic", color="#374151")

    # ---------------- band 1: the guest ----------------
    box(ax, 2.0, 88.0, 96, 6.4, *GUEST, lw=2.0)
    txt(ax, 50, 92.8, "GUEST VM — completely unmodified", size=11.5,
        weight="bold", color=GUEST[1])
    body(ax, 50, 91.4,
         "CUDA / PyTorch / Vulkan applications  →  libcuda  →  the STOCK NVIDIA "
         "kernel driver + UVM (no guest patches, no paravirt driver)\n"
         "The guest believes it owns a real GPU: it allocates through RM, builds GPU "
         "page tables, allocates channels, and rings doorbells.", size=8.6)

    # ---------------- band 2: the emulated boundary ----------------
    box(ax, 2.0, 74.5, 66, 11.2, *EMUL, lw=2.0)
    txt(ax, 35.0, 83.9, "WHAT THE GUEST DRIVER TALKS TO — an emulated GPU with a "
        "FAKED GSP", size=11, weight="bold", color=EMUL[1])
    for (x0, w, head, text) in [
        (3.5, 20.0, "BAR MMIO + registers",
         "trapped reads and writes;\nstatic values are RAM-backed\n"
         "so guest polls do not vmexit"),
        (24.5, 20.5, "the faked GSP RPC ring",
         "GSP = the GPU System Processor,\nthe on-die CPU the real driver\n"
         "sends RM work to. We ARE it."),
        (46.0, 20.5, "doorbell writes + DMA",
         "the one mandatory hot-path trap:\n'channel N has work'\n(token → vChid)"),
    ]:
        box(ax, x0, 75.4, w, 7.0, "#f5f3ff", EMUL[1], lw=1.2)
        txt(ax, x0 + w / 2, 81.1, head, size=8.8, weight="bold", color=EMUL[1])
        body(ax, x0 + w / 2, 79.8, text, size=7.5)
    arrow(ax, 20, 88.0, 20, 85.9, lw=2.2, color=GUEST[1])

    # ---------------- the passthrough column (right) ----------------
    box(ax, 70.5, 44.0, 27.5, 41.7, "#ecfdf5", "#047857", lw=2.2, ls="--")
    txt(ax, 84.25, 83.9, "★ THE PASSTHROUGH FAST PATH", size=11, weight="bold",
        color="#047857")
    body(ax, 84.25, 81.6,
         "Userspace pushbuffers and completion\n"
         "semaphores are ORDINARY SHARED PAGES.\n"
         "The host GPU DMA-writes a semaphore\n"
         "straight into the page the guest polls,\n"
         "and the guest's own vCPU sees it.\n\n"
         "No trap. No thread of ours. No lock.\n\n"
         "This is why the steady-state hot path\n"
         "costs about nothing — and it is the\n"
         "reason the locking on the left is\n"
         "affordable at all: the contended paths\n"
         "are, by design, low-frequency ones.\n\n"
         "(It is also the load-bearing bet. If a\n"
         "real workload turns out to trap more\n"
         "than the C artifact measured, the lock\n"
         "story gets more expensive than it\n"
         "looks here.)", size=8.2, color="#065f46")
    arrow(ax, 84.25, 88.0, 84.25, 85.9, lw=2.2, color="#047857")
    arrow(ax, 84.25, 44.0, 84.25, 42.6, lw=2.4, color="#047857", ls="--")

    # ---------------- band 3: kayfabe ----------------
    box(ax, 2.0, 55.0, 66, 17.5, "#ffffff", "#111827", lw=2.4)
    txt(ax, 35.0, 70.7, "kayfabe  —  recover the guest's INTENT, forward real work",
        size=11.5, weight="bold")
    body(ax, 35.0, 69.3,
         "The guest kernel has already shredded 'cuCtxCreate' into privileged, "
         "GSP-internal steps.\nWe do not replay those. We reconstruct the userspace "
         "op and re-issue it, unprivileged.", size=8.2, style="italic",
         color="#374151")

    box(ax, 3.5, 61.5, 30.5, 4.6, *SHELL, lw=1.6)
    txt(ax, 18.75, 64.9, "L1 — the threaded shell   (BUILT: L1-M1)", size=8.8,
        weight="bold", color="#854d0e")
    body(ax, 18.75, 63.7,
         "ranked locks · plan/execute/commit verb seam\n"
         "N-worker isolate pool · reactor model · executor", size=7.4)

    box(ax, 36.0, 61.5, 30.5, 4.6, *CORE, lw=1.6)
    txt(ax, 51.25, 64.9, "L0 — the pure logic core   (COMPLETE)", size=8.8,
        weight="bold", color="#166534")
    body(ax, 51.25, 63.7,
         "RmGraph → projections · address table (MISS=FAULT)\n"
         "four planes, one set per guest process", size=7.4)

    box(ax, 3.5, 56.0, 63.0, 4.6, "#f9fafb", "#6b7280", lw=1.3, ls="--")
    txt(ax, 35.0, 59.4, "the two translation rules that make this legal",
        size=8.4, weight="bold", color="#374151")
    body(ax, 35.0, 58.2,
         "CASE 1 — the RPC essentially IS a userspace op (alloc a channel, map "
         "memory): re-issue it 1:1 on the host, through this process's isolate.\n"
         "CASE 2 — a GSP-internal control with no userspace equivalent "
         "(PROMOTE_CTX): ACK the guest, do nothing on the host. The host kernel "
         "already did it.", size=7.6)
    arrow(ax, 35.0, 74.5, 35.0, 72.7, lw=2.2, color=EMUL[1])

    # ---------------- band 4: isolates ----------------
    box(ax, 2.0, 43.5, 66, 9.6, *ISO, lw=2.0)
    txt(ax, 35.0, 51.4, "HOST ISOLATES — one sandboxed, UNPRIVILEGED process per "
        "(guest process, GPU)", size=10.5, weight="bold", color="#9a3412")
    body(ax, 35.0, 50.1,
         "namespaces · pivot_root · seccomp · cleared env and fds · unprivileged "
         "uid   —   its RM handle namespace dies with it",
         size=7.8, style="italic", color="#9a3412")
    for (x0, name) in [(4.0, "isolate (proc A, GPU 0)"),
                       (25.0, "isolate (proc B, GPU 0)"),
                       (46.0, "isolate (proc A, GPU 1)")]:
        box(ax, x0, 44.4, 19.0, 3.9, "#fff7ed", ISO[1], lw=1.2)
        txt(ax, x0 + 9.5, 47.2, name, size=8.0, weight="bold", color="#9a3412")
        body(ax, x0 + 9.5, 46.2,
             "a bounded POOL of workers,\neach one verb in flight", size=6.9)
    arrow(ax, 35.0, 55.0, 35.0, 53.3, lw=2.2, color="#9a3412")
    txt(ax, 36.4, 54.2, "RM verbs over a 1-deep request/reply channel",
        size=7.3, ha="left", color="#9a3412")

    # ---------------- band 5: host ----------------
    box(ax, 2.0, 34.5, 96, 7.4, *HOST, lw=2.0)
    txt(ax, 50, 40.2, "HOST — kernel RM via /dev/nvidia*  →  the real GPU(s)",
        size=10.5, weight="bold", color="#1f2937")
    body(ax, 50, 38.8,
         "Real contexts, real page tables, real execution. The host kernel-RM "
         "re-derives every privileged step itself — which is exactly why nothing we "
         "issue needs privilege.\nCompletions come back either as semaphore DMA "
         "writes into the passthrough pages (right), or as host os-event file "
         "descriptors (left).", size=7.8)
    arrow(ax, 35.0, 43.5, 35.0, 42.1, lw=2.2, color="#374151")

    # ---------------- the layer model ----------------
    txt(ax, 50, 31.0, "THE LAYER MODEL — what exists today, and what does not",
        size=12.0, weight="bold")

    layers = [
        (2.0, "L0 — the pure logic core", DONE, "COMPLETE",
         "No OS, no syscalls, no wall clock, no\n"
         "NVIDIA struct layouts, no GPU-generation\n"
         "or driver-version name anywhere. A\n"
         "deterministic state machine over facts\n"
         "the guest declared.\n\n"
         "99.2% mutation score on this surface;\n"
         "15 real bugs found before any hardware.\n\n"
         "★ Being OS-free is not tidiness. It is\n"
         "what lets the entire suite run with no\n"
         "GPU and no syscalls, and turns every\n"
         "multi-vCPU interleaving into a scripted\n"
         "call order instead of a race."),
        (26.5, "L1 — the Linux OS layer", PART, "M1 BUILT",
         "BUILT: the ranked lock discipline with\n"
         "always-on asserts, `SharedDevice` in both\n"
         "lock modes, the plan/execute/commit verb\n"
         "seam, the N-worker isolate pool, the\n"
         "condemned-component mechanism, the\n"
         "reactor's pure model, and the executor.\n\n"
         "NOT BUILT: the real wait loop and wake\n"
         "primitive, real sandboxed isolate\n"
         "processes and their wire protocol, verb\n"
         "interruption, the mmap / guest-RAM\n"
         "plumbing, and the one audited raw module\n"
         "that will hold the only `unsafe`."),
        (51.0, "L2 — QEMU / VMM adapter", TODO, "NOT BUILT",
         "The register file and MMIO trap dispatch;\n"
         "the GSP boot state machine, its mailbox\n"
         "latches and the seqNum RPC transport;\n"
         "memslot install and removal; interrupt\n"
         "injection.\n\n"
         "`Gpu` does not implement the `Device`\n"
         "port at all yet — the core's\n"
         "adapter-facing surface today is the\n"
         "event-level API, not registers.\n\n"
         "The GSP boot/reboot lifecycle is the\n"
         "largest un-modelled behaviour in the\n"
         "project, and is flagged as such."),
        (75.5, "L3 — per-arch ABI + real apps", TODO, "NOT BUILT",
         "`impl Arch for <generation>` — the class\n"
         "IDs, doorbell-token encoding, page-table\n"
         "formats, USERD layout and pushbuffer\n"
         "method encodings of one real GPU\n"
         "generation.\n\n"
         "Plus the wire-layout codegen from the\n"
         "open kernel modules, the GMMU page-table\n"
         "walker, and real CUDA / Vulkan apps on a\n"
         "real bench.\n\n"
         "★ The standing rule: adding a GPU\n"
         "generation must edit NO logic crate. A\n"
         "deliberately non-NVIDIA mock arch is the\n"
         "standing proof of that seam."),
    ]
    for (x, name, color, state, text) in layers:
        box(ax, x, 3.0, 22.5, 25.5, "white", color, lw=1.9)
        txt(ax, x + 11.25, 26.7, name, size=9.4, weight="bold", color=color)
        badge(ax, x + 11.25, 23.6, state, color)
        body(ax, x + 11.25, 21.4, text, size=7.1)

    fig.savefig(f"{OUT}/l1_diagram_system.png", dpi=150, bbox_inches="tight",
                facecolor="white")
    plt.close(fig)


# ===========================================================================
# FIGURE 2 — the runtime thread and lock architecture, as built
# ===========================================================================
def figure_runtime():
    fig, ax = canvas(18.0, 13.0)

    txt(ax, 50, 98.2, "The L1 runtime — threads, ranked locks, and the "
        "plan / execute / commit seam   (as BUILT at HEAD 3569d46)",
        size=14.5, weight="bold")
    txt(ax, 50, 95.8,
        "R1  no blocking call under ANY lock    ·    R3  acquire only in strictly "
        "increasing rank, at most one lock per rank    ·    R5  re-validate after "
        "every re-lock",
        size=9.6, style="italic", color="#374151")
    txt(ax, 50, 93.8,
        "all three are ALWAYS-ON asserts — a thread-local read is cheaper than the "
        "lock it guards, and compiling the detector out of the build that runs in "
        "production inverts the whole argument",
        size=8.4, style="italic", color="#6b7280")

    # ---------------- lock rank ladder (left) ----------------
    box(ax, 1.5, 24.0, 13.0, 65.0, "#fffbeb", "#a16207", lw=1.8, ls="--")
    txt(ax, 8.0, 87.0, "LOCK RANKS", size=10.0, weight="bold", color="#854d0e")
    for (y, h, rank, name, text, col) in [
        (68.0, 16.0, "0", "device",
         "`RankedRwLock`\nover the pure spine:\nRmGraph · projections\n"
         "by_pdb / by_vchid\nthe condemned set\nthe delivery pump", "#a16207"),
        (47.0, 16.0, "1", "proc",
         "`RankedMutex<Proc>` per\nguest process — plus a\nseparate cell for the\n"
         "system proc, which is\nnot in the map at all.\nAll four planes.", "#15803d"),
        (26.5, 15.0, "2", "leaf",
         "the executor inbox\nand the verb recorder.\nNothing may be\n"
         "acquired above a leaf.", "#be185d"),
    ]:
        box(ax, 2.5, y, 11.0, h, "white", col, lw=1.6)
        txt(ax, 8.0, y + h - 2.2, f"rank {rank} · {name}", size=8.8,
            weight="bold", color=col)
        body(ax, 8.0, y + h - 4.2, text, size=6.9)
    arrow(ax, 8.0, 68.0, 8.0, 63.3, lw=2.0, color="#374151")
    arrow(ax, 8.0, 47.0, 8.0, 41.8, lw=2.0, color="#374151")
    txt(ax, 8.0, 25.2, "one-way, total order", size=7.2, style="italic",
        color="#374151")

    # ---------------- thread roles ----------------
    roles = [
        (16.5, 26.0, "vCPU TRAP THREADS  (N)", "#e0e7ff", "#3730a3",
         "Owned by the VMM. A KVM exit is a\n"
         "SYNCHRONOUS upcall — the guest cannot\n"
         "resume until we reply. That is why the\n"
         "calling thread drives its own verb end\n"
         "to end instead of handing it to a\n"
         "scheduler: an actor model would put a\n"
         "cross-thread round trip on the one\n"
         "mandatory hot-path trap."),
        (44.5, 26.0, "REACTOR LOOP  (1)", "#f3e8ff", PUR,
         "One blocking wait over the registered\n"
         "completion sources. Maps readiness to an\n"
         "opaque `CompletionSource` (a pure table\n"
         "lookup) and pushes a `CoreEvent`.\n\n"
         "★ Holds NO reference to the device, so it\n"
         "cannot touch core state — that is\n"
         "enforced by what it was handed, not by\n"
         "a rule it must remember."),
        (72.5, 26.0, "EXECUTOR  (1, serialized)", "#f3e8ff", PUR,
         "Drains the `CoreEvent` inbox in order and\n"
         "runs the core's dispatch under the SAME\n"
         "locks and the SAME R1/R3/R5 rules a vCPU\n"
         "thread obeys.\n\n"
         "Asynchronous isolate I/O is meant to\n"
         "complete here — never by re-entry from a\n"
         "reactor or isolate thread. Today no verb\n"
         "is asynchronous, so that arm is unused."),
    ]
    for (x, w, name, fc, ec, text) in roles:
        box(ax, x, 74.5, w, 16.5, fc, ec, lw=1.8)
        txt(ax, x + w / 2, 89.1, name, size=10.0, weight="bold", color=ec)
        body(ax, x + w / 2, 87.4, text, size=7.3)

    # ---------------- the verb timeline ----------------
    box(ax, 16.5, 40.0, 82.0, 32.0, "white", "#111827", lw=2.0)
    txt(ax, 57.5, 69.6, "★ HOW ONE GUEST OPERATION THAT NEEDS THE HOST ACTUALLY "
        "RUNS — plan / execute / commit", size=10.8, weight="bold")
    body(ax, 57.5, 68.0,
         "This is the shape R1 forces. A locked phase may EMIT a verb; it may not "
         "CALL one. `Isolate::rm()` does not exist — a backend lives in a pool slot "
         "and `checkout` MOVES it out,\nso the old violating shape does not merely "
         "panic at runtime: it does not type-check.",
         size=7.8, style="italic", color="#374151")

    phases = [
        (18.5, 19.0, "1 · ROUTE + PLAN", "#fef9c3", "#a16207",
         "device READ  +  proc lock",
         "Decode the token or RM op, look up\n"
         "`by_vchid[(gpu, vchid)]`, gate the\n"
         "working set against the address table\n"
         "(a miss is a FAULT, here, before any\n"
         "host op), check out an idle worker, and\n"
         "emit a typed `VerbPlan` — IDs only,\n"
         "never a held reference.\n\n"
         "Microseconds. Calls nothing."),
        (40.5, 19.0, "2 · EXECUTE", "#fee2e2", "#b91c1c",
         "NO LOCK HELD  (asserted)",
         "The blocking round trip, on the calling\n"
         "thread, against the checked-out worker.\n"
         "The isolate is a separate process, so\n"
         "even ringing a doorbell is an IPC round\n"
         "trip rather than a store.\n\n"
         "The assert lives at the host-verb entry\n"
         "itself, on a thread-local held-rank\n"
         "mask — not at a wrapper someone has to\n"
         "remember to use."),
        (62.5, 19.0, "3 · COMMIT", "#dcfce7", "#15803d",
         "device READ  +  proc lock  (again)",
         "Re-resolve every ID through the graph\n"
         "and RE-VALIDATE. Three outcomes:\n\n"
         "· fine → apply the reply, field by field\n\n"
         "· CONVERGING stale — a sibling thread\n"
         "  materialized the same thing first →\n"
         "  release the duplicate and re-plan,\n"
         "  bounded at 8 passes\n\n"
         "· DIVERGENT stale — proc retired,\n"
         "  channel torn down, route rewritten →\n"
         "  REFUSE loudly, and hand back every\n"
         "  host object this attempt orphaned."),
    ]
    for (x, w, name, fc, ec, lockline, text) in phases:
        box(ax, x, 42.0, w, 23.5, fc, ec, lw=1.6)
        txt(ax, x + w / 2, 63.6, name, size=9.4, weight="bold", color=ec)
        txt(ax, x + w / 2, 61.7, lockline, size=7.5, weight="bold", color=ec)
        body(ax, x + w / 2, 60.2, text, size=6.9)
    for x in (37.8, 59.8):
        arrow(ax, x, 53.0, x + 2.3, 53.0, lw=2.4, color="#111827")

    box(ax, 83.0, 42.0, 14.5, 23.5, "#f9fafb", "#6b7280", lw=1.4, ls="--")
    txt(ax, 90.25, 63.6, "THE COST, NAMED", size=8.8, weight="bold")
    body(ax, 90.25, 61.6,
         "Each rank is taken\nTWICE per verb-issuing\nop, where the in-lock\n"
         "shape took it once —\nand more than twice on\nthe error and retry\n"
         "paths.\n\n"
         "A test asserts that\nCOUNT, not merely the\nabsence of a panic:\n"
         "collapsing it back to\none is exactly the\nregression R1 exists\n"
         "to prevent.\n\n"
         "And every dropped-lock\ngap is now a STALENESS\nwindow.", size=6.7)

    # ---------------- isolate pool ----------------
    box(ax, 16.5, 22.5, 45.0, 15.5, "#ffedd5", "#c2410c", lw=1.8)
    txt(ax, 39.0, 36.1, "ISOLATE — one sandboxed process per (Proc, GpuId)",
        size=10.0, weight="bold", color="#9a3412")
    body(ax, 39.0, 34.6,
         "Inside it: a BOUNDED POOL of workers (4 by default), each strictly one "
         "verb in flight\non its own 1-deep channel. Concurrency comes from channel "
         "COUNT, never from\nmultiplexing one channel — so the C artifact's shared "
         "in-flight slot table has no\nhome to return to. Pool-full is backpressure: "
         "the caller parks with every lock released.",
         size=7.1, color="#7c2d12")
    for i, x0 in enumerate((18.5, 29.0, 39.5, 50.0)):
        busy = i == 0
        box(ax, x0, 23.4, 9.5, 6.6, "#fecaca" if busy else "#fff7ed",
            "#b91c1c" if busy else ISO[1], lw=1.3)
        txt(ax, x0 + 4.75, 28.6, f"worker {i}", size=8.0, weight="bold",
            color="#9a3412")
        body(ax, x0 + 4.75, 27.3,
             "CHECKED OUT\nby thread A\n(verb pending)" if busy
             else "idle,\nin the pool", size=6.8)

    # ---------------- reactor / completion status ----------------
    box(ax, 64.0, 22.5, 34.5, 15.5, "#f3e8ff", PUR, lw=1.8)
    txt(ax, 81.25, 36.1, "THE COMPLETION-SOURCE REACTOR", size=10.0,
        weight="bold", color=PUR)
    body(ax, 81.25, 34.6,
         "★ The MODEL is core-owned and PURE: opaque handles that are never\n"
         "reused (so a stale one is permanently unresolvable rather than\n"
         "re-bindable), four source kinds, and the dispatch 'source S\n"
         "signalled → which process → what to do'. The words 'eventfd' and\n"
         "'epoll' do not appear in the core, and a CI grep gate fails the\n"
         "build if they ever do — in code OR in a comment.",
         size=7.1)
    body(ax, 81.25, 27.2,
         "HONEST STATUS: the OS half is NOT BUILT, and of the four completion\n"
         "pump edges the design names, the shell wires the poll edge and the\n"
         "deferred backstop. The observe→pump edge and the drain edge are\n"
         "still absent, and the backstop still pumps a hardcoded GPU 0.",
         size=7.1, color="#b91c1c")

    # ---------------- the guarantee ----------------
    box(ax, 16.5, 6.0, 82.0, 14.0, "#ecfdf5", "#047857", lw=2.0)
    txt(ax, 57.5, 17.9, "★ THE INVARIANT THE WHOLE SHAPE EXISTS TO BUY",
        size=10.6, weight="bold", color="#047857")
    txt(ax, 57.5, 15.2,
        "A blocking GPU-work verb issued by guest thread A must not stall guest "
        "thread B OF THE SAME PROCESS — in particular B's poll, event-wait and "
        "completion paths.",
        size=9.6, weight="bold", color="#065f46")
    body(ax, 57.5, 13.0,
        "Per-process sharding alone cannot deliver this, because a multi-threaded "
        "guest process is ONE `Proc`, with one lock and one isolate. Three things "
        "compose to make it hold: (1) R1, so A's pending\n"
        "verb holds no lock at all and B's bookkeeping takes microseconds no matter "
        "how long A has been waiting; (2) N workers, so B's own verb does not queue "
        "behind A's on the wire; (3) the completion path is\n"
        "STRUCTURALLY independent of the RM-verb path — it needs no worker, so even "
        "a permanently wedged verb cannot sit between B and its completions. The "
        "mean test asserts exactly this, as an edge, never a clock.",
        size=7.7, color="#065f46")

    fig.savefig(f"{OUT}/l1_diagram_runtime.png", dpi=150, bbox_inches="tight",
                facecolor="white")
    plt.close(fig)


# ===========================================================================
# FIGURE 3 — RmGraph -> projections -> the four planes
# ===========================================================================
def figure_dataflow():
    fig, ax = canvas(18.0, 11.0)

    txt(ax, 50, 97.6, "The data-plane spine — from a protocol fact to a host GPU "
        "operation", size=14.0, weight="bold")
    txt(ax, 50, 94.9,
        "Everything derived is a pure function of the FACTS the guest declared, "
        "never of the ORDER they arrived in — which is what makes single-threaded "
        "scripted testing of multi-vCPU interleavings sound.",
        size=9.3, style="italic", color="#374151")

    cols = [
        (2.0, 21.0, "1 · PROTOCOL FACTS", "#dbeafe", "#1d4ed8",
         "`RmEvent` — an ABSTRACT fact,\n"
         "never a wire struct. Exactly six:\n\n"
         "   Alloc { client, parent,\n"
         "           handle, class }\n"
         "   Dup { src, dst }\n"
         "   SetPageDir { vaspace, pdb }\n"
         "   MapMemoryDma { va, off, len }\n"
         "   Unmap { vaspace, va }\n"
         "   Free { client, handle }\n\n"
         "plus decoded doorbell tokens and\n"
         "decoded pushbuffer methods.\n\n"
         "The wire→fact decode is L3's job\n"
         "and does not exist yet, so today\n"
         "these facts are minted by tests."),
        (24.5, 22.0, "2 · THE RmGraph", "#dcfce7", "#15803d",
         "★ THE SOURCE OF TRUTH.\n\n"
         "A refcounted RESOURCE / HANDLE\n"
         "split: the resource identity is a\n"
         "private monotonic id, so a freed\n"
         "and re-allocated handle is a NEW\n"
         "resource. A `DUP_OBJECT` alias is\n"
         "one more reference, and keeps a\n"
         "resource alive past its origin\n"
         "handle's free.\n\n"
         "PARKED FACTS: a fact naming a\n"
         "handle that does not exist yet is\n"
         "kept, not dropped, and resolves\n"
         "when its target lands. That is\n"
         "where order tolerance comes from.\n\n"
         "Every guest-growable table is\n"
         "capacity-bounded: a flood is a\n"
         "loud refusal, never an OOM."),
        (48.0, 22.0, "3 · PURE PROJECTION", "#fef9c3", "#a16207",
         "`project()` derives, from the\n"
         "graph alone and nothing else:\n\n"
         "· PROCESS BOUNDARIES — the\n"
         "  dup-connected components of the\n"
         "  client graph. A 'process' is a\n"
         "  derived label, never a timing\n"
         "  guess and never a client id.\n\n"
         "· ROUTING —\n"
         "   `by_pdb[(GpuId, Pdb)] → Vas`\n"
         "   `by_vchid[(GpuId, VChid)] → Chan`\n\n"
         "Keys are per-GPU. Identical ids on\n"
         "two GPUs are legal; on ONE GPU\n"
         "they are a loud, contained\n"
         "collision — never a silent\n"
         "wrong-resolve."),
        (72.0, 26.0, "3b · THE CONDEMNED SET", "#fee2e2", "#b91c1c",
         "When an isolate worker dies out of band we retire\n"
         "the process — but the guest never freed anything, so\n"
         "the very next refresh re-derived that component and\n"
         "handed it a BRAND NEW isolate. A guest able to crash\n"
         "its own worker could get a clean sandbox on demand.\n"
         "The mean test found this; it is fixed.\n\n"
         "`refresh` now runs a condemnation pass BEFORE it can\n"
         "mint anything. The key is the component's CLIENT SET\n"
         "— not its process id (minted per derivation, which\n"
         "was the bug) and not its anchor handle (a guest can\n"
         "re-label that). Condemnation is monotone over growth,\n"
         "and clears only when the GUEST frees the client root.\n\n"
         "An op against it resolves FORWARD to a named\n"
         "`Condemned` fault: no reverse lookup was invented in\n"
         "order to produce a prettier error message."),
    ]
    for (x, w, name, fc, ec, text) in cols:
        box(ax, x, 55.0, w, 36.5, fc, ec, lw=1.8)
        txt(ax, x + w / 2, 89.4, name, size=9.8, weight="bold", color=ec)
        body(ax, x + w / 2, 87.6, text, size=7.2)
    for x in (23.0, 46.5, 70.5):
        arrow(ax, x, 73.0, x + 1.3, 73.0, lw=2.4)

    # row 2 — the Proc and its four planes
    box(ax, 2.0, 17.5, 64.0, 33.0, "white", "#111827", lw=2.2)
    txt(ax, 34.0, 48.0, "4 · `Proc` — ONE GUEST PROCESS OWNS ALL FOUR PLANES",
        size=10.4, weight="bold")
    body(ax, 34.0, 46.4,
         "The blast-radius boundary that fixed the C artifact's multi-process "
         "corruption, reused as the concurrency boundary. Two processes share "
         "nothing.", size=7.8, style="italic", color="#374151")

    planes = [
        (3.5, "ADDRESS", "#dbeafe", "#1d4ed8",
         "one `AddressTable` per `Vas`,\n"
         "one `Vas` per `(GpuId, Pdb)`.\n\n"
         "★ FORWARD-POPULATED ONLY.\n"
         "Bound when the guest declares\n"
         "the mapping; read at lookup.\n"
         "Never walked backwards from\n"
         "a VA at execution time.\n\n"
         "A miss is a FAULT. There is\n"
         "no fallback, no most-recently-\n"
         "used guess, no content scan,\n"
         "no 'try the other address\n"
         "space'. The table plays the\n"
         "role of the guest's TLB."),
        (19.0, "EXECUTION", "#fef9c3", "#a16207",
         "`Channel` per vChid, plus a\n"
         "PER-PROCESS record of what\n"
         "has been scheduled — never a\n"
         "device-global one-shot flag,\n"
         "which is what put the C\n"
         "artifact's second context on\n"
         "the wrong runlist.\n\n"
         "`handle_doorbell` is the ONE\n"
         "path that may ever ring a\n"
         "host doorbell, and it gates\n"
         "the working set against the\n"
         "address table first.\n\n"
         "No ungated sibling exists."),
        (34.5, "COMPLETION", "#f3e8ff", "#7e22ce",
         "per-process queue plus the\n"
         "mapped-fence arms.\n\n"
         "Re-delivery is driven off the\n"
         "OWNER'S OWN poll, so a\n"
         "process that polls but does\n"
         "not submit cannot starve when\n"
         "its neighbour goes quiet —\n"
         "the exact shape of a bug the\n"
         "C artifact hit at round 8 of\n"
         "its multi-process campaign.\n\n"
         "The per-GPU drain gate is a\n"
         "TRANSPORT limit, never an\n"
         "OBSERVATION gate."),
        (50.0, "ISOLATE + ARENA", "#ffedd5", "#c2410c",
         "one sandboxed isolate and one\n"
         "disjoint guest-physical arena\n"
         "per `(Proc, GpuId)`.\n\n"
         "Identical guest VAs and\n"
         "identical RM handles in two\n"
         "processes reach disjoint\n"
         "arenas, disjoint host address\n"
         "spaces and disjoint isolates —\n"
         "by construction, not by a\n"
         "check that could be forgotten.\n\n"
         "An arena is released BY VALUE,\n"
         "so releasing one a live\n"
         "process still owns does not\n"
         "type-check."),
    ]
    for (x0, name, fc, ec, text) in planes:
        box(ax, x0, 19.0, 14.5, 25.0, fc, ec, lw=1.5)
        txt(ax, x0 + 7.25, 42.3, name, size=8.8, weight="bold", color=ec)
        body(ax, x0 + 7.25, 40.6, text, size=6.7)

    arrow(ax, 34.0, 55.0, 34.0, 51.0, lw=2.4)
    txt(ax, 35.6, 53.0, "`refresh` — transactional: mutate the graph, re-project, "
        "sync the runtime, roll back on any derivation fault",
        size=7.6, ha="left", color="#374151")

    # row 2 right — entry points
    box(ax, 68.0, 17.5, 30.0, 33.0, "#f9fafb", "#6b7280", lw=1.6)
    txt(ax, 83.0, 48.0, "5 · THE ENTRY POINTS ADAPTERS CALL", size=9.8,
        weight="bold")
    body(ax, 83.0, 45.8,
         "`Gpu::apply(RmEvent)`     the control plane\n"
         "`handle_doorbell`         the ONE ring path\n"
         "`publish_backing`         materialize a range\n"
         "`parse_pushbuffer`        the ONE parser\n"
         "`forward_engine_object`   Case-1 forward\n"
         "`route_control`           Case-1 vs Case-2\n"
         "`arm_fence` / `fence_observed`\n"
         "`deliver_completions` / `poll_completions`\n"
         "`present_scanout`         the display seam\n"
         "`signal_golden_capture`   the ONE forge\n"
         "`reap_retired`            the quiesce point",
         size=7.2)
    body(ax, 83.0, 30.5,
         "Each of the mixed ones is split into a `route_*`\n"
         "half taken under the device read lock and an\n"
         "`exec_*` half taken under the owning process's\n"
         "lock. Each verb-issuing `exec_*` half is split\n"
         "again into plan / execute / commit.\n\n"
         "`signal_golden_capture` is deliberately NOT\n"
         "split: it is typed to the system process, so\n"
         "forging a completion into a user process is\n"
         "unrepresentable, not merely forbidden.",
         size=7.2, color="#374151")

    # row 3 — verbs out
    box(ax, 2.0, 4.0, 96.0, 11.5, "#ffedd5", "#c2410c", lw=1.8)
    txt(ax, 50, 13.3, "6 · WHAT LEAVES THE PROCESS — the unprivileged RM verb "
        "surface", size=10.0, weight="bold", color="#9a3412")
    txt(ax, 50, 10.5,
        "`alloc_vaspace` · `alloc_sysmem` · `map_gpu_va` · "
        "`alloc_channel(vas, engine)` · `alloc_engine_object` · `schedule` · "
        "`ring_doorbell` · `control` · `export_surface` · `free` · `unmap_gpu_va`",
        size=8.6, weight="bold")
    body(ax, 50, 8.6,
        "These are INTENT verbs, not ioctls. Every one must be issuable by an "
        "unprivileged process — an insufficient-permissions error from the host "
        "means \"we forwarded at the wrong layer\", never \"retry with privilege\".\n"
        "The engine tag on `alloc_channel` is load-bearing: dropping it is how the "
        "C artifact put a channel on the wrong runlist and spent days on the "
        "resulting failure.",
        size=7.6, color="#7c2d12")

    fig.savefig(f"{OUT}/l1_diagram_dataflow.png", dpi=150, bbox_inches="tight",
                facecolor="white")
    plt.close(fig)


if __name__ == "__main__":
    figure_system()
    figure_runtime()
    figure_dataflow()
    print("wrote l1_diagram_{system,runtime,dataflow}.png")
