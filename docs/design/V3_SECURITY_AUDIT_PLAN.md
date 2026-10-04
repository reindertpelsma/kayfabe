# V3 security audit — plan and gate

**STATUS: PLAN, 2026-10-03 (owner request). Stage 1 written up the same day (§6); stage 2 not
started.** This document records what the audit must cover, how, and when. Findings go into their
own dated documents and are linked from §6; the stage-1 blockers and majors are in §4.

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

There is no isolate in v3, so nvkvm-pv's isolate boundary no longer applies. The roles remain.
The table states what must hold; which rows hold today is in `V3_SECURITY_MODEL.md` §9 (stage 1:
not all of them do).

⊘ **Corrected 2026-10-03 (stage 1, S1-29).** The VMM row named `NV01_ROOT_NON_PRIV` as the
mechanism. RM rewrites every userspace root allocation to `NV01_ROOT_CLIENT`
(`ogkm-580: src/nvidia/arch/nvalloc/unix/src/escape.c:396-403`), so that class changes nothing. The
mechanism on `v3-sec-nonpriv` is the effective-`CAP_SYS_ADMIN` bracket around each channel birth plus
the `PRIVILEGED_CHANNEL` reply tripwire; the row below now says so.

| role | trust | what must hold |
|---|---|---|
| guest userspace | untrusted | It cannot reach another guest process's memory, the guest kernel's memory, kayfabe's memory or any host memory, through any path (GPU work, mappings, RM calls). Owner rule A.9. |
| guest kernel / guest root | untrusted to the host | It can harm only its own VM: crash it, hang it, use its own resources. It cannot reach host memory, other VMs, kayfabe-owned memory, or host resources beyond a bounded, documented amount (§2.4). |
| the VMM process (QEMU + kf3) | the host trust boundary | kayfabe grants the guest nothing beyond the VMM's own unprivileged rights. Sandboxing the VMM is not kayfabe's job (owner, 2026-10-03), but kayfabe must not depend on the VMM being privileged, and must not make its host channels privileged when it is (`v3-sec-nonpriv`: the effective-`CAP_SYS_ADMIN` bracket around each channel birth and the `PRIVILEGED_CHANNEL` reply tripwire; libcuda's own channels and the other RM calls are not yet covered, S1-22). |
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
- ★ **2026-10-04, `v3-sec-perimeter`** (`docs/design/V3_SEC_PERIMETER.md`): the gates of
  `.github/workflows/perimeter.yml` replace the lexical ones above (`ci.yml`'s retire at C11).
  They are G1 the compiler location gate, G3 the tokenizer, G4 the manifests and the resolved
  graph, G5 the export table, G6 the size ratchet and G7 `kf3.c` in CI. G2, the deny layout, is
  P2 hygiene. Each runs its known positives first.

### 2.2 Unsafe code bounds-checks every input (stage 2)

- **Check:** every safe→unsafe boundary checks offset and length (including overflow of
  `offset + length`) against the real object, and tracks references and lifetimes. Unsafe functions
  reachable only from other unsafe code may skip this, but must then not be exposed to safe code.
- **Method:**
  - an inventory of every `*_unsafe.rs` public item with its precondition — ★ 2026-10-04:
    `docs/design/PERIMETER_EXPORTS.md`, generated from rustdoc and gated (E1-E14);
  - a test per precondition that a violating call is refused, so the test can fail — ★ E3 (each
    check of an `OK` row names a test that runs in CI and names the item) plus E3c (nightly
    cargo-mutants: an `OK` row has no missed mutant);
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
| P0: host twins were privileged when QEMU runs as root | client audit 2026-10-03 | fix on `v3-sec-nonpriv`: the effective-`CAP_SYS_ADMIN` bracket around each birth + the `PRIVILEGED_CHANNEL` (bit 5) reply tripwire (corrected 2026-10-03, S1-29; `NV01_ROOT_NON_PRIV` is rewritten by RM) |
| P1: identity windows in every twin address space = **S1-21 (blocker)**: in-guest isolation A.9 does not hold | same; `crates/kf-qemu/src/mem.rs:1755-1762` | design in progress: windows only where Translated work runs |
| P2: kayfabe's Translated rings writable from the guest kernel's space = **S1-23 (major)** | same; `crates/kf-chan/src/translated.rs:26`, `crates/kf-qemu/src/mem.rs:487-503` | design in progress: Translated in its own address space |
| Scratch memfd host-RAM amplification; mapping growth | owner question | fixed on `v3-scratch-bound`, merge bar passed |
| Display-SW twins uncapped | `v3-dispsw-exp` review | caps in progress |

**Stage-1 blockers and majors (2026-10-03).** Every row is open; the location, the check that
catches it and who can trigger it are in `docs/audits/2026-10-03-v3-stage1.md` under the same ID.
P1 and P2 are the rows above.

| finding | where | status |
|---|---|---|
| **S1-20 (blocker):** USER host channels refusing physical-mode and privileged work is the host boundary, and it is untested in v3 | `crates/kf-host/src/channel.rs:603-611`; closed GSP firmware | open: client-audit box test T-PHYS-CE, both privilege arms, every family |
| S1-01: the four lexical unsafe gates can all be passed at once by rustfmt-clean code | `.github/workflows/ci.yml:1462-1473`, `:676-682` | closed by replacement on `v3-sec-perimeter` (2026-10-04): the compiler location gate, the tokenizer, M1-M9 |
| S1-02: the host-pointer gate sees only pointer type names | `.github/workflows/ci.yml:734-751` | open |
| S1-03: the console frame's host address crosses safe code as a `usize`; `Kf3Frame` has no length | `crates/kf-cuda/src/display.rs:44-53`, `crates/kf-qemu/src/ffi_unsafe.rs:513` | open |
| S1-04: kf-cuda's public safe API takes raw device addresses | `crates/kf-cuda/src/driver_unsafe.rs:672-842` | open |
| S1-05: under unified addressing or HMM the in-process CUDA contexts can address the VMM; the compose kernel has no in-kernel bound | `crates/kf-cuda/src/display.rs:122-154`; `cuda/display/kf_scanout.ptx` | open |
| S1-06: the v3 unsafe-soundness ledger walks the v2 crates | `crates/kf-linux-raw/tests/unsafe_naming.rs:41` | closed on `v3-sec-perimeter` (2026-10-04): the export table is the ledger |
| S1-22: under a root VMM only births are de-privileged; other RM calls and both libcuda contexts run as admin | `crates/kf-qemu/src/device.rs:303`; `crates/kf-cuda/src/driver_unsafe.rs:364-429` | open |
| S1-24: kayfabe's RPC and control decoders are reachable from unprivileged guest userspace | `crates/kf-rm/src/rmrpc/` | open (stage 2 fuzz/property tests) |
| S1-25: shared host-GPU resources are not bounded per VM | `V3_SECURITY_MODEL.md` §7 | open (§2.4 table) |
| S1-26: `THE_CONSTRAINTS.md` §20's walker placement argument is stale | `THE_CONSTRAINTS.md:1531-1542` | open |
| S1-27: copy-then-check is the only guard against guest writes underneath; only the PTX side is gated | §39(a); the Rust guest-memory readers | open (stage 2, §2.3) |
| S1-28: this plan omits packaging, install and supply chain | §2 | open: a proposed stage-2 area for the owner |
| S1-40: RM companion size fields (`paramsSize`, NVOS02 `limit`) are a safe-caller contract | `crates/kf-linux-raw/src/chardev_unsafe.rs:365-377`, `:565-776` | open |
| S1-41: no host-side allowlist of RM controls and classes | `crates/kf-host/src/lib.rs:793`, `:1029`; `crates/kf-abi/src/hostabi.rs:849` | open |
| S1-42: NVENC session slots are GPU-wide with no per-VM cap | `crates/kf-qemu/src/chan.rs:2551-2598` | open |
| S1-60: VER3 (Hopper, Blackwell): an unmapped big PTE does not veto stale 4 KiB PTEs | `cuda/walk/kf_walk.cu:363-368` | open; a blocker for any Hopper or Blackwell release |
| S1-61: walk resources are shared per refresh, so one space can fail every batched space's walk | `cuda/walk/kf_walk.cu:1849-1852`; `crates/kf-mem/src/vasmgr.rs:396-434` | open |
| S1-80: kf3 is hot-unpluggable and its exit neither joins threads nor deletes its bottom half | `qemu/hw/misc/kf3/kf3.c:858-912` | open |
| S1-81: the late-invalidate tripwire is never called | `crates/kf-trap/src/shadow.rs:186-195` | open |
| S1-82: twins' RM-owned context buffers take host VRAM outside `fb-mb`, uncapped | `crates/kf-host/src/channel.rs:57-97`; `crates/kf-qemu/src/cardbudget.rs:1-19` | open |
| S1-83: the hostile-input instruments (fuzz, tsan, mutants, the adversarial guest kernel) were retired at the v3 cutover | `.github/workflows/ci.yml:17-19` | open; first instrument on `v3-sec-perimeter` (2026-10-04): nightly mutants over the perimeter (E3c) |

## 5. Not in scope

- Sandboxing the VMM process (owner, 2026-10-03).
- Confidential computing and attestation (`OWNER_RULINGS.md` §H).

## 6. Findings documents

- **Stage 1, 2026-10-03:** `docs/audits/2026-10-03-v3-stage1.md`. Areas §2.1, §2.5, §2.6, §2.7 and
  §2.10, read at `12a526df` (code-identical to `7c9234d2`): 67 findings, of which 2 are blockers,
  22 major, 26 minor and 17 info. Each has its location, the check or test that catches it, who can
  trigger it, and its status. The document also carries each area's inventory: the kf3 link graph and
  gates (§2.1), every host-driver call (§2.6), the walker's bounds and tests (§2.7), and 33 bug classes
  from history (§2.10).
- **The security model:** `V3_SECURITY_MODEL.md` (STATUS: LIVE, 2026-10-03), the §2.5 output, refined
  by the other stage-1 areas. Its §9 says which rows of §1 above hold today.
- **Stage 2:** not started. §4 of the stage-1 document lists what it inherits.
