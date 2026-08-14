# The road to v1, after cup2 — gates, threat model, audit order

> **STATUS: LIVE — 2026-08-14.** Owner's plan, agreed in conversation, with three pushbacks the
> owner accepted folded in. ⊘ This is a **gate order and a specification**, not a schedule. Supersede
> it in place; do not write a successor beside it.
>
> **Precondition, not yet met:** `CUP2_RC` is **1** (was 124). The address plane is closed —
> `Xid = 0`, `host_rows = 18 295/18 309` — and the remaining wall is our own control-plane refusal
> vocabulary. **Everything below assumes cup2 passes first.**

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

★★ **Answer this by evidence before building anything:** *does cup2 — or any real client — ever put
a CE operand in a VAS that never doorbells?* ⇒ **If not, criterion 1 needs a different PROBE, not a
bigger mechanism**, and widening publication to every declared VAS would scale cost and blast radius
with **guest behaviour** rather than with work.

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
