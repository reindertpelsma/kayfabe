# Release cleanup ledger

**STATUS: LIVE, 2026-10-16.** One running list of debt to clear before release, and of what the owner needs to understand and be able to defend.
Rule: append here when you create debt or leave a shortcut; do NOT clean up now (the design is still moving). Each entry: what, where, why it exists,
what clearing it means, who decides. Newest at the bottom of each section. The month of cleanup is also the owner's month of making the code their own:
section B orders the reading by risk.

## A. Debt (to clear at release)

| # | Item | Where | Why it exists | Clearing it means | Decides |
|---|---|---|---|---|---|
| A1 | Owner Windows flag set (19 `KF3_*` flags incl. measurement-only `KF3_DISPLAY_CTRL_PROBE`, `KF3_PREEMPT_BIND_PROBE`, `KF3_*_PROBE`) | production Windows launch profile; `docs/design/V3_FLAG_INVENTORY.md` | behaviour needed on Windows was added as flags during bring-up | each flag hardwired as default behaviour or removed (flag policy: no flag may be required to boot; measurement flags never required) | owner |
| A2 | Experiment/scratch branches that must NEVER merge | `claude/perf-grain-experiment-20261015`; scan-off modes 1/2/3, `KF3_BAR1_STREAM_LOAD` on `claude/perf-lifecycle-measure-20261016`; `KF3_FULLMAP_EXP` | measurement only | delete or archive tag after the lessons are folded into docs | claude |
| A3 | Superseded / uncorrected docs | `V3_USER_RING_SCAN_RACE.md` section 0 still describes the withdrawn `Poll` fix (banner only in section 4); other STATUS lines | corrections were recorded where found, not folded above their parents | fold corrections above the text they correct; mark SUPERSEDED in the superseded text; archive moves | claude |
| A4 | Pre-v3 `kayfabe-*` crates | PR #5 (`claude/archive-prev3-20261015`, -117k lines) | frozen reference tree | merge PR #5 after review | owner |
| A5 | Local branches/worktrees | dev box (145 worktree entries; many stale), host `/data/*` | one worktree per agent | prune after verifying pushed | claude |
| A6 | Codex sweep branches | `codex/sweep-*` (1,100+ files, 340k+ inserted lines, mostly evidence) and draft PRs 7-20 | campaign evidence was committed beside code | split into small code PRs against `claude/gpu-types-20261015`, evidence as text, raw logs off-repo | owner + codex |
| A7 | Open owner decisions | D-1..D-3 (`V3_GPU_TYPES_20261015.md`), D-4 caps row keyed on die, D-5 Turing R34 policy, D-6 diagnostic oracle page | wait on owner | decide, then close the PRs that depend on them | owner |
| A8 | Provenance | `docs/design/*PROVENANCE*`, history | policy: RE small and private, no RE-derived values as fact | history sweep at release; lawyer review; no references to third-party RE material | owner |
| A9 | Secrets / push protection | GitHub settings, hooks | history scan 2026-10-16 clean | enable GitHub secret scanning + push protection; pre-push scan hook; revoke old OAuth app authorization | owner |
| A10 | Rootless / "delete one folder" claim | `traces/rootless_audit_20261016/` (audit running) | unproven until tested | keep out of README until the audit passes; then document device-mode requirements | claude |
| A11 | Host patch tier infrastructure H0 | `V3_HOST_PATCH_LIST.md` | not started; ships with the first patch | runtime probe, DKMS shape, manifest, fallback gating | owner |
| A12 | Lifecycle (Scan/Pointer/Window) | `claude/win-lifecycle-p1b-20261016`; `V3_USER_RING_LIFECYCLE_20261016.md` | measured +97% D3D11 / +56% GL upper bound; implementation under review | finish phase 1b, second review, hardware validation, then phase 2 (inline) / 3 (CE gather) | claude, owner |
| A13 | Windows pushbuffer/ring reads that still use bulk MMIO | `crates/kf-linux-raw` window copies, userpump window path | owner rule: MMIO only for small scattered words; bulk reads via CE | phase 3 batched copy; MOVNTDQA only as documented fallback | claude |

## B. Defend this: reading order for the owner (highest risk first)

1. **The hostile-guest boundary.** Everything a guest byte can reach: `kf-qemu` BAR0 trap handlers (must only post a write), `userpump`/`userrelay`/`user_scan` (guest ring and pushbuffer
   parsing), `kf-mem` page-table mirroring and the VA manager, `kf-rm` GSP RPC emulation (which controls are served, refused, or authored). Rules: host actions are authored from
   unprivileged RM verbs, never forwarded from guest bytes; any guest-reachable unbounded read or emit is a security bug.
2. **All `unsafe`.** Only in `*_unsafe.rs` of `kf-linux-raw`, `kf-qemu`, `kf-cuda`: mappings, volatile windows, ioctls, KVM. Read each block's safety comment against the code.
3. **The no-block rule.** Nothing blocks a vCPU or a lock a vCPU takes: `kf-trap` (BUSY/PENDING CAS model), the doorbell path, the drainer, the worker loop (`V3_USER_RING_EVENT_LOOP_20261014.md`).
4. **Per-die constants.** "Derive, never capture": `kf_rm::consttab`, the caps derivation, the constant matrix; which rows are measured on which die and driver.
5. **The decisions that are yours** (so you can defend them without me): `docs/OWNER_RULINGS.md` (binding, dated), the doorbell contract (weak; no GP_PUT polling; no timer carries
   correctness), pushbuffer integrity is not a security boundary for USER channels, MMIO only for small scattered words, vidmem pushbuffers out of scope on the migratable route,
   stock host needs no patches (patched host is an optional better tier), rootless deployment as the goal.
6. **Evidence, not claims.** `docs/STATUS_DETAIL.md` and `traces/`: every status claim names its revision, box and evidence; bench claims carry the source revision.

## C. Defence notes to write during the month (one per subsystem, 1 page each)
What it does, the invariants it relies on, the owner ruling or measurement behind each, what would break it, how to test it. Start with the lifecycle (it is the freshest and the model/tests
already enumerate its invariants I1-I8).

## D. Release gate: correctness and reliability checklist (owner §AK, 2026-10-17: a checkmark that must hold; explicit refusal over undefined behaviour; crash loudly, never silently wrong)

Each line must be demonstrably true at release, with the evidence named next to it. Append items; never delete one without the owner.
- [ ] Every guest-reachable decision (ring entries, pushbuffers, registrations, RM controls) has a NAMED refusal counter and log line; unknown inputs are refused, not guessed. Inventory of the refusals with their tests.
- [ ] No known path to a silent wrong result: stale-slot replay at a ring switch (lifecycle D5) detected or proven impossible; cursor semantics at the switch measured (P0 a); MOVNTDQA/WC reads rejected until the stale-line risk is tested.
- [ ] Hangs are failures: every stuck state has a loud counter; no timer repairs a state (§AE); the counted 50 ms backstop never fires on real work (counter ~0 on the app matrix).
- [ ] Containment measured: the blast radius of a channel reset on other VMs (M1-M3 in `V3_CHANNEL_RC_SCOPE`) is measured and a loud failure demonstrably stays inside the guest's own channel/VM.
- [ ] Assumptions with explicit-failure semantics (P1 the 5080 binds once; P2' a GP_PUT is a packet boundary): counters read zero over the whole app matrix (Heaven, GravityMark, Minecraft, Edge, Blender, CUDA lanes) and the unsupported case is documented with the error it produces.
- [ ] Supported-workload matrix: zero unexplained failures; failure RATES on supported apps tracked; each unsupported case listed with its error text.
- [ ] Every fallback is a clean mode (complete, named, own tests, §AE); no shipped flag trades correctness for speed; research/sloppy builds never ship.
- [ ] Mutation checks: every invariant is guarded by at least two tests; the real production function is exercised GPU-free (not a transliteration); independent hostile-guest review of each relay change; Linux gates 9/9 + gate 10, fast 30/30 and broker lane on the exact revision.
- [ ] Evidence, not claims: every status claim names its revision, box and evidence (`docs/STATUS_DETAIL.md`, `traces/`).
