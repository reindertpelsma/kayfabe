# The blocking-and-completion model — what may run inline on a guest trap

> **STATUS: LIVE — 2026-08-14.** Owner-agreed in conversation. This is a **specification and a
> predicate**, not a schedule. ★ It governs every site that runs under a guest MMIO trap.
> Supersede it in place; do not write a successor beside it.
>
> **Origin:** the owner's question — *"inline in vcpu thread, no blocking calls that can sleep/wait,
> so always 'async' or deferred with correct completion, right?"* — and the answer measured below:
> **right, and under QEMU it is stronger than that; but "always async" is too strong and would cost
> us the thing that already works.**

---

## 0. The measured fact this all rests on

**Every guest MMIO write — including every doorbell — arrives with the QEMU BQL held.**

- `crates/kayfabe-qemu-raw/src/shim.rs:4877` — *"every MMIO write arrives with the BQL held, so a
  second doorbell cannot …"*
- `:6146` — *"the trap is inline end to end — QEMU BQL → `kayfabe_shim_regs_write` → …"*
- `:6046` — the CE submission runs *"under the FSM mutex and the BQL"*.

⇒ ★★★★★ **Blocking in a trap handler does not stall the ringing vCPU. It stalls EVERY vCPU and
QEMU's main loop**, because they all serialise on the BQL: timers, I/O, the monitor. **A sleep there
is a whole-VM freeze**, not a slow path.

⊘ **Do not describe this as "occupies the vCPU thread."** That phrasing was used in this campaign
before the BQL was checked, and it understates the blast radius by the whole machine.

---

## 1. The predicate — the thing to actually check

Not *"does it sleep?"* — that is the wrong question and it forbids work that is fine. The question is:

> **INLINE-SAFE(site) ⇔**
> **(a)** it completes **without the guest running**, **and**
> **(b)** it completes **within the shortest guest-side timeout that covers this operation**, **and**
> **(c)** it holds **no lock that another vCPU's trap path takes**.

Each clause has a known way of being violated in this tree:

| clause | violation | status |
|---|---|---|
| **(a)** waiting on something the guest must do | **guaranteed deadlock** under BQL — the guest cannot run to do it | ⚠ the **doorbell severance** (§4) has this smell and must be checked against (a) when it is ruled on |
| **(b)** exceeding a guest timeout | the guest's own watchdog fires and attributes the failure to *itself* | ⚠ real numbers below |
| **(c)** holding a lock another trap path takes | **ABBA**, and the guest builds it by ringing on one vCPU while touching a register on another | ★ **live and guest-buildable today** — see §4 |

### The timeouts that bound clause (b) — real, named, not guesses

- **4000 ms** — `scrubberDestruct` waiting on `pCeUtils->lastCompletedPayload == lastSubmittedPayload`
  (`ce_utils.c:349`; the guest's own `NVRM: scrubberDestruct: Timed out …` line is the symptom).
- The guest kernel's **soft-lockup detector** and **RCU stall detector** sit above that. ★ A guest
  `dmesg` carrying a soft-lockup **is** the BQL-stall signature, and is far better evidence than an
  unattributed timeout.

⇒ **Clause (b) is bounded by 4 s at the most generous, and by the guest kernel well before that if
the whole VM is frozen rather than one operation being slow.**

---

## 2. The three tiers, ranked — best first

### Tier 1 — PASSTHROUGH. No completion of ours at all. **Prefer this always.**

We do not wait because there is nothing of ours to wait for: **the guest polls its own semaphore and
the hardware writes it.** No completion architecture is involved, so none can be wrong.

★ **This is why cup3 crossed with no completion-wait architecture in the tree.** `CUP3_VAL=43`
(w297, `c5d0510`) was produced with **zero** code in this stack waiting on anything — the host GR
engine wrote the guest's semaphore directly. ⇒ **the absence of a completion-wait architecture on
the user compute plane is the passthrough being real, not a gap.**

⊘ It also means the owner's rule — *"a completion is sent only if the observed state after it is
intended and safe in the guest"* (§0 of `road_to_v1_after_cup2.md`) — is currently satisfied
**vacuously, because we send none on this plane.** **A rule never seen to fire is not a rule**: the
first completion we ever wire is unexercised code meeting an untested rule.

### Tier 2 — INLINE, bounded, guest-independent, lock-free. **Legitimate. Do not "fix" it.**

Where `INLINE-SAFE` holds, running inline is **simpler and faster** than deferring, and its
correctness is easier to argue: there is no ordering question, no redelivery, no completion to get
wrong.

★ **This is what the kernel-CE plane does today and it is correct today.** The scrubber's CE copy is
**short and emulated**, so *"it runs synchronously off the doorbell"*
(`kayfabe-abi/src/eventnotify.rs:191-193`) satisfies (a), (b) and — modulo §4 — (c).
`announce_completion` then raises the non-stall vector, and the promise is **auditable rather than
asserted**: every local serving lands in exactly one of `Counters::nonstall_raises` or
`Counters::nonstall_unvectored`, and the second — *work that happened and was never announced* — has
a healthy value of **zero**.

⚠ **Tier 2 is correct today ONLY because the work is short and emulated.** See §3.

### Tier 3 — DEFERRED, with an explicit completion. **Mandatory where the predicate fails.**

Submit, return to the guest, complete later off a real signal.

★★ **The machinery already exists**: `crates/kayfabe-completion/`, and
`crates/kayfabe-shell/src/reactor.rs` (`Reactor`, `ReactorHandle`, `ReactorStats`). A prior audit
(`026374c`) found the reactor subsystem **built and unreached**. ⇒ **tier 3 is a WIRING job, not a
design job** — which is good news for cost and bad news for risk, because unreached code has never
had to be right.

---

## 3. ★★★ The transition to design for — it is foreseeable, not hypothetical

> **Tier 2's correctness for kernel CE depends on the work being short and emulated. The moment
> kernel-CE work is forwarded to a REAL engine, clause (b) fails and that site becomes tier 3.**

This is the single scheduled event that moves a site between tiers, and it is coming: the whole
point of the project is that real engines do the work. ⇒ **do not treat "kernel CE is inline" as a
permanent fact**; treat it as a tier-2 site with a **known expiry condition**.

⊘ **And the expiry is silent.** Nothing today fails when a tier-2 site starts waiting on real
hardware — it just gets slower, then crosses 4 s, then the guest blames itself. **That is why the
predicate has to be checkable rather than remembered.**

---

## 4. What is open against this model right now

> ### ⊘⊘⊘ CORRECTED `[w300, 2026-08-13]` — **THE BULLET BELOW WAS ALREADY FALSE WHEN THIS DOC
> ### WAS WRITTEN, AND IT IS THE CLASS THIS TREE PAYS FOR MOST.**
> `Mutex<PlaneState>` **is not a bare `std::sync::Mutex` and has not been since 2026-08-11.**
> It is `RankedMutex<PlaneState>` at `plane.rs:1035`, `LockRank::Plane` (rank 0), ranked by
> **w236 `56269390`** — *two days before this doc was committed* — and `forward_ring` was
> restructured by **w237 `7107bba5`** into PLAN / **FETCH** / ACT, where the framebuffer read
> happens in the FETCH phase with **every ranked guard dropped**. Both are ancestors of
> `master`. ⇒ **the specific ABBA described below is CLOSED**: `core → plane` now panics by
> name in `check_acquire`, always-on (not a `debug_assert`), *before* the OS-level acquire.
> ⚠ **The reason it read as live is worth more than the correction.** The finding was true at
> `e683681`, was fixed four commits later, and was carried forward verbatim into an
> owner-agreed doc by someone citing the finding rather than re-reading the code. *A ruling's
> date is part of its citation* — and **citing the oracle is not the oracle being right.**
>
> ★★★ **WHAT IS ACTUALLY STILL OPEN FOR CLAUSE (c), measured at `77ad6c2f`:** the enforcement
> covers **ranked** locks only, and the census that was supposed to catch the rest —
> `tests/tests/unranked_locks.rs` — was **blind to `Arc<Mutex<…>>`**, the spelling used for
> every lock shared between the vCPU and another thread. **Nine unranked vCPU-path locks were
> never classified** while the gate reported zero unclassified, including
> `CeShellState::gr_cursors`, which is held by two threads and whose own doc *claimed* a
> classification in that file that did not exist. Fixed in w300: the scanner steps out through
> wrapper generics, the nine are classified, and a **known-positive fixture suite** now pins
> every spelling — so a future blind spot fails **naming the shape** instead of shortening the
> list silently. ⇒ **(c) is ENFORCED for ranked locks and ENUMERATED for unranked ones; it is
> still not *enforced* for the unranked set.**

- ⊘ **[SUPERSEDED — see the correction directly above; kept for the reasoning, not the status.]**
  ★★★★★ **Clause (c) is violated today, and the guest can build it.** `Mutex<PlaneState>`
  (`plane.rs:1034`) is a bare `std::sync::Mutex`; `RegPlane::pt_bytes` takes it per read, while
  `forward_ring` calls the ring reader inside a `route_act` closure already holding the rank-0
  device read lock + rank-1 proc mutex — **and the opposite order already ships** (the policy chain
  takes core locks under `state.lock()` on another vCPU's MMIO trap). **A guest that rings a
  doorbell on one vCPU while touching a register on another builds the deadlock itself.**
  ⊘ **No gate caught it because `assert_lock_free` masks only RANKED locks** ⇒ the R1 witness
  passes **vacuously**. *A gate that cannot see the thing it gates returns the same answer for a
  safe design and an unsafe one.* → in flight as `w300`.
- ⚠ **The doorbell severance** — `FwdFault::SystemDataPlane` refuses `publish_backing` on
  `SYSTEM_PROC`/`SYSTEM_ANCHOR` at all three sites that can set `Binding::host`. **Open owner
  ruling.** ★ When it is ruled on, check the resolution against **clause (a)**: any design where we
  wait inline for something the guest must supply is a whole-VM deadlock, not a slow path.
- ⊘ **Clauses (a) and (b) have no mechanism at all.** (c) is getting one via `w300`. (a) and (b) are
  currently **prose in this file**, which by this repo's own history means they will be violated by
  a well-meaning patch. **Making them checkable is the next build item under this doc.**
  ★ `[w300, 2026-08-13]` **Still true, and w300 did not change it.** w300's mechanism is a
  *census* — it enumerates locks and demands a written ruling per lock. That shape cannot see
  (a) or (b) even in principle: both are properties of **what runs beneath a lock**, not of the
  lock's existence, and a fixture-checked scanner over field declarations has no reach into
  call graphs. ⇒ (a)/(b) need a different instrument (a call-graph or effect-typed one), and it
  was deliberately **not** widened into w300's rung.

---

## 5. Why this shape rather than "make everything async"

★★ **NVIDIA's own model is tier 3 where tier 3 is needed, and tier 2 elsewhere.** RM submits and
returns; completion arrives by interrupt/notifier; GSP RPC is a message queue with a separate
completion path. ⇒ **mirroring the vendor is the low-risk choice** — the same argument that decided
the SM-debugger policy by following nvproxy rather than re-deriving it.

⊘ **And converting tier-1 or tier-2 sites to tier 3 is not free — it is a regression in safety.**
It replaces "nothing can be wrong because nothing is waiting" with an ordering problem, a
redelivery problem, and a completion that must satisfy a rule **never yet observed to fire**. ⇒
**the goal is not "async everywhere"; it is "tier 1 wherever possible, tier 2 where the predicate
holds, tier 3 only where it fails — and the predicate CHECKED rather than remembered."**

---

## 6. What "done" looks like for this doc

1. **`INLINE-SAFE` is a checkable predicate**, not prose — clause (c) via `w300`'s lock visibility;
   clauses (a) and (b) need a mechanism that does not exist yet.
2. **Every site that runs under a guest trap is classified** tier 1 / 2 / 3, with its clause-(b)
   bound named.
3. ★ **A known-positive for each clause** — a deliberately-unsafe site observed to be **refused**.
   *A rule never seen to fire is not a rule*, and this campaign has shipped several gates that could
   not fail.

Related: `road_to_v1_after_cup2.md` §0 (the completion rule, stated as construction),
`mode2_interrupt_delivery.md`, `unranked_locks.rs`, `docs/audits/` (`026374c`, the reactor audit).
