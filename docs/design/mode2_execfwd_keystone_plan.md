# Mode-2 execution-forward keystone — implementation plan (2026-06-10)

Branch: `consolidation`. Goal: get the host GPU to actually RUN the guest's GR context-init work so a
REAL completion fires, the guest's `MC_SERVICE_INTERRUPTS` poll is satisfied for real, and
`cuCtxCreate` completes — then drive `matmul` end-to-end. No faking; verified by numeric correctness.

This plan follows the governing rules in `mode2_cuctxcreate_resume.md §0.3` (map-vs-stub) and §0.7
(real completion via reused Mode-1 #127 poll), and the user's interrupt principle: *userspace polls,
not interrupts → for any op a real host GPU would interrupt on, QEMU polls the host fd and forwards.*

## Status entering this plan
- ✅ M8.4: cuCtxCreate `rbp=0` crash FIXED + verified (GR-alloc reply rbp-restore).
- ✅ M5.30: UVM/GR VAS root captured from `SET_PAGE_DIRECTORY` (0x801813) — the missing PDB source.
- ⛔ Blocker: `nvkvm_m2_exec_doorbell` (M5.9) fires 0× for the GR channel because
  `nvkvm_m2_populate_cvas` → `nvkvm_chan_own_pdb` returns 0 (GR-VAS root not resolvable). So the GR
  work is never mapped into the host VAS, never run, never completes → `MC_SERVICE_INTERRUPTS` spins.

## Design decision: authoritative PDB from the instance block (not the snoop chain)

The page-table CONTENTS live in our FB/GPGA (CPU-RM writes PDEs/PTEs via BAR2 — incremental MMIO,
always current). Only the ROOT pointer is missing, because in GSP-client mode the channel instance
block's `NV_RAMIN_PAGE_DIR_BASE` is written by GSP, not the CPU-RM — and we are GSP. Therefore:

**When we capture a VAS root (SET_PAGE_DIRECTORY 0x801813, and/or VASPACE_COPY_SERVER_RESERVED_PDES
0x90f10106), write it into the emulated channel instance block's PAGE_DIR_BASE field** (the GSP job),
and make PDB resolution read it from the instblk (as `nvkvm_bar2_translate` already does for BAR2).
This gives one authoritative source of truth and removes the fragile `client→m2_devvas→chan_vas[]`
lookup that returns 0. Keep `chan_vas[]` as a fallback during bring-up.

## Steps (each = a commit + fresh-boot verification; m2exec-gated; DEBUG logs gated)

### Step 1 — Resolve the GR-VAS root authoritatively (unblock populate_cvas)
1a. Add one DEBUG log in `nvkvm_chan_own_pdb` dumping the resolution chain (chan_client, matched
    m2_devvas vas, chan_vas pdb) so we SEE why it returns 0 for 0xc1d00003 (timing vs absence).
1b. On `SET_PAGE_DIRECTORY` (M5.30) capture, also write `physAddress` into the owning channel's
    emulated instance block `PAGE_DIR_BASE` (and record aperture). Add a `nvkvm_chan_instblk_pdb()`
    that reads it back (mirror of `nvkvm_bar2_translate`'s instblk path).
1c. Make `nvkvm_chan_own_pdb` try, in order: instblk PDB → M5.30 captured root for the client's VAS →
    existing chan_vas[] scan. Return the first that yields a valid walk.
- **VERIFY:** fresh boot, `populate_cvas` no longer logs `no own PDB`; it logs `runs=N backed=M` with
  M>0 (leaves enumerated + backed). No regression to boot/cuInit. COMMIT.

### Step 2 — Confirm the GR working set is mapped into the host VAS (link 2)
2a. With populate_cvas running, confirm the GPGA double-mmap (`nvkvm_m2_leaf_flush` →
    `nvkvm_m2_gpga_obj`/`back_and_map`) maps the GR ctx buffers + GPFIFO + pushbuffers + completion
    semaphore into the host channel VAS at the guest VAs (st=0x0, not 0x51 self-promote collision).
2b. Cross-check against the M5.31 PROMOTE_CTX buffer list (MAIN/PATCH/etc.) — every referenced leaf
    resolves in the host VAS.
- **VERIFY:** QEMU log shows each GR leaf mapped st=0x0; no `dmaAllocMapping` faults for GR. COMMIT.
  (Do NOT ring yet.)

### Step 3 — Host os-event plumbing: reuse Mode-1 #127 poll (link 5/6, before ring)
3a. Ensure the guest's GR-completion OS-event (NV01_EVENT_OS_EVENT, snooped fn=103 class 0x0079) is
    forwarded to a host eventfd via the isolate (Mode-1 already does this).
3b. Arm that host eventfd in the isolate poll set via `nvkvm_isolate_poll` (the #127 ABI). On
    `ISOLATE_RESP_POLL_EVENT`, deliver to the EMULATED guest via GSP `POST_EVENT` (nvkvm_gpu_emul
    M8.38) — NOT VQ_EVT (stock guest has no nvkvm module).
3c. RACE-GUARD (user requirement): the host eventfd must be level-readable until consumed so a
    completion that fires between arming and ppoll is not lost. Fable-verify this property + that
    POST_EVENT delivery can't drop a wake (mirror #127's "re-arm + re-fire" recovery).
- **VERIFY (no ring yet):** arming + a manual host signal delivers a POST_EVENT to the guest (unit-
  style). COMMIT.

### Step 4 — Ring the host doorbell (link 3) — THE WEDGE-RISK STEP, gate it
4a. Gate the ring on "GR working set fully mapped for this channel" (step 2 complete) — a premature
    ring faults the host GPU → cuInit 999 / host wedge.
4b. On the guest GR doorbell: translate vChid → host work-submit token, write the host USERMODE
    doorbell (primitives exist: M5.8 doorbell_setup). Keep `chan_execute` faking OFF for GR under
    m2exec so a green can ONLY come from the host.
- **VERIFY:** `ssh vh nvidia-smi` shows real GR utilization during the run; the completion semaphore
  transitions 0→nonzero by a GPU write (CRASHWIN), NOT by QEMU. Host not wedged. COMMIT.
  Recovery if wedged: `rmmod nvidia_uvm nvidia_drm nvidia_modeset nvidia; modprobe nvidia` (or
  `vastai reboot instance`).

### Step 5 — MC_SERVICE_INTERRUPTS satisfied for real → cuCtxCreate completes
With steps 3–4, the host completion fires → #127 poll → POST_EVENT → guest services the interrupt →
`MC_SERVICE_INTERRUPTS` returns serviced → cuCtxCreate returns.
- **VERIFY:** `ctx_probe`/`cup2` prints `CTX OK` with NO uprobe bridge, NO pbmap, NO credit-shortcut.
  COMMIT. (This is the real cuCtxCreate milestone.)

### Step 6 — matmul end-to-end + correctness gate (the legitimacy proof)
`matmul_pause 8` → `matmul_pause 64` → small LLM. The result must be NUMERICALLY CORRECT and host
nvidia-smi must show util. Correct = every simulate we did was legitimate (kernel-internal only);
wrong/idle = something was faked → revert per §0.3. COMMIT the milestone.

## Guardrails (carry the whole way)
- Each step: committed, builds, fresh-boot no-regression, m2exec/DEBUG-gated, deleted if wrong.
- Serial GPU only; kill stale QEMU/stub; fresh boot per clean run; stage test files AFTER boot.
- NEVER accept a green guest log alone — verify real host work (nvidia-smi util + GPU-written sema).
- If a fix would fake a value the guest userspace consumes, STOP (slop signal) and surface it.
- Use Fable 5 for bounded byte/ABI/race analysis; verify its output against hardware before building
  (its byte-diff was right; its open-ended source-trace once over-committed to a wrong theory).

## PROGRESS LOG (2026-06-10, autonomous run)

- **Step 1 DONE (with caveats):** M5.32 — `chan_own_pdb` now also tries the channel's own
  `hVASpace` vs `chan_vas[]` (M5.30-populated) + the instblk PDB. HW: `populate_cvas` resolves
  `pdb=0x3114000` and walks **26 leaves** (was `no own PDB` bail). Two caveats:
  - **Flaky:** resolution is timing-dependent — some boots the VAS isn't captured before
    `populate_cvas` runs → `no own PDB` again. **TODO: make deterministic** by having fake-GSP write
    the captured SET_PAGE_DIRECTORY root into the channel instance block (`PAGE_DIR_BASE`, RAMIN+0x200)
    so `chan_pdb` is always authoritative (Step 1b, not yet done).
  - **`backed=0` is BENIGN:** the GR working set is **already mapped** at `0xc7c0`-alloc time
    (M6.5 sysmem leaves return `st=0x51 ALREADY-MAPPED`; M7 R2 vidmem `gpu_mapped=1 st=0x0`). So
    Step 2 (map the working set) is effectively already satisfied — NOT the blocker.
- **Real frontier = Step 4 (the RING).** The host GPU has the buffers mapped but is **never told to
  run the work** — `exec_doorbell` doesn't ring → `nvidia-smi` shows **0% util** → no real GR
  completion → `MC_SERVICE_INTERRUPTS` hangs (or flakily self-terminates into a HOLLOW `CTX OK` with
  no compute). cuCtxCreate "passing" today is a hollow pass; the real proof is matmul, which needs
  the host to actually execute. So the next move is Steps 3+4: poll the completion (reuse #127) and
  **ring the host doorbell** (wedge-risk — gate on working-set-mapped, keep `chan_execute` faking OFF).
- **M8.4 (crash fix) remains solid + verified.** M5.30 + M5.32 committed.

## PROGRESS LOG 2 (2026-06-10, autonomous run cont'd)

- **Step 1b DONE:** `populate_cvas` now returns success; the caller only latches `.populated` on
  success, so it retries across doorbells until the async-captured GR-VAS root arrives. HW: resolves
  `pdb=0x3114000` deterministically (no more flaky `no own PDB`). Committed.
- **Step 4 frontier LOCALIZED (the real keystone):** the ring (M5.22) fires, but the host GR channel
  cannot run:
  - `M5.25 GPFIFO_SCHEDULE` on the GR TSG `0x5c000049` (client 0xc1d00003) → **st=0x57 =
    NV_ERR_OBJECT_NOT_FOUND** — the host TSG handle isn't found (never forwarded/created, or wrong
    handle translation). CE scrubber TSGs schedule fine; the GR compute TSG does not.
  - GR channel **host USERD `put=0 get=0`** — no guest work is bridged to the host channel (the
    GP_PUT work-submit bridge is missing), so even a successful schedule + ring would be a no-op.
  - Net: 0% GPU util, no real completion, cuCtxCreate hangs (or flaky hollow pass).
- **Two concrete sub-problems for the next session (both required for real execution):**
  1. **TSG-handle forwarding/translation** so `GPFIFO_SCHEDULE` on the GR TSG resolves (no
     OBJECT_NOT_FOUND). Investigate: is `c->tsg` (0x5c000049) a guest handle passed without
     translation, or a host handle that was never allocated? Cross-check against the per-channel CVAS
     TSG (0x5c000012) — the schedule may be targeting the wrong TSG. (Good Fable/source task once the
     handle lineage is dumped.)
  2. **GP_PUT work-bridge:** propagate the guest channel's GP_PUT into the host channel's USERD
     (0x8C) so the host GPU sees `put>get` and fetches the (already-mapped) GPFIFO entries. The
     GPFIFO/pushbuffers are mapped (M6.5/M7 R2); the doorbell rings; only the USERD GP_PUT propagation
     is missing.
  Then: host runs GR work → real completion on a host eventfd → reuse #127 poll → POST_EVENT →
  MC_SERVICE_INTERRUPTS satisfied → cuCtxCreate for real → matmul gate.
- **Headline unchanged:** M8.4 crash fix verified; the rest is now a precisely-localized execution-
  forward problem (TSG schedule + GP_PUT bridge), not a fog.

## PROGRESS LOG 3 (2026-06-10) — GP_PUT bridge done; LAYER-2 is the real keystone (RESUME HERE)

Done this run (committed): M5.32 Step-1b (deterministic populate_cvas), M5.33 (GP_PUT bridge:
guest `c->gp_get` → host USERD `+0x8C` before the doorbell; + skip redundant GR-TSG reschedule).
Host-VERIFIED SAFE (no wedge). But two layers remain:

- **Layer 1 (cuCtxCreate-specific):** the compute channel submits NO sustained guest GPFIFO work
  during cuCtxCreate (`gp_put` transient 1→0) — the golden ctx is GSP/FECS-internal. So the GP_PUT
  bridge is for MATMUL, not ctx. cuCtxCreate's `MC_SERVICE_INTERRUPTS` wait is a GSP-side completion
  (handle later: simulate-per-rule, or poll a host equivalent per the interrupt principle).
- **Layer 2 (THE KEYSTONE — do this first, plan item A):** even the CE scrubbers, which DO have work
  (`hostUSERD put=30`, TSG scheduled `st=0x0`), show `get=1` and **0% GPU util** — the host GPU is
  **not executing a scheduled channel that has queued work**. This is beneath the GP_PUT bridge and
  is the load-bearing nut: nothing real runs until the host executes forwarded work.

### LAYER-2 investigation plan (A) — resume here, hand tracing/precise-code to Fable, keep oversight
Suspects, in priority order (each cheap to check, mostly one boot + host dmesg):
1. **USERD identity mismatch:** is the host USERD page the GPU actually reads the SAME page we write
   via `m2_chanbuf[].qva` (+0x8C)? Verify the double-mmap target equals the host channel's real
   instance-block USERD pointer (not a stale/duplicate mapping). Fable: trace `nvkvm_m2_back_channel_userd`
   (M5.23) — does the qva map the host channel's RAMFC/USERD that the host GPU's runlist reads?
2. **Pushbuffer faults:** the GPFIFO entries point to pushbuffer VAs — are they all mapped+valid in
   the HOST channel's VAS? A bad pushbuffer VA → host MMU fault → channel stalls (get frozen). Check
   host `dmesg` for Xid 13/31/etc. right after a ring. (We map via M6.5/M7 R2/M5.24 — verify coverage
   of the CE scrubber's pushbuffers specifically, since those have real work.)
3. **Channel not actually runnable:** TSG bound to a runlist? channel in error/disabled state? Check
   host dmesg + read host channel state. The schedule returned st=0x0 but binding may be incomplete.
4. **Wrong doorbell token:** is `c->host_token` the correct work-submit token for THIS host channel
   (NVC36F GET_WORK_SUBMIT_TOKEN per channel), or are we ringing one token for all?
Method: pick the CE scrubber channel (gpfifo 0x121010000, has put=30) as the test case — it has real
work, so if the host runs it, `get` advances + util>0 + host writes its sem. Ring it, watch host
`dmesg` (Xid?) + host USERD `get` + nvidia-smi util. Wedge recovery: `rmmod nvidia_uvm nvidia_drm
nvidia_modeset nvidia; modprobe nvidia` on vh, or `vastai reboot instance <id>` (key in memory
vastai_credentials; find instance id via `vastai show instances`).

## PROGRESS LOG 4 (2026-06-10) — LAYER-2 KEYSTONE CRACKED (M5.34 per-client VA dedup)

Found the keystone root cause with fresh HW data + fixed it. **`m2_va_seen()` deduped on VA
alone, but host VASpaces are PER-CLIENT** (`m2_devvas[]` → {dev,vas} by client). A pushbuffer VA
(`0x120000000`) mapped into client `0xc1d0000a`'s VAS was marked "seen" and SKIPPED for the CE
scrubber's client `0xc1d00001` → the scrubber's host VAS lacked that page → **Xid 31 MMU FAULT_PTE
@ 0x120000000 on GR0_PBDMA0** → channel halt (get frozen at 1, 0% util). That IS the "host won't
run a scheduled+queued channel" keystone.

**Fix M5.34 (committed 1d5d6af):** key the dedup on `(client, va)`. All 6 call sites thread the
client (`s->chan_client` ×3, `a->client` ×2, `grc`). Each client allocs a fresh OS_DESCRIPTOR per
map + has its own VAS, so placing the same guest GPA into two VASes is safe + WB-coherent.

**HW-VERIFIED (fresh boot, cup2):** page `0x120000000` now PLACED into the scrubber's VAS
(`M5.19 fwd-map ... client=0xc1d00001 -> MAPPED`, `M6.5 back_sys ... PLACED`); **host MMU fault
GONE**; the host GPU EXECUTES the forwarded CE work — guest UVM observes the completion semaphore
advance (`completed_value 0x100000054`, low dword `0x54` == queued ops `put=84`). The keystone
("host runs a scheduled+queued forwarded channel") is cracked.

### TWO next-layer problems surfaced (fresh data):
- **Problem A (the matmul-critical one): GR/compute channels never become schedulable.** Host
  dmesg, BEFORE any Xid: `NV_ERR_INVALID_OBJECT_HANDLE (0x33) from vaspaceGetByHandleOrDeviceDefault
  (pClient, hParent, hVASpace) @ kernel_channel_group_api.c:224 ← AllocWithSecInfo(KEPLER_CHANNEL_
  GROUP_A) @ kernel_channel.c:381`. The host **TSG (channel-group) alloc fails** because the
  `hVASpace` we forward is invalid in the host client → TSG never created → `GPFIFO_SCHEDULE` on the
  GR TSG returns `st=0x57 OBJECT_NOT_FOUND` (client 0xc1d00003, TSGs 0x5c00003b/0x5c000049). This is
  the real blocker for compute. Fix path: trace shadow_fwd's KEPLER_CHANNEL_GROUP_A (fn=103) alloc —
  is `hVASpace` a guest handle passed untranslated, or a VASpace never created in the host client?
- **Problem B (CE scrubber residual): completion-semaphore value incoherence + Xid 32.** Host writes
  the sema with bit-32 set (`0x1_00000054` vs guest-expected `0x54`, delta exactly 2^32) → guest UVM
  assert (uvm_channel.c:205 / uvm_gpu_semaphore.c:776, non-fatal, guest continues). Plus Xid 32
  (corrupted pushbuffer stream) on the CE sub-channel. Deeper semaphore-forwarding coherence; may be
  scrubber-host-internal-specific. Revisit after Problem A (GR channel may behave differently).

**NEXT: Problem A (GR TSG hVASpace).** It's the matmul path + a concrete handle-translation bug with
an exact host-kernel call site. Hand the precise source-trace to Fable, anchored in the dmesg.

## PROGRESS LOG 5 (2026-06-10) — Fable reframe of "Problem A" (VERIFIED vs HW)

Handed Problem A to a Fable subagent (deep multi-file trace); it corrected my framing, and I
verified its load-bearing claims against the live host trace:
- **GR/compute scheduling is NOT broken.** The GR compute TSG `0x5c000012` schedules `st=0x0 OK`
  (M5.8 doorbell). The `0x57 OBJECT_NOT_FOUND` GPFIFO_SCHEDULE failures are only on the **COPY2/COPY3**
  engine TSGs (`0x5c00003b` eng=0xb, `0x5c000049` eng=0xc) — host physical-RM rejects those (likely
  RTX-3060 LCE/runlist topology vs the emulated GA106; needs host RM introspection; may not matter
  for matmul). NOT the compute path.
- **The dmesg `kernel_channel.c:381` / `INVALID_OBJECT_HANDLE 0x33` asserts are non-fatal and not the
  GR compute path** (RM auto-TSG for bare channels parented to a device, broken device-default VAS via
  an untranslated NV0080 `hClientShare`). Benign for now; optional cleanup = translate hClientShare on
  the NV0080 device alloc in shadow_fwd. So "force GR TSG hVASpace" was the WRONG fix — dropped.
- The `status=0x33` SHADOW allocs in THIS run are classes `0xc574` + `0x0079` (NV01_EVENT_OS_EVENT),
  i.e. the COMPLETION-EVENT registration path failing on the host — ties into MC_SERVICE_INTERRUPTS /
  completion delivery (per the map-vs-stub rule + #127 reuse, we likely should NOT forward the host
  NV01_EVENT_OS_EVENT alloc at all; deliver via POST_EVENT instead — revisit in Step 3).

### REVISED next-thread priority (after M5.34 cracked the keystone mapping):
1. **Problem B — CE-scrubber completion-semaphore incoherence (freshest, on live path).** M5.34 made
   the host run the scrubber; guest UVM now asserts `completed_value 0x1_00000054 > queued 0x54`
   (delta EXACTLY 2^32) + a `0x1e -> 0x1_00000001` jump, plus Xid 32 (corrupted pushbuffer) on the CE
   sub-channel. Signature = host writes the sema with the HIGH dword set — host channel's own
   semaphore progression/width differs from the guest's. Investigate the scrubber completion-sema
   forwarding (what VA, what value the host writes, 32 vs 64-bit release).
2. **Layer 1 — MC_SERVICE_INTERRUPTS (0x20801702)** completion (known post-M8.4 blocker; oracle
   M8.108 service-interrupt credits). The real cuCtxCreate gate.
3. COPY2/COPY3 `0x57` (only if matmul HtoD/DtoH needs those copy TSGs).

## PROGRESS LOG 6 (2026-06-10) — CORRECTION: keystone only HALF-cracked; host does NOT execute yet

Ran the m2hostsem A/B experiment (M5.35: gate OFF QEMU's Phase-B completion-sema stub writes so the
host would be the sole writer). RESULT **disproved the double-write hypothesis and corrected an
over-claim**:
- With `m2hostsem=on`, the CE scrubber wait **TIMES OUT** (`NV_ERR_TIMEOUT memmgrMemSet PREFER_CE @
  mem_mgr.c:463`; `pCeUtils->lastCompletedPayload == lastSubmittedPayload @ ce_utils.c:349`) →
  `RmInitAdapter failed` → **cuInit 999**. So the **host GPU is NOT writing the completion semaphore**
  — QEMU's Phase-B stub write was the ONLY thing satisfying the scrubber wait. The `0x1_00000054` seen
  earlier was QEMU's OWN buggy write (NVC56F `sem_pay_hi=1` + `lo=0x54`), not a host completion.
- Therefore M5.34 fixed the MMU fault (real — page now mapped) but the host then hits **Xid 32
  (corrupted/invalid pushbuffer stream)** on the CE channel and does NOT execute the work. The
  keystone advanced `Xid 31 MMU-fault → Xid 32 parse-error` but is **NOT** "host runs forwarded work".

**M5.35 kept as a default-OFF diagnostic toggle** (NVKVM_M2HOSTSEM=1) — it cleanly proves whether the
host writes completions; it is NOT a fix. Normal boots stay m2hostsem=off (QEMU stub load-bearing).

### THE REAL KEYSTONE WALL = Xid 32 (host can't execute the forwarded pushbuffer)
Why does the host GPU read the (now-mapped) pushbuffer and reject it as corrupt? Candidates:
1. **Wrong backing content** (most likely): M5.19 maps VA→gpa via the per-client PDB walk
   (nvkvm_chan_translate). If that walk resolves the WRONG gpa (the M5.16/M5.17 "VAS-walk gave wrong
   page" aliasing), the host reads garbage as methods → Xid 32. The forwarded pushbuffer must be the
   guest's EXACT bytes.
2. GP-entry LENGTH/encoding mismatch (host reads wrong # method words from the double-mmapped GPFIFO).
3. A method-referenced VA (copy src/dst, sema) not mapped in the host VAS (would more likely be Xid
   31, but check).
Method: fresh boot (m2hostsem=off), capture the exact Xid 32 (host dmesg) + for the faulting CE
channel dump the GP entry + the pushbuffer bytes the host reads (host VA content) vs the bytes the
guest wrote (guest gpa). A content mismatch confirms candidate 1. Hand the precise trace to Fable.

## PROGRESS LOG 7 (2026-06-10) — DECISION + cuCtxCreate blocker PINNED

User principle (decisive): "if it's forwardable from host userspace unprivileged, use that; otherwise
simulate, especially if it's kernel-only." → The CE scrubber is the guest KERNEL's internal vidmem
zeroing (CeUtils), NOT host-userspace-forwardable, and forwarding it is what fails (Xid 32). So
**SIMULATE it (QEMU's Phase-B stub already does — cuInit passes) and STOP chasing Xid 32.** The
forwardable workload is the compute channel (matmul); forward THAT for real. Dropped: the Xid-32
host-execute rabbit hole.

**cuCtxCreate blocker PINNED with fresh data (m2hostsem=off, default):** cuInit + all device queries
pass; cuCtxCreate hangs. The control histogram during the hang: **28× `0x20801702`
MC_SERVICE_INTERRUPTS** (top), 18× `0x2080012b` PROMOTE_CTX, interleaved — then timeout → teardown
(`fn=47 UNLOAD → WPR2 down → GSP restart`; the tail SEC_CPUCTL/PMC_BOOT_0 falcon spin is the
post-reset reboot, not the hang itself).

Root mechanism: the guest poll-loops MC_SERVICE_INTERRUPTS waiting for a GSP completion event (GR
ctx-init / PROMOTE_CTX completion) that we never post. The event machinery EXISTS
(`nvkvm_m3_post_event` 0x1003 + `nvkvm_gsp_deliver_events` + SWGEN0 raise + osevents[] registered on
class-0x0079 alloc) but only fires on `any_completed` (a channel's gp_get advanced). During
cuCtxCreate the GR/compute channel submits NO sustained GPFIFO work (Layer 1 — golden ctx is
GSP/FECS-internal), so `any_completed` is false → no event → infinite wait. Per the principle we ARE
the GSP, so we must SIMULATE posting the completion event the guest's cuCtxCreate wait needs.

The current branch's `0x20801702` default reply (echo mask, status=0, no forward) is already the
oracle's clean guest-only behavior; the oracle's M8.108 env-knob maze (ZERO/_AFTER_LOCAL/_AFTER_HOST/
HOST_ZERO_BUDGET) is SLOP — do NOT port it. The real task = pin WHICH GSP event/notifier the guest's
cuCtxCreate wait drains, and post it at the right trigger (cleanly). Oracle solved this only flakily
(2/4). NEXT: Fable reverse-engineers the precise wait + clean post-trigger; I implement + verify.

## Escalation rule (user, 2026-06-10)
**Never report "stuck" until Fable is also stuck on it.** When you reach the point where you'd stop
and ask the user / declare a blocker, FIRST hand the problem to a Fable subagent (`model: fable`) —
the precise source-trace or byte/fault correlation. Only surface a blocker to the user once Fable has
also failed to crack it. (Fable's grounded byte/fault analyses are reliable; verify its output vs
hardware. Its open-ended source-traces can over-commit — anchor every Fable task in concrete data.)

## Stop-and-report forks
- Step 4 ring wedges repeatedly / needs `vastai reboot` → report.
- A required completion turns out NOT to come from a host-pollable fd (host wouldn't interrupt) →
  rule question, report.
- matmul (step 6) is wrong despite a real host run → content-coherence fork, report.
