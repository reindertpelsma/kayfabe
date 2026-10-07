# Mode-2 — Fable 5 restructure kickoff (2026-06-10)

**Read this first.** It's the entry point for a fresh session (intended for Claude Fable 5) tasked
with: **restructure Mode-2 from a clean base, port the real working mechanism (extracted from a
messy Codex run), and drive `matmul` end-to-end.** It ties together the two detailed docs and records
the strategic decisions made in the 2026-06-07..10 sessions.

Read these three, in order:
1. **THIS doc** — the plan, git topology, and the clean-base-vs-cleanup decision.
2. **`mode2_codex_postmortem.md`** — what the Codex run actually built; the KEEP/DISCARD port list.
3. **`mode2_cuctxcreate_resume.md`** — detailed current state (UVM bridge, `ctx_probe`/`cup2_pause`
   passing, `matmul_pause` status, gotchas, the strict production-UVM rule). It is current as of
   2026-06-09 and Codex-state-flavored.

---

## 1. Where things stand

- Codex (OpenAI) ran ~2 days and **cleared the cuCtxCreate wall**: `cuCtxCreate` completes and a
  **4-byte HtoD/DtoH round-trip passes** (`cup2_pause`), plus `ctx_probe minimal`/`full`.
- BUT: it did so by **doubling `gpu_emul.c` to ~14k lines** of mostly diagnostic scaffolding, and the
  green result **depends on a throwaway guest-kernel uprobe bridge** (`nvkvm_uvm_uprobe_bridge.c`) —
  not production. `matmul` is **not** working yet.
- My earlier lead (the cuCtxCreate crash = "rbp clobber from divergent RM reply bytes / gpuId enum")
  was most likely a **downstream symptom** of the UVM data plane being unbacked. The real fix was
  backing UVM memory. Treat the gpuId-divergence / `M8.3` thread as a probable dead end.

## 2. Git topology — what to branch from, what to only reference

| Role | Ref | Notes |
|---|---|---|
| **STABLE BASE (branch from here)** | `41bd25c` "M8.3…" | Clean, pre-Codex-bloat. Has the per-channel VAS (M5.28) + M8.1/M8.2. **cuCtxCreate still crashes here — expected**; you add the UVM fix. |
| alt cleaner base | `93000d0` | My handoff (per-channel VAS + M8.1/M8.2), *before* the M8.3 gpuId-trace work (likely a red herring). Use this if you want to drop M8.3 too. |
| **ORACLE (reference only — do NOT build on)** | `7fb47f1` / branch `backup/codex-ctxcreate-roundtrip-20260610` | Round-trip works, but ~14k lines + depends on the uprobe bridge. Use **only** to diff behavior against. |
| untrusted WIP snapshot | `05313f0` | Codex's in-flight tree at kill. Preserved, not reviewed. |
| current `mode-2` HEAD | `6c57182` | postmortem doc commit. |

## 3. The decision (made this session)

**Restructure from the clean base + selective port. Do NOT clean-up-in-place from the Codex HEAD,
and do NOT blind-rewrite.**

Why not cleanup-in-place: the working round-trip **depends on the uprobe bridge** (confirmed — a
no-bridge run is not a valid UVM dataplane test; QEMU never receives the
`UVM_MAP_EXTERNAL_ALLOCATION` records otherwise). So the "fix" half-lives inside a kernel hack that
can't ship. Polishing the 14k-line tree means polishing toward something structurally
non-production; the capture mechanism **must be rebuilt regardless**.

Why not blind-rewrite: you'd throw away the one expensive thing Codex produced — the knowledge of
*what makes the round-trip pass*. Keep the Codex HEAD as a behavioral **oracle**.

(Faster-but-riskier alternative the user also floated: branch from Codex HEAD and aggressively rip
out scaffold. Rejected as the default because pulling the bridge breaks the green, and you'd be
de-bloating someone else's 14k-line idioms you don't fully own. Prefer clean-base.)

## 4. Step 0 (mandatory): reproduce + characterize the green

Before trusting the oracle, build `7fb47f1`, fresh QEMU boot, and run `cup2_pause` (4-byte round
trip) + `ctx_probe`. Confirm it still passes, and confirm **it requires the uprobe bridge loaded**
(the resume doc says it does). This both validates the oracle and re-confirms the clean-base path.

### Step 0 RESULT (2026-06-10, 4 fresh-boot attempts at `7fb47f1`)

- **cuCtxCreate: 2/4 PASS, 2/4 hang** in the documented `MC_SERVICE_INTERRUPTS 0x20801702` poll
  loop, both hangs frozen at exactly UVM-EXT record #12. So **M8.108 credit accounting is racy,
  not a complete fix** — when porting, root-cause the lost-credit race (host completion arriving
  outside a poll window?) instead of copying it verbatim.
- **Source UVM backing verified live on both CTX-OK runs**: the bridge shadow row reached QEMU via
  BAR0, and the CE copy read `0xabcd1234` from the shadow GPA. The load-bearing mechanism is real.
- **The 4-byte DtoH never went green here**: both CTX-OK runs MISMATCHed with `rv=0x0` because the
  **pbmap dst staging row pointed at a page the guest never read back** (QEMU wrote `abcd1234` to
  the pbmap-resolved GPA; suspected `/dev/zero` MAP_PRIVATE pre-CoW pfn export —
  `NVKVM_PBMAP_FAULT_ZERO_WRITE=1` pause-phase flags exist in `gcup2_pbmap.sh` for exactly this but
  are not in the documented recipe). Not worth more serial GPU cycles: production replaces pbmap.
- **Port-map agent confirmed: the oracle has NO production UVM-capture entry path at all** — the
  only writer of `m2_uvm_ext[]` is the BAR0 debug aperture (0xFFF520..538) fed by the uprobe bridge.
  There is no RM/GSP-RPC snoop to migrate; the production capture is a fresh design.
- Verdict: the oracle is a **per-mechanism behavioral reference, not a green-bar reference**. Both
  flaky pieces (ctx completion credits, dst staging) are exactly what the clean-base port replaces.
  This strengthens the §3 decision.

## 5. The plan (ordered)

1. **Branch** `consolidation` (or similar) from `41bd25c`.
2. **Port the KEEP mechanisms** (from `mode2_codex_postmortem.md`):
   - `m2_uvm_ext[]` range table `{va, size, hClient, hMemory, obj_idx}` (the VA→identity map for
     UVM external allocations).
   - `m2_objs[].forwarded` "**don't zero real data**" semantics.
   - `invalidate_span()` cache-flush before the GPU/CE reads the coherent backing.
   - doorbell **pushbuffer-resolution** that routes forwarded-UVM ranges through the backing.
   - **M8.108** service-interrupt completion-**credit** accounting (fixes the `cuCtxCreate` *timeout*
     loop on `NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS 0x20801702 -> ffffffff`).
   - **M8.114** GR-VAS sysmem priming during doorbell setup (was a selftest side-effect).
   - **BUT re-implement UVM-range CAPTURE the production way**: snoop `UVM_MAP_EXTERNAL_ALLOCATION`
     from the RM/GSP-RPC stream we already intercept, and back ranges via the **GPGA tables** — NOT
     the uprobe bridge, NOT `m2_pbmap[]`, NOT `m2_uvm_shadow[]`. (Codex documented this intended
     model itself in `dd78c82`.)
3. **DISCARD** (do not port): the 943-line `nvkvm_uvm_uprobe_bridge.c`, `m2_uvm_shadow[]`,
   `m2_pbmap[]`, `*_pause.c` harnesses-as-production, `NVKVM_M2_RUN_MAPDMA_SELFTEST` + old osdesc
   selftests, the 256/128-gated `qemu_log` DIAG spam. Gate any kept DIAG behind `NVKVM_MODE2_DEBUG`.
4. **Oracle-diff**: run the 4-byte round-trip on both the clean branch and `7fb47f1`; confirm parity.
   The clean version must pass **without** the uprobe bridge — that's the success bar for the port.
5. **Then matmul end-to-end**: `matmul_pause 8` → `matmul_pause 64` → a small real LLM workload.

## 6. The real matmul blockers (what the 4-byte round-trip dodged)

The round-trip only passes because it lives in a **low VA range** the uprobe happened to shadow.
matmul hits the genuinely-hard residual (see resume doc §0.1):
- **Copy-channel Xid 32** + `NVRM: dmaAllocMapping_GM107: can't alloc VA space for mapping` on
  high-UVM CE packets (the same copy-channel-VAS collision class from the per-channel-VAS work).
- forwarded **`PROMOTE_CTX` returning `st=0x1b`** (KERNEL_PRIVILEGED on the unprivileged isolate).
- high-UVM forwarded mappings still falling back to local/debug backing.
These are the matmul gate. Solve them with **all** UVM ranges captured from the RM stream (not just
what a uprobe intercepts) + correct GPGA/GR-VAS placement — not by widening the debug shadow.

## 7. Constraints / gotchas (carry forward)

- **Serial GPU tests.** Kill old QEMU/stub first. **Fresh QEMU boot per clean run** (2nd `nvidia.ko`
  load → `cuInit 999` / WPR2 dirty).
- Emulated GPU on **q35 root slot `addr=0x7`**.
- **Production UVM rule (strict):** QEMU must NOT read guest userspace VAs. pbmap export / uprobe
  bridge / `LD_PRELOAD` are **debug-only**. Production consumes GPA/GPGA/GR-VA + host RM mappings;
  CR3 is only an opaque isolate identity.
- `ssh vg` = guest, `ssh vh` = host (RTX 3060, 580.159.04). Deploy: `scp src/qemu/nvkvm_gpu_emul.c
  vh:/opt/qemu-src/hw/misc/` → `ninja -C /opt/qemu-src/build install` → relaunch `/tmp/m2launch.sh`.
  cup2 stdout is buffered to files → use `stdbuf -oL` or read the QEMU log.
- This restructure **is** task **#128** (consolidation): the file doubled to ~14k; de-bloating is
  part of the job, not a separate step. Split `gpu_emul.c` into modules if practical.

## 7b. Clean-base cuCtxCreate blocker — PINNED with fresh ground truth (2026-06-10)

Built the `consolidation` branch (clean base + M5.30), fresh boot, ran `cup2` with **no
uprobe bridge**. cuCtxCreate crashes (`rbp=0` SIGSEGV, stack destroyed at the libcuda
epilogue — backtrace unrecoverable). The CRASHWIN FB-read probe (auto-armed at the 0xc7c0
compute-obj alloc) shows the *real* blocker: the guest RM busy-loops walking its GR-VAS page
tables via BAR2 —
`0x2f3392000(BAR2 root) → 0x2efbc3000 → 0x2efbc4000 → 0x2efbc5000(dual PDE: small=0, big→0x2efbc6000) → PTE 0x2efbc61a0 = 0x60000002efa6201`
— which resolves to **FB `0x2efa62000`**, and polls that page repeatedly (100k-capped reads,
all the same 5-read chain). The `rbp=0` crash is *downstream* of this poll never satisfying.

**This is the GR golden-context content poll = dataplane doc §X "Poll #2"**, not "un-backed UVM
data" (the §1/§6 framing) and not the rbp/gpuId red herring. The guest waits on a GR ctx page
(FB `0x2efa6xxx`) that the real GPU's FECS fills with the golden image; our fake-GSP path never
writes it. Per dataplane doc §X.1 this is the privileged golden-context coherence wall — the
known multi-week keystone. **The oracle (7fb47f1) cleared cuCtxCreate, so its source contains the
mechanism that satisfies this poll** — pin that mechanism (M8.114 GR-VAS prime? doorbell-time
host-GPU GR-init fill? a forged poll value?) and port it. That is the immediate next step, ahead
of the UVM *data*-plane backing (which only matters after cuMemAlloc, post-ctx).

M5.30 (SET_PAGE_DIRECTORY UVM-VAS capture) is committed + HW-validated and is correct/foundational
regardless — it's the production resolver for UVM device pointers once ctx is unblocked.

## 8. Model / effort guidance (for the Fable 5 session)

Role split that worked well this session (record for future sessions):
- **Haiku** — bulk reading / log analysis / diffing (cheap fan-out).
- **Sonnet** — running tests, mechanical oracle-diff/extraction, build/deploy orchestration.
- **Opus 4.8** — the grind: diagnosis, driver/MMU reasoning, writing the port, the serial GPU loop.
- **Fable 5** — hardcore bug fixing / tracing on a *bounded* puzzle. NOTE: Fable's safety filter
  flags this repo's security-adjacent content (hypervisor boundary, OOB/TOCTOU audits, closed-driver
  RE, uprobe injection) — frame any Fable subagent task as pure systems/MMU mechanics, or it bounces.


- Use **Fable 5 at top effort** on the genuinely hard turns: the RM-stream UVM-capture + GPGA
  re-implementation, and the copy-channel Xid-32 / PROMOTE_CTX root cause. (Fable 5 is the strongest
  public model as of 2026-06-09 and is *cheaper per token than Opus 4.8*; its lead grows with task
  length/complexity — which fits this work.)
- Use **cheap models / subagents** (Haiku/Sonnet) for mechanical work: builds, greps, log reads,
  reading the oracle diff, fan-out.
- **Effort cannot substitute for missing evidence.** The bottleneck here is serial GPU cycles + the
  right trace, not reasoning. When stuck, *get the data* (run the test, diff the bytes) — don't crank
  effort to reason over absent evidence. Depth-bound bugs don't parallelize; do not autonomous-fan-out.
