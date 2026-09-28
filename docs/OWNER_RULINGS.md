# Owner rulings — the decisions that govern kayfabe v3 work

**STATUS: LIVE, 2026-09-28.** Every ruling the owner made in the 2026-09-25 … 09-28 working sessions,
with its date, so work can resume from the repository alone. The architecture itself is in
`docs/design/THE_V3_PLAN.md` and `THE_CONSTRAINTS.md`; this file records *decisions* on top of it.
Where a ruling was later refined, the refinement is listed under it. A ruling's date is part of its
citation: ask whether its reason still holds before relying on it.

## A. Standing principles

1. **v3 is a rewrite; never mix planes.** No CPU data plane; no mixing of CPU- and GPU-plane paths.
2. **Host verbs are authored, never forwarded.** kayfabe issues its own unprivileged host RM calls;
   it never replays a guest's privileged control raw.
3. **Completions are host events on an fd — never forged.** ★ (09-26) *A refusal the guest reads as
   "wait over" or "done" IS a forged completion* (`MC_SERVICE_INTERRUPTS`, the sysmembar flush): audit
   non-OK statuses on wait/flush paths first. An entry retired without its work having executed is the
   same defect.
4. **No blocking on a vCPU and none under a lock another thread blocks on.** The one exception: a PRAMIN
   window move = one host map + one mmap, synchronously in the vCPU.
5. **VMM addresses are never guest-chosen or guest-visible.**
6. **§13 unsafe discipline:** no raw VMM pointers in safe code; lengths/offsets validated inside the
   unsafe crates. `kf-cuda` may be a third unsafe crate (09-25).
7. **All GPU families are first-class** (Turing … Blackwell, incl. Hopper). **Derive per die, maintain
   per family:** per-architecture values come from the open guest driver source (ogkm) — generated
   tables, not hand rows; per-die values from unprivileged host controls at VM start.
8. **Compliance principle (09-26):** kayfabe only needs guest *userspace* to work. The guest kernel
   driver is open source, so read what it checks and supply values that satisfy it; prefer deriving
   from source over measuring.
9. **Hostile-guest isolation is the value proposition.** Guest root may only affect isolation *inside*
   the guest, never the host. In-guest isolation matters too: unprivileged guest userspace must not
   reach what the guest kernel protects (09-25 Q7; 09-26 PRIV leaves are withheld from user twins).
10. **Bare metal passes + guest fails ⇒ kayfabe bug.** (09-26) *Always confirm on bare metal first*
    before blaming a box or host.
11. **Don't silently change a constraint — tell the owner.** For open design choices, don't wait:
    run the best-bet **experiment on a sub-branch** (never master), report evidence, let the owner decide
    what lands (09-26).

## B. Specific decisions (2026-09-25)

- **Q7 kernel identity** = RM's `internalFlags` PRIVILEGE stamp (+ the RM-internal handle range).
  Guest-root (ADMIN) channels stay Passthrough.
- **Translated** rings/pushbuffer/USERD live outside the GPGA but inside the VA space; operands still
  reference the GPGA and are never copied.
- **`GPU_PROMOTE_CTX`: stub** — the host twin already holds the real context; golden context is
  guest-kernel-only.
- **Q8 memory plane:** the GPU walker sends only a **diff** against its last snapshot, kept **in vidmem**;
  no checksums; the host kernel is the ledger; **commit-on-ack** (only host-confirmed placements commit).
- **Non-GSP guests:** a future target, not v1 (Windows can force GSP on).
- **Q11:** narrow the ring-collision refusal to the offending leaf — self-harm only, never inducible
  across isolation by unprivileged guest userspace.
- **Per-twin RC** is fine iff unobservable to guest userspace and leaving no state the guest RM doesn't
  expect.
- **Hopper stays supported** (e.g. MEM_OP_D MMU_OPERATION is Hopper's invalidate: serve it, don't refuse).
- **Suite budget** may be raised when measured variance justifies it (it is 180 s since 09-26).

## C. Roadmap (2026-09-26), in order

1. Apps working (the nvkvm-pv CUDA app set; works/fails, not parity).
2. All headless-graphics tests nvkvm-pv passed.
3. Display (scanout), then display apps / a desktop (Mint) that nvkvm-pv ran.
4. In parallel: the Linux guest doorbell module (parity), Blackwell (done: RTX 5080 30/30).
5. Driver matrix — the same driver range nvkvm-pv supports (535 → 610), both driver axes.
   ★ **Refined 2026-09-28 (owner): keep the GPU-architecture axis — the sweep covers every family
   Turing and newer** (host driver × guest driver × GPU arch), not one bench die. Vast VM offers seen on
   2026-09-28: TU116 (GTX 1660 S/Ti) and TU106 (RTX 2060 S), GA10x, AD10x and GB20x in number; **no
   GA100, GH100 or GB10x as VMs** — those rows stay source-derived until such a host is available.
6. Windows guest last.
- ⊘ *Superseded 2026-09-28 by the refinement of item 5 (Turing is on the arch axis, so it runs on
  hardware before the sweep):* Later, if time: **Turing** on hardware (its GSP model is source-derived only).
- **All working, verified work lands on master.** Nothing verified may be stranded on a branch.

## D. Guest doorbell helper module (2026-09-26)

- **Approved for design; security posture accepted:** giving guest *root* write access to the real host
  doorbell page (it can ring any host token) is accepted risk, as in shared CUDA containers — a ring only
  makes the GPU re-read that channel's own GP_PUT. Stock guests keep the trapped path.
- Design specifics (table = one atomic u64 per guest token; always opportunistic; virtio discovery;
  inert on bare metal; BAR1 map immediate / unmap waits for the module's ack; free load/unload order;
  BAR0 offset supplied per family; multi-GPU per BDF): `docs/design/V3_GUEST_DOORBELL_MODULE.md`.
- **Windows:** a legitimate Windows equivalent is not possible today (WDDM rings from the kernel
  driver); possibly not needed — measure doorbells/token on a Windows guest first
  (`docs/design/V3_WINDOWS_DOORBELL_RESEARCH.md`).
- **Open:** how to proceed with the module (build as designed / a paravirtual interface in an optional
  guest driver build / cheaper host-side exits first) is an **owner decision** — see
  `docs/STATUS_AND_HANDOFF.md`.
- **BAR1 doorbell (Hopper+):** must follow where RM places it — built (`V3_BAR1_DOORBELL.md`).

## E. UVM demand paging (2026-09-26)

- Unprivileged-only is believed impossible (host replayable faults need a host-UVM-owned VA space; the
  fault buffer is kernel-privileged; the GPU reaches memory at the guest's VA).
- **A privileged host piece is allowed, for UVM only, and it must not trust the VMM**; the rest stays
  unprivileged and untrusted.
- **Preference is maintainability, not bypass:** prefer a separate kayfabe module on NVIDIA's exported
  interface (nvidia.ko stock) over carrying a fork/patch of nvidia-uvm. The host admin owns the machine.
- Stock UVM read-duplication (`cudaMemAdviseSetReadMostly`) is not a kayfabe mode — kayfabe must *handle*
  it correctly (read-only PTEs, collapse on write): permission bits are now carried.
- **Route (b3 patch vs N4 module) is undecided**; N4 is the best-bet experiment. State and the open
  problem: `docs/design/V3_UVM_DEMAND_PAGING.md` (branch `v3-uvm-n4`) and `docs/STATUS_AND_HANDOFF.md`.

## F. Work practice (2026-09-26)

- **Agents:** each on its own worktree + sub-branch and its own vast box(es); builds on the box; local
  cargo only under a global flock with 2 jobs and a deleted-after target dir (the dev host is shared).
- **Boxes:** untrusted, not guaranteed to persist, no secrets, no executables copied back, evidence
  pushed to git after each run, keep only boxes in use, teardown only by ids you created
  (`scripts/bench/box/README.md`).
- ⊘ *2026-09-27 — a reference update, not a ruling: the allowlist commit the next bullet names as
  `ee35ca4a` is now **`a50265f8`**, the same change. `ee35ca4a` was an earlier (pre-rebase) SHA of it;
  `v3-drivers` has since been rewritten and rebased onto `6ec7ec1a`. It is
  the tip of `v3-drivers`, the only commit there not on master, and it is **still held**; the rest of
  the branch (`8f9bdd14`) went to master via v3-mc20 (`5018bb57`, bar at `c0ef7b75`).*
- **Security-policy changes need explicit owner review before merge** — e.g. the 535/545 capability
  allowlist (commit `ee35ca4a` on `v3-drivers`, held).
