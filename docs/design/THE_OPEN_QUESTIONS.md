# What is still open

**STATUS: LIVE, 2026-09-20 (w821).** Everything the design does **not** decide, with what each
would cost to close. ⊘ Nothing here is a placeholder for work in progress — these are the places a
reader should push, because they are the places I would.

---

## 1. Open, and cheap to close

| # | question | what closes it | cost |
|---|---|---|---|
| 1 | **Does the page-table walker survive a hardened sandbox?** The GPU-side walker runs in-process; under a sandbox profile that denies process spawning, the device nodes must already exist. That is a host prerequisite either way — but it is unmeasured | one CI test: initialise the driver API and run one launch under exactly the sandbox string a production manager uses | an afternoon |
| 2 | **Is the doorbell-table scan really sub-microsecond?** The decision to delete the hint ring rests on it, and the number is **arithmetic** | a microbenchmark on the bench box, warm and cold | an hour |
| 3 | **Do two clients mapping the same usermode object land on one guest-physical page?** This decides whether *"one doorbell page"* is literally true on Hopper+ | read the mapping path, or observe two clients in a guest | a morning |
| 4 | **Can both the BAR0 and BAR1 forms of the usermode object be live at once in one guest?** Both are legal on Hopper+ | the same read | with #3 |

## 2. Open, and genuinely undecided

| # | question | why it is hard |
|---|---|---|
| 5 | **Windows beyond consumer Turing.** The default is **per-SKU** by construction — the policy takes device and subsystem id, a workstation/server flag, and the driver mode. We measured **one corner** | the deciding logic is stripped from the published source. ⇒ It is a **measurement programme**, not a reading exercise: a Windows box per SKU class |
| 6 | **The hand-maintained overlay.** Read/write-only-ness generates from the published access codes; **write-1-to-clear, trigger and data-port semantics do not** — the pending register and its enable port carry the *same* code | ~a dozen names per family, hand-written, and **the build must fail when a register has no entry**. ⚠ It is small, and it is the one place a per-die fact is maintained by hand rather than derived |
| 7 | **The line target.** ~50 k hand-written is reachable only if generation holds and the served-control set stays near 60; an adversarial re-count puts the all-in figure at **70–90 k** | ⊘ These are different denominators, not competing claims. The number that settles it is whether a second driver version adds **descriptor rows or code** |

## 3. Deliberately not decided — deferred with a reason

| # | item | why it is deferred, not open |
|---|---|---|
| 8 | **The no-GSP plane** (a guest whose firmware is disabled — which is the **default** on consumer Windows) | it has a different oracle, a different floor, and a bounded ceiling at Ampere. ★ It is **additive reach**, and letting it shape the trap path or the chip descriptors now would make both worse |
| 9 | **Display** | NVIDIA ships a displayless class; it is a later phase and not on the path to compute |
| 10 | **Peer-to-peer between GPUs** | a third leaf verb, fully specified in the design, built after single-GPU works |
| 11 | **Interrupt delivery** | deferred **only because the things that need it poll**, and only on condition that a blocking-sync arm enters the suite so the gap is **red rather than silent** |

## 4. ⊘ Decided the other way — what we give up, and why it is worth it

★ These are not gaps. They are refusals, and a reader should attack them as choices.

| we refuse | we give up | why that is right |
|---|---|---|
| **A guest IOMMU** | guests configured with one; eventually Windows DMA protection | every guest address we consume resolves through a guest-physical-keyed layout, so a guest IOMMU is bypassed **by construction**. Supporting it means a translation layer on every consumer **including a GPU kernel that cannot call the VMM**. No CUDA or LLM guest needs one |
| **Broadcast device groups** | multi-GPU SLI-style configurations | it is the only case that produces *"one VA space, many page-directory bases"*, and CUDA never uses it. Refusing it removes a whole class from the address model |
| **Fault recovery** (we deliver faults; we never recover) | nothing a guest can reach | the recovery registers are **host-kernel-owned and unimplementable from userspace**. ⇒ The host driver recovers our twin and we tell the guest through the mechanism it already has |
| **Guest-chosen pointers in host calls** | the memory class that carries one | it would hand the host driver an address the guest chose. ★ Our host verbs take no guest flag word either — the same rule, applied to bits instead of pointers |
| **Emitting the firmware bug-check event** | nothing | it crashes the host OS on Windows. **We are the firmware.** Any path that could induce us to emit it is a guest-crash primitive |
| **Pre-Turing, on this plane** | older hardware | there is no firmware processor to impersonate below Turing. ⊘ Not a scope choice — the architecture does not exist there. The no-GSP plane is what would reach it |

---

## 5. ⚠ The three things most likely to be underestimated

Not open questions — **predictions**, recorded so they can be checked later.

1. **Teardown and re-init.** A guest crash, a forced reboot, a kexec and a machine reset never run
   the driver's unload path, and the stock driver **refuses to boot while the protected firmware
   region is still up**. ⇒ Every gate should be run **twice in one process**. A plane that works on
   the first init and not the second does not work.
2. **Error propagation.** Named refusals are load-bearing — but what the *guest* does with each one
   is modelled nowhere, and `NOT_SUPPORTED` is a status the driver **forgives**. ⇒ A wrong refusal
   does not fail; it runs on.
3. **The served-control set.** It is the budget's weak point and the surface most likely to grow
   under contact with a real workload.
