# V3 security audit — plan and gate

**STATUS: PLAN, 2026-10-03 (owner request).** Nothing in this document has been audited yet. It
records what the audit must cover, how, and when. Findings go into their own dated documents and
are linked from §6.

## 0. Why, and when

**Owner, 2026-10-03:** *"Maybe worth doing a security audit. Meanwhile continue on display."* The
owner listed ten areas (§2), ending with: *"if security errors can be prevented at compile time, at
design, or panics in runtime in something we overlooked and is abused rather than a breakout is very
much worth it right? Thats the idea of unsafe code verifying safe code."*

The audit runs in two stages.

- **Stage 1, now, alongside the display work.** These areas do not move while the display branches
  change: §2.1 (no host pointers in safe code), §2.5 (the threat model), §2.6 (the RM call
  inventory), §2.7 (the page-table walker) and §2.10 (bug classes from history). They are read-only
  and conflict with no branch.
- **Stage 2, after the display and security branches merge and before any public binary.** That
  covers `v3-gop`, the broker with its GPU-copy rung and cursor, `x11-dispsw`, `v3-sec-nonpriv`, the
  P1/P2 rework and `v3-scratch-bound`. Stage 2 covers §2.2, §2.3, §2.4, §2.8 and §2.9. Auditing
  code that is about to change produces findings that go stale, and the broker adds new attack
  surface (a socket parser, fd passing, exported dma-bufs).
- **Gate:** no public binary (`V3_SWEEP_AND_INSTALL.md`, the install artifact) ships before every
  stage-2 blocker is closed or recorded by the owner as accepted.

## 1. The threat model, by role

There is no isolate in v3, so nvkvm-pv's isolate boundary no longer applies. The roles remain:

| role | trust | what must hold |
|---|---|---|
| guest userspace | untrusted | It cannot reach another guest process's memory, the guest kernel's memory, kayfabe's memory or any host memory, through any path (GPU work, mappings, RM calls). Owner rule A.9. |
| guest kernel / guest root | untrusted to the host | It can harm only its own VM: crash it, hang it, use its own resources. It cannot reach host memory, other VMs, kayfabe-owned memory, or host resources beyond a bounded, documented amount (§2.4). |
| the VMM process (QEMU + kf3) | the host trust boundary | kayfabe grants the guest nothing beyond the VMM's own unprivileged rights. Sandboxing the VMM is not kayfabe's job (owner, 2026-10-03), but kayfabe must not depend on the VMM being privileged, and must not make its host channels privileged when it is (`v3-sec-nonpriv`: `NV01_ROOT_NON_PRIV`). |
| the display broker | a separate, less trusted process | It sees only finished frames and the cursor image kayfabe gives it; never guest memory (§L). kayfabe validates everything it receives. |
| other tenants on the same host GPU | must be isolated | No guest-steered operation reaches their memory. Shared-resource exhaustion is bounded per VM. |
| host RM and driver | trusted | Their behaviour is cited from ogkm-580; anything closed-firmware is marked UNVERIFIED. |

## 2. The ten areas

Each area lists its method and what already exists. **Stage** says when it runs.

### 2.1 No host-process pointers in safe code (stage 1)

- **Check:** no raw VMM virtual address, raw pointer, `mmap` or `transmute` in safe Rust. Window
  offsets (BAR offsets, store offsets) are used instead of addresses.
- **Already enforced in CI:** the host-pointer gate (no `*mut`, `*const`, `NonNull` or `transmute`
  outside `*_unsafe.rs`), the unsafe-surface gate (every unsafe file is named `*_unsafe.rs`), and
  the unsafe-containment gates (forbid inheritance, the named crates, a ratchet).
- **Audit:** confirm the gates cannot be bypassed (macros, `include!`, build scripts, C code in
  `kf3.c`), and that no safe API returns or accepts a host address in disguise, such as a `u64`
  that is really a pointer.

### 2.2 Unsafe code bounds-checks every input (stage 2)

- **Check:** every safe→unsafe boundary checks offset and length (including overflow of
  `offset + length`) against the real object, and tracks references and lifetimes. Unsafe functions
  reachable only from other unsafe code may skip this, but must then not be exposed to safe code.
- **Method:**
  - an inventory of every `*_unsafe.rs` public item with its precondition;
  - a test per precondition that a violating call is refused, so the test can fail;
  - Miri on the unit tests that exercise unsafe code;
  - bounded model checking (Kani) of the bounds arithmetic in the hottest wrappers (window
    placement, store slices, the PRAMIN pool).

### 2.3 Races and interleavings (stage 2)

- **Check:** no unlock, relock and changing-underneath class of bug; a value read under one lock
  is never used under another without revalidation; no time-of-check to time-of-use gap on guest
  memory (the guest can write any guest page at any time).
- **Method:**
  - a lock and ordering inventory (the repo already ranks its locks, `kayfabe-util` lock ranks);
  - loom or shuttle models of the lock-free parts: the doorbell wakeup word (an open P1 gate item),
    the FrameRing state word, and the PRAMIN slot generations;
  - re-read every guest-memory read for double-fetch.

### 2.4 Where the guest can cause denial of service (stage 2)

- **Check:** every guest-steered allocation of host memory, host kernel objects, host VRAM, host
  BAR1, file descriptors, threads or log volume is bounded per VM and refused by name past the
  bound.
- **Known so far:**
  - the scratch memfd bound (`v3-scratch-bound`);
  - mapping-count growth (the same branch);
  - display-SW twins (caps being added on `v3-dispsw-exp`);
  - the window-advice and kernel-mapping counters;
  - unbounded log lines found in several reviews.
- **Output:** one table of every guest-steered resource, with its cap and the cap's test.

### 2.5 The security model document (stage 1)

- Refresh the model for v3 from nvkvm-pv's and kayfabe's earlier models: drop the isolate, keep the
  roles of §1, and list the attacks in and out of scope.
- **Sources:** nvkvm-pv `docs/audit/` (seven audits, including `audit-guest-pointers.md` and the
  full security and reliability audit), kayfabe `docs/audits/`, and the 2026-10-03 client and
  single-store audit (its findings P0–P2 are in work on `v3-sec-nonpriv` and in the P1/P2 design).

### 2.6 Every RM call and ioctl kayfabe makes (stage 1)

- **Check:**
  - every ioctl and RM call kf3 issues to the host driver is listed;
  - every struct sent is built by kayfabe (intent constructed from the guest, not forwarded as
    nvkvm-pv did);
  - whatever is forwarded carries no CPU virtual address and can only affect the guest's own GPU
    channel;
  - an allowlist applies, there are no privileged blobs, and every action works for an
    unprivileged client;
  - every reply is parsed with bounds.
- **Output:** the inventory table, generated from source where possible so it cannot go stale.

### 2.7 The GPU page-table walker (PTX) (stage 1)

- **Check:** the walker treats guest page tables as hostile. It must hold against malicious tables
  (cycles, out-of-range pointers, oversized levels, aliasing), against the guest rewriting tables
  while they are walked, and it can neither deadlock nor loop without bound. An honest guest kernel
  never writes invalid tables, so this is breakout protection only.
- **Method:**
  - read the PTX and its host-side bounds (store length, pointer checks);
  - a property or fuzz test that feeds hostile tables to the walker's host-side model, with a
    known-positive;
  - confirm every bound has a test that can fail.

### 2.8 Integer, cast and compiled-out checks (stage 2)

- **Check:** truncating and sign-changing casts on guest-controlled values, unchecked arithmetic,
  and security checks that exist only in `debug_assert!` or behind a feature, so a release build
  drops them.
- **Method:** Clippy with `cast_possible_truncation`, `cast_sign_loss`,
  `arithmetic_side_effects` and `debug_assert_with_mut_call` on the guest-facing crates
  (ratcheted); a grep audit of `debug_assert`, `cfg(test)` and `cfg(debug_assertions)` near
  validation code; release-mode test runs.

### 2.9 Settle what can be settled; say what is uncertain (stage 2)

- Prefer prevention at compile time (types such as `GuestOffset` and `StoreSlice` that cannot be
  built unchecked), then at design, then a runtime refusal or panic over silent continuation. A
  panic in an overlooked case is a crash, not a breakout.
- Every finding is either fixed with a test that can fail, settled by a model check or a written
  argument, or recorded as an uncertainty with its reason.

### 2.10 Bug classes from history (stage 1)

- Collect the classes nvkvm-pv and kayfabe actually hit; there are memory notes and audit documents
  for many. Then check each against v3.
- **Classes already known:**
  - guest pointers trusted;
  - a missing table row that never denies;
  - capture-derived tables expiring;
  - a CAS failure read as ownership;
  - a check that reports but does not gate;
  - a refusal on a wait path that forges a completion;
  - stale evidence read as current;
  - unbounded logs.

## 3. Outputs

- `docs/design/V3_SECURITY_MODEL.md` (§2.5) and one findings document per stage, each finding with
  its severity, its proof or test, and its status.
- New CI gates where an area can be enforced mechanically (§2.1, §2.6, §2.8).
- A residual-risk section the owner signs off on before the first public binary.

## 4. Already found (2026-10-03) and in work

| finding | where | status |
|---|---|---|
| P0: host twins were privileged when QEMU runs as root | client audit 2026-10-03 | fix on `v3-sec-nonpriv` (`NV01_ROOT_NON_PRIV` + a per-birth tripwire) |
| P1: identity windows in every twin address space | same | design in progress: windows only where Translated work runs |
| P2: kayfabe's Translated rings writable from the guest kernel's space | same | design in progress: Translated in its own address space |
| Scratch memfd host-RAM amplification; mapping growth | owner question | fixed on `v3-scratch-bound`, merge bar passed |
| Display-SW twins uncapped | `v3-dispsw-exp` review | caps in progress |

## 5. Not in scope

- Sandboxing the VMM process (owner, 2026-10-03).
- Confidential computing and attestation (`OWNER_RULINGS.md` §H).

## 6. Findings documents

None yet.
