# `kayfabe-gsp` — the 580.159.04 correction brief

> **Status: BRIEF ONLY. No code was changed producing this, and no `cargo` was run.**
> Another agent holds the cargo slot; this file describes the changes for that agent to
> execute. Every target below was located by grep in the working tree at
> `master @ 634b253`, not guessed. Where a change cannot be specified precisely without
> compiling, it says so instead of guessing.

## 0. Why this exists

`docs/design/mode2_gsp_port_plan.md` — the plan `kayfabe-gsp` (3 550 lines, 8 modules) was
built from — took its NVIDIA citations from `research_clones/ogkm`, which is **610.43.02**.
**The bench runs 580.159.04.** The matching tree is now vendored at
`research_clones/ogkm-580.159.04/`, and the two disagree materially on the GSP path. The plan
is corrected (see its new §0.1, §3.1a, §4.6, §11.1, §14); this file is the code half.

**Citation convention, normative from here on** (plan §0.1): `ogkm-580:` =
`research_clones/ogkm-580.159.04/`, `ogkm-610:` = `research_clones/ogkm/`. A bare `ogkm:` is
a defect — the crate's doc comments are currently full of them and every one is a 610 path
with a 610 line number. Fixing those is item **B11**.

## 1. The order, and why

Sorted by **risk × cheapness**. Every item B1–B9 is **pure logic**: no OS, no GPU, no guest,
no bench. That matters right now because *the bench host is offline at the provider*
(`4a0cb29`), so a brief that front-loaded hardware would be unexecutable. Only **B10** needs
anything else, and what it needs is reading, not hardware.

| # | change | risk if unfixed | pure logic? |
|---|---|---|---|
| **B1** | the mock guest is 610-shaped for both profiles — fix the **oracle** first | ★★ nothing below can fail-before | yes |
| **B2** | `elemCount > 16` ⇒ guest kernel heap corruption | ★★★ memory safety, guest-visible | yes |
| **B3** | receive advances the ring by a derivation, not by `elemCount` | ★★ permanent ring desync | yes |
| **B4** | 580 queues the init RPCs **before** bootstrap; E6 does not drain, E8 misclassifies | ★★ boot fragility + a false attack signal | yes |
| **B5** | `ElementLayout` has no version selector at all; the interval is `>= 610` | ★★ no production path picks a layout | yes |
| **B6** | 610's transport words are invented placeholders; 580 has none (already right) | ★ wrong on 610, fine on the bench | yes |
| **B7** | init-args geometry: 4 fields at 580, not 9 | ★ the "derive it" path has nothing to read on the bench | yes |
| **B8** | the bootup allowlist is one hardcoded set; it is 6 at 580 and 8 different at 610 | ★ currently *safe* (too narrow), but wrong as data | yes |
| **B9** | suspend sentinel: 580 tests `==`, 610 tests `&` | ★ a teardown hang if a shadow is OR-ed | yes |
| **B10** | the 580 resume path is RPC-driven (`GSP_RUN_CPU_SEQUENCER`) | ? unknown — see §B10 | **reading, not HW** |
| **B11** | 121 untagged `ogkm:` citations in crate docs | ★ the class defect recurs | yes |

---

## B1 — Fix the oracle: `Guest::recv` is 610-shaped for **both** profiles

**Target:** `tests/src/gspworld.rs`, `impl Guest { pub fn recv(&mut self, ram: &mut FakeRam) }`
(currently at `:860-932`), and `pub struct Profile` (`:110-122`).

**What is wrong.** The mock guest derives its element count from the declared length —

```rust
let n = msg_len.div_ceil(self.msg_size);   // :892
…
self.rx_read_ptr = (self.rx_read_ptr + n) % self.msg_count;   // :930
```

— for **both** `P580` and `P610`. That is 610's algorithm
(`ogkm-610: message_queue_cpu.c:698-705`, consumed at `:838`). **At 580 the guest reads the
field**: `nElements = pMQI->pCmdQueueElement->elemCount`
(`ogkm-580: src/nvidia/src/kernel/gpu/gsp/message_queue_cpu.c:652-658`), and *that* is what
`msgqRxMarkConsumed(hQueue, nElements)` advances by (`:774`). 580 never derives the count from
`rpc.length`; its `msgLen` check at `:760-770` runs after the element is already consumed and
gates nothing.

`Profile.elem_count` exists and `P580` sets it to `Some(40)` — but only the *encoder* uses it
(`element.rs:377-379`). The decoder ignores it entirely. So the oracle currently cannot tell
the two protocols apart on the axis where they actually differ.

**Change.** In `recv`, when `self.p.elem_count == Some(off)`, take `n` from
`get32(&first, off)` and use it for the read loop, the checksum span under CC, and the
read-pointer advance. When `elem_count` is `None`, keep the derivation. Keep the derived value
as a separate local so B3's cross-check has something to compare against.

**The test that must FAIL before and PASS after:**

> `tests/tests/gsp_boot.rs::a_580_guest_consumes_by_elem_count_not_by_declared_length` — new.
>
> Build a `World::new(P580, MODEL_A)`, boot, and post a message; then, **in the guest's
> backing store only**, rewrite the element's `elemCount` word at +40 to `n + 1` and fix the
> checksum so the element is otherwise valid. Assert `guest.rx_read_ptr` advances by **`n +
> 1`**, not by `n`.
>
> Before B1 the mock advances by `n` and the assertion fails. After B1 it advances by `n + 1`
> and passes. Non-vacuity arm: the same test against `P610` must advance by `n` regardless of
> what is written at +40, because at 610 that offset is `rpc.sequence` and carries no count
> (`ogkm-610: message_queue_priv.h:52-67`).

★ **Do B1 first.** B2 and B3 are changes to production code whose failing tests can only be
written against an oracle that models 580 correctly. Fixing the oracle after the production
code would be marking your own homework.

---

## B2 — ★★★ The `elemCount > 16` safety clamp (plan §4.6, invariant GSP-S1)

**Targets:**
- `crates/kayfabe-gsp/src/element.rs`, `pub fn encode_message` (`:345-392`) — the write site,
  `if let Some(eo) = layout.elem_count_off() { … len.elements() … }` at `:377-379`.
- `crates/kayfabe-gsp/src/fault.rs`, `pub enum GspFault` — one new variant.
- `crates/kayfabe-gsp/src/element.rs`, `impl MsgLen { pub fn new }` (`:234-256`) — where the
  bound is already computable.

**What is wrong, and which version it is right for.** Nothing is *wrong* today by
derivation — `MsgLen::new` bounds `rpc_length` by `element_size_max - hdr`, so
`len.elements()` cannot currently exceed 16 with the bench's geometry. **The defect is that
the bound is nowhere checked at the field that carries it**, and at 580 that field is read
directly into an unbounded `memcpy` loop inside the guest kernel:

- staging buffer: `workAreaSize = (1 << GSP_MSG_QUEUE_ELEMENT_ALIGN) +
  GSP_MSG_QUEUE_ELEMENT_SIZE_MAX + msgqGetMetaSize()` = 4096 + 65536 + meta, from
  `portMemAllocNonPaged` (`ogkm-580: message_queue_cpu.c:132-134`);
- carve: `pCmdQueueElement = ALIGN_UP(pWorkArea, 4096)`,
  `pMetaData = pCmdQueueElement + GSP_MSG_QUEUE_ELEMENT_SIZE_MAX` (`:143-145`) ⇒ **exactly 16
  elements of staging, then the live `msgq` metadata**;
- loop: `for (i = 0; i < nElements; i++) { portMemCopy(pTgt, 4096, …, 4096); pTgt += 4096; }`
  (`:628, 648-650`) with no bound but the ring, which holds 63 elements
  (`msgqRxGetReadBuffer` stops at available, `ogkm-580: src/common/shared/msgq/msgq.c:673-693`).

⇒ an `elemCount` of 17 overwrites `pMetaData`; at the reachable maximum of 62 it writes
`(62 − 16) × 4096 = 188 416` bytes past a kernel allocation. This is the same defect class as
the C's unguarded `% s->q_msgcount` SIGFPE (`C:1615`) pointed at the guest instead of at QEMU.

**Change.** Add `GspFault::ElementCountOutOfRange { count: u32, max: u32 }`. In
`encode_message`, before writing `elem_count_off`, compute
`max = element_size_max / element_size` and refuse when `len.elements() > max`. Derive `max`
from the geometry — **do not write the literal 16**; CLAUDE.md rule 1 and the plan's own
"derive, don't declare" both apply, and the bench's 16 is `65536 / 4096`.

**The test that must FAIL before and PASS after:**

> `tests/tests/gsp_boot.rs::an_over_wide_element_count_is_refused_before_it_can_overrun_the_guest_staging_buffer`
> — new, and it needs a small addition to the oracle (see below).
>
> 1. Extend the mock guest with the real staging bound: give `Guest` a staging buffer of
>    `element_size_max` bytes and have `recv` **panic** (not refuse — the driver has no check
>    here, and modelling one would be modelling a behaviour it does not have) if `n *
>    msg_size` exceeds it. Cite `ogkm-580: message_queue_cpu.c:132-145, 648-650` in the panic
>    message.
> 2. Assert `encode_message(&P580.layout(), …, payload_of(17 * 4096), …)` returns
>    `Err(GspFault::ElementCountOutOfRange { count: 17, max: 16 })` — the exact variant and
>    both fields, per `testing_doctrine.md` §2, never `is_err()`.
> 3. **Bite check** (`testing_doctrine.md` §1c): with the refusal removed, the same message
>    must reach the guest and the oracle's panic must fire. Assert that with
>    `#[should_panic(expected = "staging")]` in a sibling test that constructs the element
>    by hand rather than through `encode_message`.
>
> Before B2, step 2's call returns `Err(MsgLenOutOfRange)` — a *different* variant, so the
> test fails on the assertion it makes rather than passing by accident. That distinction is
> the point: today the bound is enforced by a coincidence of derivation and reported under a
> name that describes something else.

★ **This item's justification does not rest on the bound being currently reachable.** It rests
on the coupling being invisible: any future path that sets `elemCount` from something other
than the same `MsgLen` — a continuation record, a CC layout, a replayed trace, a fuzz
harness — breaks it silently. Check the bound where the field is written.

---

## B3 — Receive must advance the ring by `elemCount`, not by a derivation

**Target:** `crates/kayfabe-gsp/src/boot.rs`, `impl GspFsm { pub fn service_command_queue }`
(`:681-740`), specifically:

```rust
let len = peek_len(&self.abi.element, &first, element_size, self.abi.element_size_max)?;  // :703
…
read_ptr = count.slot(read_ptr + len.elements()).index();   // :719
avail -= len.elements();                                    // :721
```

and `crates/kayfabe-gsp/src/element.rs::peek_len` (`:420-440`), which reads only
`rpc.length`.

**What is wrong, and which version it is right for.** `peek_len` implements 610's step 1. On
the **command** queue the guest is the producer, and at 580 it advances its own `writePtr` by
the `elemCount` it wrote (`ogkm-580: message_queue_cpu.c:482` sets
`pCQE->elemCount = GSP_MSG_QUEUE_BYTES_TO_ELEMENTS(msgLen)`, `:578` submits
`msgqTxSubmitBuffers(hQueue, pCQE->elemCount)`). So **the guest's `elemCount` is authoritative
for how far our `readPtr` must move.** For a conforming guest the two agree; for a
non-conforming one they do not, and a derivation-based consumer desynchronises the ring
permanently with nothing downstream to catch it — the seqNum check will then fail forever on
`>`, for which the driver's recovery branch does not exist (`ogkm-580: :699-714` handles only
`<`).

**Change.** Where `layout.elem_count_off()` is `Some`, `peek_len` (or a sibling
`peek_run_length`) returns **both** the field value and the derived value. `service_command_queue`
advances by the **field**, and refuses by name when the two disagree:
`GspFault::ElementCountMismatch { declared: u32, derived: u32 }`. Where `elem_count_off` is
`None`, the derived value is the only one and behaviour is unchanged.

★ The field must **also** be range-checked here, not only in `encode_message` — a guest can
write any `u32`. Reuse B2's `ElementCountOutOfRange` and bound it by `avail` as well, since a
count larger than the available elements is the "producer not finished" case the driver treats
as `NV_ERR_NOT_READY` (`ogkm-580: message_queue_cpu.c:632-644`) and `:709-714` already models.

**The test that must FAIL before and PASS after:**

> `tests/tests/gsp_boot.rs::a_command_whose_elem_count_disagrees_with_its_length_is_refused_and_does_not_move_the_ring`
> — new.
>
> Drive a bound FSM, have the mock guest **send** a command (the `Guest` already has a send
> path — `:824` writes `elem_count` when the profile has one), then corrupt the queued
> element so `elemCount = 2` while `rpc.length` implies 1, fixing the checksum. Assert:
> - `service_command_queue` returns `Err(GspFault::ElementCountMismatch { declared: 2, derived: 1 })`;
> - `binding.command_cursor().read_ptr` is **unchanged** — the plan's existing rule that a
>   refused message does not advance the cursor (`boot.rs:678-680`);
> - and the mirrored positive arm: with `elemCount = 2` **and** a 2-element `rpc.length`, the
>   cursor advances by exactly 2 and the command decodes.
>
> Before B3 the first arm advances by 1 and reports success, so both the error assertion and
> the cursor assertion fail. Non-vacuity: the same test under `P610` must not produce
> `ElementCountMismatch` at all, because there is no field to disagree with.

---

## B4 — 580 queues the init RPCs **before** bootstrap (plan §3.1a)

**Targets:**
- `crates/kayfabe-gsp/src/boot.rs`, `fn publish` (`:532-653`) — add a drain after the
  `INIT_DONE` post at `:650`.
- `crates/kayfabe-gsp/src/boot.rs`, `fn doorbell` (`:656-671`) — the `QueueNotBound` refusal
  at `:666-668`.
- `tests/tests/gsp_boot.rs::a_doorbell_on_an_unbound_queue_refuses_by_name_and_reads_zero_guest_ram`
  (`:424`) — its framing, not its assertion.

**What is wrong, and which version it is right for.** The plan's §3.1 B6 says the two init
RPCs (`GSP_SET_SYSTEM_INFO` 72, `SET_REGISTRY` 73) are sent *inside* `kgspBootstrap_TU102`,
after Booter Load. **That is 610** (`ogkm-610: kernel_gsp_tu102.c:576-585`, impl
`kernel_gsp.c:4686-4709`). At 580 `kgspSendInitRpcs` **does not exist**: the same two RPCs, in
the same order, are sent by `kgspQueueAsyncInitRpcs_IMPL`
(`ogkm-580: kernel_gsp.c:3753-3777`) from `kgspInitRm_IMPL` at `ogkm-580: kernel_gsp.c:4141`
— **before** `_kgspBootGspRm` (`:4184`), therefore before FWSEC, Booter Load, RISC-V start and
the status-queue link. Skipped only under SPDM (`:4123-4133`). And the doorbell rings with
them: `rpcSendMessage` calls `kgspSetCmdQueueHead_HAL` unconditionally after every submit
(`ogkm-580: kernel_gsp.c:425`), and `_kgspRpcSanityCheck` (`:281-321`) has no "is the GSP up"
gate.

Two consequences, both real:

1. **`QUEUE_HEAD(0)` is written twice while `QueueState` is `Unbound`, on every healthy 580
   boot.** The refusal *behaviour* is right — read no guest RAM — but its **classification** is
   not. `doorbell` returns `Err(GspFault::QueueNotBound)` out of `mmio_write`, which the
   device shell may escalate, and ledger row GSP-D4 plus the negative-trace class in plan §6.3
   treat this exact event as the stale-binding attack signature.
2. **At bind time the command ring already holds a backlog**: the guest's cmd `writePtr` is
   **2**, not 0. `publish` sets `cmd: RxCursor { read_ptr: 0 }` (`boot.rs:638`) and posts
   `INIT_DONE`, but never drains. It recovers only because the guest sends
   `SET_GUEST_SYSTEM_INFO` right after `INIT_DONE` and *that* doorbell drains all three in
   sequence order. Recovery by luck.

**Change.**
- In `publish`, after `self.init_done_posted = true`, call the same drain
  `service_command_queue` performs, and fold its `ServiceReport` into the caller's. (Note the
  borrow: `publish` is `&mut self` and already re-reads geometry from the binding, so this
  should be mechanical — but if the borrow checker forces a restructure, say so in the PR
  rather than reshaping `ServiceReport` to dodge it.)
- Split the refusal: keep `QueueNotBound` for a doorbell arriving with a **stale** binding
  (the C's actual defect — a previous life's GPA), and introduce a distinct, non-escalating
  outcome for a doorbell arriving **before any binding has ever existed in this device life**.
  The FSM already distinguishes these: `phase` is `Cold`/`FwsecRan` in the pre-bind case and
  `Halted` after a teardown. Whether that becomes a second `GspFault` variant or a
  `ServiceReport` field is a design call the executing agent should make against
  `ServiceReport`'s existing shape; **both must still read zero guest RAM.**

**The tests that must FAIL before and PASS after:**

> 1. `tests/tests/gsp_boot.rs::the_580_boot_order_rings_the_doorbell_twice_before_the_binding_exists`
>    — new. Drive the mock guest through the 580 order: GFW reads → **two commands queued +
>    two `QUEUE_HEAD` writes** → STARTCPU → mailbox pair → SEC2 Booter Load → `rx_link`.
>    Assert the two pre-bind doorbells produce the *pre-bind* outcome (not
>    `QueueNotBound`), that **zero** guest-RAM reads occurred during them (the existing
>    `FakeRam` read counter that test `:424` already uses), and that after E6 the two queued
>    commands appear in the `ServiceReport` **without a further doorbell**.
>    Before B4: the first assertion fails on the variant, and the last fails because
>    `report.commands` is empty until the next doorbell.
> 2. `tests/tests/gsp_boot.rs::a_doorbell_on_an_unbound_queue_refuses_by_name_and_reads_zero_guest_ram`
>    (`:424`) — **existing, must be narrowed, not weakened.** It currently asserts
>    `QueueNotBound` for any unbound doorbell. After B4 it must assert `QueueNotBound`
>    specifically for the *stale-binding* case (post-teardown, `phase == Halted`), which is
>    the case the measurement actually found
>    (`docs/reference/mode2_bench_lifecycle.md` §4, 508 log lines). ★ Narrowing here is
>    legitimate because the test's *subject* changes; it is **not** an instance of the
>    forbidden "narrow a test to make it pass" — the pre-bind case gains its own test in (1),
>    so total coverage strictly increases. State that in the test's doc comment.

---

## B5 — `ElementLayout` selection: there is no version predicate anywhere, and the interval is `>= 610`

**What is wrong.** The brief asked to change the predicate from `> 570` to `>= 610`.
**There is no predicate.** Grep confirms `ElementLayout::new` has exactly two callers, both in
test code: `tests/src/gspworld.rs:174` (via `Profile::layout`) and
`tests/tests/gsp_boot.rs:1238/1246/1251`. `crates/kayfabe-abi/src/versions.rs::DriverAbiTable`
carries only `version`, `map_dma` and `note`. `(570, 610]` appears solely as prose — in
`crates/kayfabe-gsp/src/element.rs:22`, `crates/kayfabe-gsp/src/lib.rs:36`, and three places
in the two design docs. So **no production code path selects an element layout at all**; the
S6 device shell would have nothing to ask.

**Targets:**
- `crates/kayfabe-abi/src/versions.rs` — add `pub enum GspElementWire { Pre610, From610_43_02 }`
  alongside `MapDmaWire`, a `gsp_element: GspElementWire` field on `DriverAbiTable`, and an
  entry to each of the three rows of `TABLES`. The existing `table_for` "newest entry `<=`
  version" mechanism then supplies the predicate for free, and the 610 row already exists
  (`versions.rs:99-105`) — it currently carries `note: "…carries no delta…"`, which stops
  being true.
- `crates/kayfabe-gsp/src/element.rs:22` and `crates/kayfabe-gsp/src/lib.rs:36` — the prose
  `(570, 610]` becomes `(595.84, 610.43.02]`.
- `tests/src/gspworld.rs`, `P580` / `P610` (`:130-159`) — these become *consumers* of the
  production table rather than the only definition. Keep them as fixtures if that is cheaper,
  but they must be derived from `kayfabe-abi`, or the version key is still untested.

**Which version each is right for.** The 48-byte form with `elemCount@40` holds at
575.64.05, 580.65.06, **580.159.04** (read: `ogkm-580: message_queue_priv.h:43-51`),
580.173.02, 590.44.01, 590.48.01, 595.44.02 and 595.84; the 16-byte MCTP form appears only at
**610.43.02** (read: `ogkm-610: message_queue_priv.h:52-67`). ⇒ the boundary is
**`major >= 610`**, and 580/590/595 are all on the 48-byte side. ★ Only the two endpoints
610.43.02 and 580.159.04 were read here; the other seven tags are relayed evidence (plan
§14.4). A `>= 610` predicate is safe under either reading because the *610* boundary is the
directly-verified one.

**The test that must FAIL before and PASS after:**

> `crates/kayfabe-abi/tests/oracle_layout.rs::the_gsp_element_wire_boundary_is_610_not_570`
> — new, and it must not compile before the change (the field does not exist), which is the
> strongest bite check available. Assert, by exact enum variant:
> - `table_for(575.64.5)`, `table_for(580.65.6)`, `table_for(BENCH_DRIVER)`,
>   `table_for(595.84.0)` ⇒ `GspElementWire::Pre610`;
> - `table_for(610.43.2)` and `table_for(999.0.0)` ⇒ `GspElementWire::From610_43_02`;
> - `table_for(609.255.255)` ⇒ `Pre610` — the off-by-one at the boundary, exactly as
>   `versions.rs:569` already does for `580.65.5` vs `580.65.6`. That existing test
>   (`selection_lands_on_the_exact_boundary_not_the_major`, `versions.rs:553`) is the pattern to copy.

---

## B6 — Transport words: 580 is already right; 610's are invented

**Targets:** `tests/src/gspworld.rs`, `P580` (`:130-138`) and `P610` (`:150-158`);
`crates/kayfabe-gsp/src/element.rs`, `TransportHdr::Mctp` (`:63-72`) — its doc comment.

**What is wrong.** `P580.mctp = None` is **already correct** and must not be "fixed" —
`ogkm-580` has no `mctp_format.h`, no `NVDM_TYPE_RM_RPC`, and its only MCTP is FSP
(`fsp_mctp_format.h`), SEC2 (`sec2_mctp_format.h`) and NVSwitch. Bytes @0–@7 of a 580 element
are `authTagBuffer[0..8]`, which a CC-off guest never reads. Leave it alone; add the citation
so the next reader does not "restore" a placeholder.

`P610` is wrong: `mctp: Some((0, 0x0000_0001, 4, 0x0000_10de))`, with a doc comment
(`:143-149`) that honestly says the words are placeholders. The real assembled values are
**`mctpHeader = 0xC000_0001`** and **`nvdmHeader = 0x2510_DE7E`**, derived in
`docs/reference/nvidia_abi_oracles.md` §6 from
`ogkm-610: src/nvidia/arch/nvalloc/common/inc/mctp_format.h:39-58` and `.../nvdm_format.h:61`.
Once B5 lands they belong in `kayfabe-abi` beside the layout, which is what that doc comment
already asks for.

★ **A constraint on what may be asserted.** 610 validates **only** the version nibble
(`== 1`) and the vendor id (`== 0x10de`) — `ogkm-610: message_queue_cpu.c:737-758`. SOM, EOM,
the sequence field and the NVDM **type** byte are unread. So **no test may assert that the
guest rejects a wrong SOM/EOM/SEQ/NVDM-type.** The existing mock enforces both whole words
(`gspworld.rs:911-913`), which is stricter than the driver; with real values that stops being
observable, but the *rule* must be written into the mock's doc comment so nobody later adds a
"the guest rejects a bad NVDM type" test, which would assert a behaviour the driver does not
have. This is the same rule §4.4 already applies to `signature`.

**The test that must FAIL before and PASS after:**

> `tests/tests/gsp_boot.rs::the_610_transport_words_are_the_drivers_own_and_a_wrong_one_is_refused`
> — new. Assert the exact constants (`0xC000_0001`, `0x2510_DE7E`) come out of the
> `kayfabe-abi` table, and that flipping **the version nibble** or **the vendor id** in a
> posted element produces `GuestRefusal::MctpViolation` from the 610 mock.
> Before B6 the constants assertion fails on the placeholder values.
>
> **And a negative test**, `…::the_610_guest_does_not_check_som_eom_or_nvdm_type`: flip only
> the SOM/EOM bits and the NVDM type byte, leaving version and vendor intact, and assert the
> guest **accepts** — with `ogkm-610: message_queue_cpu.c:737-758` cited. This is the arm that
> stops the mock from drifting stricter than the driver.
>
> ⚠ Writing the second test requires knowing the bit positions of SOM/EOM/type inside
> `0xC000_0001` / `0x2510_DE7E`. Those are in `mctp_format.h:39-58` / `nvdm_format.h:61`;
> **this brief did not re-derive them**, and `nvidia_abi_oracles.md` §6 is the place to take
> them from. If they do not decompose as expected, report that rather than inventing a
> decomposition — the whole reason the placeholders existed was that somebody did not.

---

## B7 — Init args: 4 fields at 580, and the geometry must come from Axis A

**Targets:** `crates/kayfabe-gsp/src/boot.rs`, `pub struct InitArgsLayout` (`:68-82`) and
`pub struct GspAbi` (`:87-102`); `tests/src/gspworld.rs`, `Profile::init_args` (`:186-201`).

**What is wrong, and which version it is right for.** The plan's §1.3 presents "the guest
declares its own geometry" as the high-leverage design choice. **That is 610 only.**
`ogkm-580: src/nvidia/inc/kernel/gpu/gsp/gsp_init_args.h:29-34` has exactly four fields —
`sharedMemPhysAddr, pageTableEntryCount, cmdQueueOffset, statQueueOffset` — populated at
`ogkm-580: kernel_gsp.c:4486-4489`, identical to nouveau r570. 610's has nine
(`ogkm-610: gsp_init_args.h:32-45`). ⇒ **queue geometry is not negotiated on the bench.** It
is compile-time: `queueElementHdrSize = 48`, `queueElementSizeMin = 4096`,
`queueElementSizeMax = 65536`, `GSP_MSG_QUEUE_HEADER_ALIGN = 4`,
`GSP_MSG_QUEUE_ELEMENT_ALIGN = 12` (`ogkm-580: message_queue_priv.h:91-104`).

The code is **already mostly right by accident**: `InitArgsLayout::element_hdr_size_off` is
`Option`, `P580` sets `declares_hdr_size: false`, and `GspAbi::element_size_max` is a field
rather than a constant. What is missing:

- `element_size_max` is hardcoded `4096 * 16` in the **test fixture** (`gspworld.rs:224`), not
  supplied by `kayfabe-abi`. Post-B5 it belongs in the table, keyed the same way, and so do
  `element_size_min`, `header_align` and `element_align` if anything ever needs them.
- `GSP_ARGUMENTS_CACHED` differs beyond the queue block: 580's lacks 610's
  `rmStateMonitorBufferArgs` and `bindataArgs`, and since `MESSAGE_QUEUE_INIT_ARGUMENTS` is
  the first member and grows 40 bytes at 610, **every subsequent offset differs**. Nothing
  reads them today. Add a comment at `InitArgsLayout` saying so, so the first person who needs
  one does not transcribe a 610 offset.

**The test that must FAIL before and PASS after:**

> `tests/tests/gsp_boot.rs::the_bench_driver_declares_no_queue_geometry_and_the_fallback_supplies_it`
> — new. Assert that the `GspAbi` built for `BENCH_DRIVER` has
> `init_args.element_hdr_size_off == None` **and** `element.hdr_size() == 48` **and**
> `element_size_max == 65536`, i.e. the fallback path produced a complete geometry with
> nothing read from the guest. Then the mirror: a 610-keyed `GspAbi` has
> `element_hdr_size_off == Some(32)` and takes its header size from a guest write, which the
> mock can vary — assert that varying it changes `hdr_size()`.
>
> Before B5+B7 the first half cannot even be written, because nothing maps `BENCH_DRIVER` to
> an element layout; that is the failure.

---

## B8 — The bootup-window allowlist is version-keyed data, not one set

**Target:** `crates/kayfabe-gsp/src/rpc.rs`, `impl RpcFunction { pub fn allowed_in_bootup_window }`
(`:196-198`), currently `matches!(self, RpcFunction::InitDone)`, with a doc comment (`:191-194`)
citing an "eight-entry allowlist (`ogkm: kernel_gsp.c:1419-1440`)".

**What is wrong.** The citation is 610's, and the count is version-split:

| | 580.159.04 (`ogkm-580: kernel_gsp.c:1469-1474`) | 610.43.02 (`ogkm-610: kernel_gsp.c:1424-1431`) |
|---|---|---|
| entries | **6** | **8** |
| `GSP_RUN_CPU_SEQUENCER` | ★ present, and **first** | **absent** |
| `UCODE_LIBOS_PRINT`, `GSP_LOCKDOWN_NOTICE`, `GSP_POST_NOCAT_RECORD`, `GSP_INIT_DONE`, `OS_ERROR_LOG` | present | present |
| `PFM_REQ_HNDLR_STATE_SYNC_CALLBACK`, `GSP_LOAD_EXEC_GENERIC_BOOTLOADER`, `GSP_LOAD_EXEC_HS_BINARY` | absent | present |

The **implementation is currently safe** — `InitDone` alone is a strict subset of both — so
this is not a correctness bug. It is a *data* bug: the doc asserts a version-independent
eight-entry list that exists at neither tag as described, and any future widening (an agent
adding `UCODE_LIBOS_PRINT` for log forwarding, say) would be done against the wrong list.

**Change.** Move the allowlist into the same version-keyed table B5 creates, as the
**intersection** of the tags we support unless a specific version's entry is needed:
`{UCODE_LIBOS_PRINT, GSP_LOCKDOWN_NOTICE, GSP_POST_NOCAT_RECORD, GSP_INIT_DONE,
OS_ERROR_LOG}`. Keep `allowed_in_bootup_window` returning `InitDone` only if nothing needs
more — but make it read from the table, with both citations, so widening it is a data edit
with a version behind it.

**The test that must FAIL before and PASS after:**

> `tests/tests/gsp_boot.rs::the_bootup_window_allowlist_is_version_keyed_and_post_event_is_on_neither`
> — new. Assert `POST_EVENT` (0x1003) is refused in the bootup window under **both** the 580
> and 610 keys; assert `GSP_RUN_CPU_SEQUENCER` (0x1002) is allowed under the 580 key and
> refused under the 610 key. The second assertion cannot be written today — there is one
> global predicate — which is the failure.
>
> ★ **Do not** turn this into "we emit `GSP_RUN_CPU_SEQUENCER`". The allowlist says the guest
> would *accept* it during boot at 580; §B10 is where whether we must *send* it is decided.

---

## B9 — The suspend sentinel is exact equality at 580

**Targets:** `tests/src/gspworld.rs`, `FakeGspModel::encode`, arm `GspReg::GspFalconMailbox0`
(`:405-411`); and — when it is written — the real GA10x `GspModel`. There is currently **no**
production `impl GspModel` anywhere (grep: one impl, in `tests/src/gspworld.rs:358`).

**What is wrong, and which version it is right for.**

- `ogkm-580: kernel_gsp_tu102.c:1226-1238` — `return (mailbox == 0x80000000);` **exact
  equality**, constant inlined, no `INTERRUPT_PROCESSOR_SUSPENDED_VALUE` symbol in the tree.
- `ogkm-610: kernel_gsp_tu102.c:333, 348` —
  `#define INTERRUPT_PROCESSOR_SUSPENDED_VALUE 0x80000000` and
  `return (mailbox & INTERRUPT_PROCESSOR_SUSPENDED_VALUE) != 0;` — a **mask**.

⇒ we must **write the whole value, never OR the bit onto a shadow**. A shadow still holding a
boot-args half with bit 31 set reads as suspended at 610 and hangs the teardown poll *forever*
at 580 — and 580 polls it from two places: after fn-47 (`ogkm-580: kernel_gsp.c:4310`) and as a
**bootstrap liveness fallback** (`ogkm-580: kernel_gsp_tu102.c:551`,
`kflcnIsRiscvActive || _kgspIsProcessorSuspended`).

**The crate side is already correct by construction** and should be left alone:
`GspFsm::observe` exposes `suspended: bool` (`boot.rs:365`) and the encoding lives behind
`GspModel`, which is exactly the seam plan §3.5 designed. `FakeGspModel` returns
`self.suspend_sentinel` (a whole value) rather than OR-ing, which is right. What is missing is
that **nothing tests the distinction**, so the first real `GspModel` can get it wrong silently.

**The test that must FAIL before and PASS after:**

> `tests/tests/gsp_boot.rs::the_suspend_sentinel_replaces_the_mailbox_shadow_rather_than_setting_a_bit`
> — new. Boot, write a boot-args low half with **bit 31 already set** (e.g. `0x8000_1234` —
> a legal GPA low half), reach `Suspending` via fn-47, and assert `mmio_read` of
> `GspFalconMailbox0` returns **exactly** the sentinel, not `0x8000_1234`. Then the converse:
> while **not** suspended, assert it returns exactly `0x8000_1234` and that a 580-shaped
> `== sentinel` comparison is therefore **false** — which is what the guest's poll does.
>
> Before B9 the test can be written and will pass against `FakeGspModel` (it already
> replaces). ★ **So this item's honest status is: the test is the deliverable, not a fix.**
> It is a *pin* that makes the invariant fail-fast when the real `GspModel` lands, and the
> brief says so rather than inventing a defect to justify a change. Add a second fake model
> that OR-s, and assert the pin catches it — that is the bite check, and it is the part that
> fails today because no such assertion exists.

---

## B10 — ⚠ The 580 resume path is RPC-driven. Status: OPEN, and it is a reading task

**No code change is specified here, because none can be specified honestly yet.**

**What was relayed:** *"580 has a GSP-resume handoff 610 never reads —
`NV_PGC6_BSI_SECURE_SCRATCH_14._BOOT_STAGE_3_HANDOFF` plus SEC2 `FALCON_MAILBOX0`."*

**The "610 never reads" half is wrong**, and this brief reports that rather than quietly
correcting it. 610 has the identical `_kgspIsReloadCompleted` reading the identical register
and field at `ogkm-610: src/nvidia/src/kernel/gpu/gsp/arch/turing/kernel_gsp_falcon_tu102.c:441-452`,
used at `:471` inside `kgspExecuteCoreResume_TU102`.

**What is actually version-split is how it is reached**, which is a sharper finding:

| | 580.159.04 | 610.43.02 |
|---|---|---|
| where | `kgspExecuteSequencerCommand_TU102`, `case GSP_SEQ_BUF_OPCODE_CORE_RESUME` (`ogkm-580: kernel_gsp_tu102.c:913-960`) | `kgspExecuteCoreResume_TU102` (`ogkm-610: kernel_gsp_falcon_tu102.c:455-…`) |
| trigger | ★ **only** `_kgspRpcRunCpuSequencer` ← the `GSP_RUN_CPU_SEQUENCER` event — **something we would have to send** | locally, from `kernel_gsp_falcon_tu102.c:563` and `kernel_gsp_falcon_ga102.c:401` — **no RPC** |

⇒ the owner's standing requirement that **GPU restart (idle → back) must work without a
bolt-on** is affected: at 580 a faked GSP that never emits `GSP_RUN_CPU_SEQUENCER` cannot
drive `CORE_RESUME`, whereas at 610 that path needs nothing from us.

**What is not established, and why no change is specified.** Nothing here traced a restart
scenario we must support down to a `CORE_RESUME` sequencer buffer. The buffer's *contents*
come from GSP-RM firmware, which is not in the open tree — so the open source can show that
the path exists and how it is triggered, but not whether the firmware ever triggers it on a
path we care about. **Specifying an emitter now would be guessing.**

**The task (reading, not hardware):** enumerate the callers of consequence of
`kgspExecuteSequencerBuffer_IMPL` (`ogkm-580: kernel_gsp.c:5259`) and of every
`GSP_SEQ_BUF_OPCODE_*` case (`:5293-5394`, plus `kernel_gsp_tu102.c:928` and
`kernel_gsp_ga102.c:151`), and determine whether any *host-initiated* resume in the plan's
scope reaches `CORE_RESUME`. Recorded as plan §11-O7a. If the answer is yes, this becomes a
new emitter and a new milestone — not a patch.

---

## B11 — The class fix in the code: tag every `ogkm:` citation

**Target:** every doc comment in `crates/kayfabe-gsp/src/*.rs` (and the handful in
`crates/kayfabe-abi/src/`). Grep `ogkm:` — the crates carry **121**, and **every one
is a 610 path with a 610 line number**, several of which do not exist at 580 (the whole
`msgq` library moved to `src/common/shared/msgq/`, and e.g. `kgspWaitForRmInitDone` is
`kernel_gsp.c:5214` at 580 vs `:6264` at 610).

**Change.** Mechanical rewrite to `ogkm-610:` where the claim was checked at 610, and
`ogkm-580:` / both where §14.1 and §14.3 of the plan record a re-read. Per plan §0.1 rule 4,
`[src]` unqualified now means *checked at both and identical*.

**The test that must FAIL before and PASS after:** a CI grep gate, in
`.github/workflows/ci.yml` beside the existing boundary / VMM-vocabulary / unsafe-surface
gates: **fail if `ogkm:` appears without a version suffix** in `crates/**/*.rs` or `docs/**`.
Before B11 it fails on 121 hits in `crates/` alone; after, on zero. ★ This is the only item that prevents the
recurrence rather than fixing an instance, and it is the cheapest thing in the brief.

⚠ Note the plan's own §13.1 item 3: `ci.yml` was previously declared "not this work's to
change". If that still holds, this gate should be raised as its own task rather than
smuggled in — but it should be raised, because without it the tags rot back to bare `ogkm:`
within a milestone.

---

## 2. Where this brief is uncertain

Stated plainly, because a brief that hides its gaps is worse than one that has them.

1. **Seven of the nine break-interval tags are relayed, not read.** Only 580.159.04 and
   610.43.02 are vendored here. 575.64.05, 580.65.06, 580.173.02, 590.44.01, 590.48.01,
   595.44.02 and 595.84 come from another agent's probe. B5's `>= 610` predicate is safe
   under either reading because the 610 boundary is the directly-verified one — but the claim
   "580/590/595 are all on the 48-byte side" rests on evidence this brief did not see.
2. **The MCTP/NVDM word decomposition was not re-derived.** B6 takes `0xC000_0001` and
   `0x2510_DE7E` from `docs/reference/nvidia_abi_oracles.md` §6. This brief confirmed that
   `mctp_format.h`/`nvdm_format.h` exist at 610 and do not at 580; it did **not** re-assemble
   the words from their bit fields. B6's second test needs that decomposition and may find it
   does not hold.
3. **B10 has no clean seam, and this is the real one.** At 580 a GSP resume is driven by an
   RPC *we* would have to send; at 610 it is driven locally by the guest and needs nothing
   from us. That is not a layout difference a version table absorbs — it is a difference in
   **who initiates**, and the plan's whole premise is that the GSP layer stays version-agnostic.
   A faked GSP that must emit sequencer buffers at 580 and must not at 610 has a
   version-conditional *behaviour*, which `four_axes_of_variation.md` §5 rule 1 explicitly
   forbids in a logic crate. There is no proposal here for resolving that, because resolving
   it requires first knowing whether the 580 path is ever taken (O7a) — and if it is, the
   honest options are a version-keyed *capability* on the FSM or a loud refusal, both of which
   are design decisions above this brief's pay grade.
4. **B4's drain-after-publish may not be a clean edit.** `publish` and
   `service_command_queue` both take `&mut self` and both clone the geometry out of the
   binding; whether the drain composes without restructuring `ServiceReport` could not be
   determined without compiling, which this brief may not do. If it does not compose, the
   executing agent should report that rather than reshaping `ServiceReport` to make it fit.
5. **B9 fixes nothing.** Its deliverable is a pin plus a bite check against a
   deliberately-wrong second fake model. Called out because an item in a correction brief that
   changes no production behaviour is easy to mistake for a bug fix, and it is not one.
6. **The `elemCount > 16` bound is currently unreachable through `encode_message`**, because
   `MsgLen::new` already bounds `rpc_length`. B2 is therefore defence-in-depth at the field
   rather than a live exploit today. It is ranked first anyway — the coupling between the
   derivation and the field is invisible, the failure mode is guest kernel memory corruption,
   and the check is four lines. If the executing agent disagrees with that ranking, the
   disagreement is legitimate; the *change* is not optional.
7. **Nothing here was compiled, and no test named above exists yet.** Every "must FAIL before"
   claim is a reasoned prediction from reading the code, not an observation. The first thing
   the executing agent should do with each item is confirm the test fails for the reason
   stated — and if it fails for a different reason, that is data about this brief, not a
   detail to work around.
