# The road to v1, after cup2 — gates, threat model, audit order

> **STATUS: LIVE — 2026-08-14.** Owner's plan, agreed in conversation, with three pushbacks the
> owner accepted folded in. ⊘ This is a **gate order and a specification**, not a schedule. Supersede
> it in place; do not write a successor beside it.
>
> ### ⊘⊘ CORRECTED 2026-08-14 — **THE PRECONDITION IS MET, AND THE TEXT BELOW IT WAS STALE IN A
> ### `LIVE` DOC.** It read *"Precondition, not yet met: `CUP2_RC` is **1**"* — superseded by w294's
> **2/2 at `^CUP2_RC=0`** (`1c8e508`) and again by w297. Caught by the cup3 lane, not by a reader of
> this file: ★ **a `STATUS: LIVE` header certifies that the doc has not been RETIRED, never that its
> body is current.** That is the failure this repo's own doc-hygiene rule exists for, committed in
> the file that states the rule.
>
> **Gate 1 is CLOSED, twice over `[measured, real GA106 580.159.04]`:**
> - **cup2** — `^CUP2_RC=0`, **2/2** (w294 `1c8e508`). ⊘ A **CE round-trip**, not compute.
> - **cup3** — `^CUP3_VAL=43` from `out = in*3 + 1`, `in = 14` (w297 `c5d0510`). ★★★★★ **FIRST
>   COMPUTE**: no copy engine, fill, or forged completion in this stack can produce 43, so the
>   value is un-forgeable proof the **host GR engine ran the guest's shader**. Full ladder, no
>   `✘`: CTX → MODULE → FUNC → MEMALLOC → LAUNCH → SYNC → KERNEL → DONE.
>
> **The address plane held across both:** `Xid = 0`, `host_rows = 18 295/18 309` — *identical* on
> cup3, and the unserviced-control ledger is the **same 40 ids** as cup2, so module load and kernel
> launch demanded **no control we refuse**.
>
> ⊘ **Both are RELAXED greens and neither is the milestone.** Eleven relaxations were in force
> (`PT_SWEEP=on`, `OPERAND_JOIN=join`, `VAS_PUBLISH=drain`, `FB_JOIN=shared`,
> `GR_ROUTE=passthrough`, ring/pushbuf/sema/operand pinned, `ISOLATES=real`, `CE_EXECUTOR=host`).
> ★ **But `43` is now a KNOWN-POSITIVE that can grade them one at a time** — a strictly better
> instrument than cup2 ever was, and the obvious next rung.
> ⊘ And it is **one boot** of a 1×1×1 launch of a six-instruction shader. cup2 was confirmed 2/2
> *before* it was believed; cup3 has not been.

---

## 0. The rule that governs everything downstream

> **A completion is sent only if the observed state after it is intended and safe in the guest.**
> — owner, 2026-08-14

⇒ A no-op fake GSP init **completes immediately, because the observed end-state genuinely is
"nothing to do."** A copy completes **because bytes moved.** Same for every kernel channel.

★★★ **State this as CONSTRUCTION, not policy.** *"Do not forge"* is a rule that decays; *"a
completion carries the evidence of its own end-state"* is a type. **The wins that held on this
campaign were all of the second kind** — the ring gate is `E0639` at compile time, the channel role
is a **type** so a wrong role does not compile, `MERGE-AGREES` is a checked invariant rather than a
printed number. ⇒ **Every policy-shaped completion rule will eventually be violated by a well-meaning
patch; a construction-shaped one cannot be.**

⚠ **Known-positive required:** a deliberately-unsatisfied completion must be **observed to be
refused**. A rule never seen to fire is not a rule.

> ### ★★★ EXTENDED 2026-08-14 — **THE RULE ABOVE PRESUPPOSES WE SEND COMPLETIONS. ON THE USER
> ### COMPUTE PLANE WE SEND NONE, SO IT IS SATISFIED VACUOUSLY.**
> `cup3` crossed at `CUP3_VAL=43` with **nothing in this stack waiting on anything**: the guest
> polls its own semaphore and the host GR engine writes it. ⇒ **the absence of a completion-wait
> architecture there is the passthrough being real, not a gap** — and by the line directly above,
> **a rule never seen to fire is not a rule**, so the first completion we ever wire is unexercised
> code meeting an untested rule.
>
> ★★★★★ **And there is a second rule, of the same kind, governing WHERE a completion may be
> awaited — measured, because every guest MMIO write arrives with the QEMU BQL held**
> (`shim.rs:4877`, `:6146`). **Blocking in a trap handler freezes EVERY vCPU and QEMU's main loop**,
> not just the ringing one.
> ⇒ **`INLINE-SAFE(site) ⇔` it completes (a) without the guest running, (b) within the shortest
> guest-side timeout covering it — the scrubber's is **4000 ms** — and (c) holding no lock another
> vCPU's trap path takes.**
> ⊘ Clause **(c) is violated today and the guest can build it** (`Mutex<PlaneState>` is unranked, so
> the R1 witness passes **vacuously**). Clauses (a) and (b) have **no mechanism at all**.
>
> **Full model, the three tiers, and what is open against it:
> [`blocking_and_completion_model.md`](blocking_and_completion_model.md).** ⚠ Read it before adding
> any wait, any lock, or any completion to a path that runs under a guest trap.

---

## 1. Gate order — and two of these are reordered from the owner's first draft

| # | gate | why here |
|---|---|---|
| **1** | **cup2 passes** | precondition for everything |
| **2** | **interrupt / os-event + error reporting, hard test** | ⚠ **its own rung, not a checkbox** — see §2 |
| **3** | **compatibility audit** | ⊘ **moved BEFORE security** — see §4 |
| **4** | **security · race safety · unsafe Rust** | audit code that is not about to move |
| **5** | **cup2 on a second architecture** | ⊘ **format oracle first** — see §5 |
| **6** | **LLM at 60 tok/s in guest, host compute intact** | the product metric |

---

## 2. Gate 2 — the delivery plane is half-built and half-blocked

**Do not schedule this as a checkbox.** Measured state:

- ⊘ **Criterion 1 (the guest observes the SAME fault by identity) has slipped five rungs.** Its
  blocker is named: `CRIT1 STATE = CONTROL-NEVER-LANDED`, and it is **not relaxation-dependent** —
  identical on every arm.
- ⚠ **The wall is SCOPE, not mechanism.** Arm 4's control operands live in a **third, freshly
  allocated VAS** that nothing we serve doorbells, and **every** operand mechanism we own — the pin,
  the join, FB publication, the guest-RAM merge — is scoped to the **doorbelling channel's VAS**.
- ★ **`FaultEmission` is built and orphaned** (`kayfabe-rmrpc/src/fault.rs`) — and per the
  2026-08-13 finding it is **the wrong shape anyway**: the host RM writes the guest's own notifier
  pages when we register them, so **we are not the writer.**
- ⊘⊘ **CORRECTED 2026-08-14 — the bullet below carries TWO FACTS AS ONE, and the wrong one.**
  Full derivation: `docs/reference/sm_debugger_scope_and_sm_error_registers.md` (branch
  `w289-sm-debugger-scope-and-registers`).
  **(a) `hTargetChannel` is not "never validated" — CPU-RM NEVER READS IT AT ALL.** It is a *control*
  parameter; the handle RM actually resolves is `HClass3DObject`, an **alloc** parameter, and it
  resolves it **inside the calling client's handle space** and demands `RS_ACCESS_DEBUG`
  (`= ALLOW_OWNER`) on the ref (`kernel_sm_debugger_session.c:255`, `:291-302`). ⇒ **Attaching to a
  stranger's context is structurally blocked**, and forging `hTargetChannel` buys an attacker
  nothing.
  **(b) The exposure is RESIDENCY, not the handle.** NVIDIA's own header: the control *"acts upon
  the **currently resident** GR context"* (`ctrl83dedebug.h:325-327`), and `accessRight = 0x0` on all
  31 exported methods ⇒ **there is no second gate at control time.** So the tier split is real, but
  its cause is the residency race, and the reachable leak is a **resident neighbour's**
  `hwwWarpEsrPc64` / `hwwEsrAddr` while it faults — **metadata about a faulting neighbour, not a
  read primitive.**
  ★★ **POLICY, owner 2026-08-14: FOLLOW NVPROXY.** *"they really thought about it for sandboxed
  containers meant for adversarial code."* nvproxy admits **exactly three** `0x83de03xx` controls,
  all on `compUtil` (`gvisor/pkg/sentry/devices/nvproxy/version.go:334-336`):
  `0x83de0309` SET_EXCEPTION_MASK, `0x83de030c` READ_ALL_SM_ERROR_STATES, `0x83de0310`
  CLEAR_ALL_SM_ERROR_STATES — plus the class `GT200_DEBUGGER` itself (`:427`). ⇒ **Admit those
  three and the class; deny `0307`/`0317`/`0318`** (SUSPEND/RESUME are absent from nvproxy's table,
  and denying them is what keeps `030c` coherent — they would turn the residency race into a
  **deterministic** read).
  ⚠ **`CLEAR_ALL` (`0310`) is the sharper half of the pair**: suppressing a victim's fault needs only
  to **precede**, not to win a race to observe. nvproxy carries it anyway; we match.
  ⚠ **Architecture constraint that falls out of this:** GSP's own check is
  `VALIDATE_MATCHING_SEC_TOKENS`, and a NULL token *"allows access to any client in the system"* ⇒
  **isolation here is per-RM-client, so multiplexing several guests into one host client would grant
  DEBUG across all of them.** See §3's cross-process requirement.
- ⊘ **`0x83de030c` is a DIFFERENT TIER from `0x83de0309`** (global SM registers for the currently
  resident GR context; `hTargetChannel` never validated). Serving `0309` **neither implies nor
  requires** it. Measured, not assumed.

> ### ⊘⊘⊘ CORRECTED SAME DAY, 2026-08-14 (w305) — **THE PRESCRIBED FIX BELOW IS A NO-OP, AND
> ### THE DIAGNOSIS IT RESTS ON IS REFUTED BY A NATIVE KNOWN-POSITIVE.**
>
> The block below rules: *"The fix is one line in the probe: allocate its control operands in
> the VAS of the channel it rings."* **They were never anywhere else.**
>
> **(1) STRUCTURAL — there is no line to change, and it is provable from our own source.**
> `probe_guest_reachability(vas, …)` takes `range = self.narrow(vas)`
> (`kayfabe-isolate-host/src/rm.rs:6976`), maps **every** operand into that range
> (`:7036` `ctrl_src`, `:7044` `dst`), and creates the channel it rings on **the same `vas`**
> (`:7080`, `alloc_channel_at_with_error_notifier(vas, …)`). ⊘ And `narrow` is not a
> re-derivation — it is `u32::try_from(h.raw())` (`rm.rs:4233`), a pure handle-width cast. ⇒
> **the operands and the ringing channel have always shared one VAS handle.**
>
> **(2) EMPIRICAL — the arrangement the block blames WORKS ON REAL HARDWARE.**
> `[measured 2026-08-14, w305, vh2, real GA106 580.159.04, NO QEMU]` the same binary, the same
> `--ce-client-fault`, the same THIRD freshly-allocated VAS, run natively:
> ```
> info  R33 arm 4 SPACE     = a THIRD, freshly allocated address space (range 0xcafe0011)
> ★     R33 CRIT1 STATE     = FAULT-PROVOKED-ADDRESS-READ | VA-IDENTITY MEASURED = yes
> ★     R33 arm 5 WHERE     = GET_MMU_FAULT_INFO addr=0x0000000900000000 faultType=0x0
>                             faultString="FAULT_PDE" | VA-IDENTITY HOLDS
> host dmesg: Xid 31 … channel 0x00000005 … MMU Fault: ENGINE CE0 HUBCLIENT_CE1
>             faulted @ 0x9_00000000. Fault is of type FAULT_PDE ACCESS_TYPE_VIRT_READ
> ```
> ⇒ **A third, freshly-allocated VAS is not a blocker for anything.** The control lands, the
> fault is provoked, the address plane answers, and two independent observers — the client's
> own in-process `GET_MMU_FAULT_INFO` and the host kernel log — name the same address.
>
> ★★★ **So the probe is a KNOWN-POSITIVE, and the thing under test is what fails.**
> `[measured 2026-08-14, w305, `run_w305bfresh`, rev `c7c058a3`]` the guest arm reproduces
> `CRIT1 STATE = CONTROL-NEVER-LANDED` exactly, with **zero `Xid` on the host and zero in the
> guest**, and the control's own failure line reads:
> ```
> ?? R33 arm 4 control = the POSITIVE CONTROL did not land
>    (sem 0x00000000, GP_GET 1 GP_PUT 1, moved 0xdead0000 want 0x5ea1c071)
> ```
> ⇒ the cursors **caught up** and the sentinel **survived**: the ring was consumed and no
> bytes moved. That is our CE plane, not a VA the probe chose badly.
>
> ### ★★★★★ AND THE SAME RUN CARRIES THE DISCRIMINATOR THE ORIGINAL RULING WAS REACHING FOR
>
> In **one process, one program, one boot**, on our emulated GPU:
>
> | arm | VAS | result |
> |---|---|---|
> | **1** | `vas` — allocated first, **already carried work** | ★ **4096 bytes moved**, semaphore `0x1` = declared, `GP_GET 1` caught `GP_PUT 1` — the whole four-fact bar |
> | **4** | `fvas` — **freshly allocated**, same engine `COPY0` | ⊘ control never landed, nothing moved |
>
> ⇒ **The VAS is still a live variable in the GUEST** — but the ruling below could not have
> established it, because the mechanism it named (*operands in the wrong VAS*) is not the
> difference. The difference is between a VAS that has already carried retired work and one
> that has not. ⊘ Natively **both** work, so this is ours.
> ⚠ Not yet isolated: arm 4 also differs from arm 1 by being the **second** channel in the
> process, by dictating its ring at `0x7_0000_0000`, and by carrying an error notifier.
>
> ⊘ **THE ARM BUILT TO ISOLATE IT DID NOT RUN, AND THAT IS REPORTED RATHER THAN SMOOTHED.**
> `--ce-client-fault-shared-vas` (w305, `rmladder.rs`) runs arm 4 in arm 1's own VAS.
> `[measured, `run_w305bshared`]` it returns `CRIT1 STATE = PROBE-NOT-BUILT`:
> `FAIL R33 arm 4 = the probe could not be built: BadHandle(HostHandle(iso0/gpu0:0xcafe0005))`.
> Cause is **our own bookkeeping, not RM**: `alloc_channel_in` resolves
> `self.conn.space_of(range)` (`rm.rs:5831-5833`) and arm 1's `vas` handle is the **space**
> (`0xcafe0005`), whose paired **range** is a different handle (`0xcafe0009`, the number arms
> 2/3 print). ⇒ **the shared arm says NOTHING about the VAS hypothesis**; it is a construction
> failure, and the fix is to pass the range rather than the space.
>
> ★★ **The instrument lesson, and it is the same one this file records two bullets up:** the
> ruling was derived by reading `OPERAND-PIN`'s `pdb=` in a cup2 trace and then reasoning about
> what arm 4 *must* be doing. **Nobody opened `probe_guest_reachability` and looked.** A
> diagnosis about a probe that is never checked against the probe's own source can name a fix
> that does not exist — and this one did, in the most-read planning doc in the tree, for the
> criterion that has slipped six rungs. ⚠ Note what survived: the **intuition** (the VAS
> matters) was right and the **mechanism** was wrong, so a reader who acted on the prescription
> would have changed nothing and reported it as tested.

★★★★★ **ANSWERED 2026-08-14, from the committed green trace — NO. Build the probe, not the
mechanism.** ⊘ **SEE THE CORRECTION DIRECTLY ABOVE — the conclusion this block draws about arm
4 is refuted; the cup2 measurement it rests on is not.** Measured in
`traces/w294_cudalimit/run_w294cup2_qemu.log` (the `^CUP2_RC=0` boot):

| | reading |
|---|---|
| `OPERAND-PIN` lines | **319** |
| distinct `pdb=` across all 319 | **1** — `0x201000`, every one |
| channels they cover | **all 16** (`chan=0`…`chan=15`) |
| `SEMA-PIN` / `PB-PIN` / `RING-PROJ` pdbs | `0x201000` — **the same VAS** |
| VASes cup2 *declares* | **4** (`0x0`, `0x200000`, `0x201000`, `0x2efa9c000`) |

⇒ **cup2 declares four address spaces and puts every operand, semaphore, pushbuffer and ring in the
one that doorbells.** The VAS that carries work is `0x201000` (18 309 rows, `published=18305`).

★★★ **Therefore: criterion 1 needs a different PROBE, and widening publication is RETIRED.** Arm 4's
operands live in a third VAS **because arm 4 put them there** — that is a property of our own raw
probe, not of how CUDA lays out memory. ⇒ **The fix is one line in the probe: allocate its control
operands in the VAS of the channel it rings.** ⊘ Widening publication to every declared VAS would
have scaled cost and blast radius with *guest behaviour* to serve a shape no real client produces.

⚠ **Scoped honestly:** this is **cup2's** behaviour on **one** green boot, not a proof about every
client. It is enough to retire *this* mechanism for *this* criterion, and not enough to assert
"no client ever does". A client that genuinely used a non-doorbelling VAS would reopen it — and
`OPERAND-PIN`'s `pdb=` field is exactly the instrument that would show it, so the check is cheap to
repeat on any future workload. ★ Re-run it on cup3.

---

## 3. The threat model — owner's, 2026-08-14

### Violations (all of these are bugs)

1. guest userspace **or the isolate** obtaining access to arbitrary GPGA or guest RAM
2. isolate → **VMM** breakout
3. isolate → **guest kernel** breakout
4. guest process → **RCE in the isolate**
5. guest process → **unrelated guest process** it has no access to
6. guest process → **guest kernel** escalation
7. guest kernel → **RCE in the isolate**
8. guest kernel → **VMM** breakout

### Safe by construction (not findings)

- VMM → guest kernel / guest userspace
- host GPU → VMM ⚠ **only via the privileged side; an unprivileged malicious channel reaching the
  VMM is a violation**
- guest kernel → all guest userspace processes

### ★★★ The asymmetry that is easy to miss

**`ogkm` trusts the GPU.** It applies no guardrails to GSP RPC replies and grants the device full DMA
to its memory. ⇒ **We inherit that trust because we are the GSP.**

⇒ **Therefore the red flag is: can guest USERSPACE steer the data we hand the guest KERNEL such that
it violates the spec `ogkm` expects?** That is a **confused-deputy** question — *"can an unprivileged
guest process use us as a weapon against its own kernel"* — and **nothing in the current audit set
asks it.**

⊘ **The converse is NOT a finding:** the guest kernel letting the VMM violate spec is fine, provided
it carries no breakout risk.

---

## 4. Audit order — compatibility first, and every audit carries a plant

### ⊘ Why compatibility precedes security

**Compatibility findings change the code. Security findings are only valid against code that is not
about to move.** ⇒ Auditing first and then landing a driver-version or architecture fix means
**auditing twice**, and this tree has already paid for *"a ruling's DATE is part of the citation."*

Axes: driver versions · GPU architectures · guest kernel versions · **QEMU and Cloud Hypervisor as
first-class, not bolt-on**.

### ★★★ Every audit ships with a known-positive, or it does not ship

⚠ **This is the failure mode these audits will actually hit.** *"Scan for exploits"* with nothing
planted **returns clean — and clean is exactly what a broken scan returns.**

> **Plant one reachable violation per class before running the audit. A scan that misses its own
> plant is a broken instrument, not a clean bill of health.**

★ Measured justification, one day: an instrument returned the same error for **every** address
including a known-mapped one (only the known-positive made it a refutation rather than *"nothing is
mapped"*); a headline count was wrong because it read **one of two records**; a cursor advanced on an
arm that moved **zero bytes**. ⇒ **Six times in one day, an instrument's null and the system's
success were the same reading.**

Applies equally to **race safety** (plant a reachable ordering violation) and **unsafe Rust** (plant
a soundness hole the audit must name).

---

## 5. Gate 5 — the second architecture will not fail at cup2

⊘ **It will fail earlier, at a page-table format we never validated.** Measured:

- the **arch floor is in a different GMMU format family** from the only built row;
- **`Ad10xArch::mmu()` still delegates to `MockArch`'s INVENTED format** while the Ampere row is
  oracle-checked.

⇒ ★★ **Compile the second architecture's GMMU format against `ogkm` as an oracle FIRST, then run
cup2.** Otherwise the boot reports *"it broke"* without reporting **where**, and a mock's invented
encoding makes the seam look finished.

---

## 6. Open owner questions

- **`PreemptionNotImplemented` (`0x20801210`)** — keep refusing, or serve? Native GA106 serves it;
  our payload now matches native **except the channel handle**. ⚠ **Do not assume refusing is the
  safe side** — the SM-debugger refusal was measured **backwards**: RM's default is `_ALL`, the guest
  asked for `0x3a` which **excludes `_FATAL`**, so **refusing left the guest strictly more
  permissive.**
- **`0x00801909` `PERF_CUDA_LIMIT_SET_CONTROL`** — native serves `NV_OK`; we refuse.
- **The six `not_granular` rows** (1 021 440 bytes) — RM's 64 KiB fixed-placement gate. Widening
  `FbLeafExtent` to a multi-row extent **collides with the per-row reclaim walk**; one handle in N
  rows is a double free.
- **Publication amortisation** — ~4.4 s once at the first doorbell, then zero over 187 doorbells.
  Working, not optimised.
- **Choice 1** — the constructor hard-coding `kind: RealGpuMemory` would flip `is_guest_ram()` for
  ~16 000 rows and **silently re-route the CE partitioner**. Owner's *"put the kind ON the value"*
  class. Its own item, its own control arm.

---

## 7. What this document is not

⊘ **Not a schedule.** ⊘ **Not a claim that the gates are independent** — gate 2 may be unblocked by
gate 1, and gate 5 may surface work that reopens gate 3. ⊘ **Not a substitute for the measured
state**: every number here carries a rung and a boot, and a number without one is not evidence.

Related: `docs/design/guest_ram_publication_merge.md` · `docs/design/channel_alloc_forwardability.md` ·
`/workspace/nvidia-gpu-passthrough/docs/reference/ogkm_authored_guest_userspace_structures.md`
