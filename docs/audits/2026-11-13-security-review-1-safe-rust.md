# Security review 1 of 4: the safe Rust side of `crates/kf-*`

STATUS: RESEARCH, 2026-11-13 (committed 2026-11-25; scrubbed copy). Point-in-time report, not maintained.

> Findings status in this report is as of 2026-11-13 at master `921c39444`. Current status of each finding is in the
> coverage table of the [stage-2 audit](2026-11-23-v3-stage2.md). Scrub: local paths, worktree names, host and account
> details and references to working notes or proof-of-concept files that are not in the repository were removed or
> replaced by "not retained"; the technical findings, severities, code locations and reasoning are unchanged.

Review status: Read-only source review of `origin/master` `921c39444`. No product code changed, no push, no hardware. Notes and raw tool
output: (not retained).

Labels: **CODE READ** = I (or a sub-reviewer, named) read every hop; **MEASURED** = a command was
run (named); **INFERRED** = a hop was not verified (named). An unlabelled sentence in a findings row
is CODE READ. Sub-reviewer findings that I re-read myself are marked "(re-read)"; the others are
marked "(sub-review, not re-read)" and carry less weight.

Severity uses the owner addendum of 2026-11-13: any guest breakout is CRITICAL; unbounded DoS is a
finding unless the repo already plans a limit (then "planned, not in place").

## 0. Falsifier (written before reading; the falsifier list (not retained)) and its result

Hypothesis H: "the safe side of `crates/kf-*` holds no host pointers and is sound."

| # | what would prove H wrong | result |
|---|---|---|
| F1 | an `unsafe` token outside the three crates' `*_unsafe.rs` | **not found** (MEASURED, §1) |
| F2 | a safe crate that can compile `unsafe` | **not found**: 20 crates inherit `forbid`; kf-chan inherits it too |
| F3 | a safe `pub fn` of an unsafe crate that safe code can misuse into UB | **FOUND**: `kf_cuda::driver_unsafe::zeroed<T: Copy>()`, `view_bytes<T: Copy>()`, the raw-device-address API (`mem_free(u64)` …), `HostSpan: Copy` (F-03, F-04, F-07) |
| F4 | a host virtual address as an integer in safe code | **FOUND**: `kf-host/src/precomp.rs:53` (new since stage 1), `FrameView.addr: usize` / `Frame::addr()` (S1-03, unfixed), `HostSpan` `Debug` (F-01, F-02, F-07) |
| F5 | a safe fn taking an integer the unsafe side turns into a pointer | **FOUND** in the kf-cuda device-address API (S1-04, unfixed); `Kf3Frame.data = f.addr as *mut u8` (`ffi_unsafe.rs:809`) |
| F6 | a `kf-*` crate depending on `kayfabe-*` | **not found** (MEASURED: `cargo metadata`, `scripts/ci/dependencies.py`: `violations=0`) |
| F7 | the ratchet undercounts | **FOUND** (43 `#[unsafe(no_mangle)]`, 1 `unsafe extern` block, macro bodies; S1-11 unfixed) |

**H is false as stated.** The lexical half holds (no `unsafe` keyword outside the perimeter). The
semantic half does not: four host addresses are nameable from safe code and three perimeter exports
leave a precondition to their safe callers. None of the four is guest-chosen today (CODE READ), so
none is a breakout by itself; each is a place where one safe-code logic bug becomes a host
memory-safety bug, which is exactly what owner ruling §R forbids. The branches that were to fix
this (`v3-sec-rawaddr`, `v3-sec-perimeter`) were never merged (MEASURED, §4).

## 1. (A) Mechanical results — MEASURED (2026-11-13)

Commands: `grep -rnw unsafe` over `crates/kf-*` (all `.rs`, tests and `build.rs` included); the
workflow's own ratchet regex (`.github/workflows/ci.yml:2008-2010`); `cargo metadata --offline`;
`python3 scripts/ci/dependencies.py`; `cargo clippy --lib --no-deps` with restriction lints (two
runs, outputs `safe/clippy_restrict*.jsonl`). `cargo geiger` is not installed and was not installed
(3 GiB free on the box).

- **`unsafe` outside `*_unsafe.rs`:** 23 lines contain the word in non-`_unsafe.rs` files of
  `crates/kf-*`; all 23 are comments. Zero code tokens. No `#[allow(unsafe_code)]`, no
  `#![allow]`, no cfg switch. The only `build.rs` (`kf-gop-image`) and the only code `include!`s
  (`kf-chan/src/user_storage.rs:716` test fixture, `kf-cuda/examples`) hold none.
  `crates/kf-abi/gen` is an offline generator, not a build dependency, and holds none.
- **Lints:** root `Cargo.toml:84-86` `unsafe_code = "forbid"`; 20 of 23 `kf-*` manifests say
  `[lints] workspace = true`; `kf-linux-raw`, `kf-qemu`, `kf-cuda` restate `unsafe_code = "allow"`
  with `undocumented_unsafe_blocks = "deny"`. Only `kf-abi` and `kf-linux-raw` also carry an
  in-source `#![forbid]`/`deny`; the rest rely on the manifest (gate A in CI checks the two lines
  with `grep`, S1-01 unfixed).
- **`kf-chan/src/tspace_unsafe.rs`** (866 lines) sits in a `forbid` crate and holds no `unsafe`
  token; it is an address perimeter by ruling §R(b) and is in `AUDITED` with bar 0. Consistent.
- **Counts, per file** (ratchet regex lines / my token census):

| crate | file | ratchet | blocks | `unsafe fn` | `unsafe impl` | not counted by the ratchet |
|---|---|---|---|---|---|---|
| kf-linux-raw (bar 101, actual 101) | affinity 6, capability 2, chardev 4, drm 4, epoll 4, host_fd 14, kvm 6, mapping 25, signal 6, sysconf 1, timerslack 2, unixsock 10, vcpu 8, window 9 | 101 | 92 | 1 | 10 | vcpu and window each have one line with two tokens (103 tokens in 101 lines) |
| kf-cuda (bar 78, actual 78) | driver_unsafe.rs | 78 | 76 | 0 | 2 | 1 `unsafe extern "C" {` block (`:43`), 4 `transmute`, two `macro_rules!` whose bodies hold 3 `unsafe` blocks expanded at every symbol load |
| kf-qemu (bar 86, actual 86) | ffi_unsafe.rs 39, raw_unsafe.rs 47 | 86 | 51 | 25 | 10 | **43 `#[unsafe(no_mangle)]`**; of the 43 exported `extern "C"` fns, 25 are *safe* fns and 24 of those take the raw handle and dereference it through `dev()` (S1-08) |
| kf-chan (bar 0, actual 0) | tspace_unsafe.rs | 0 | 0 | 0 | 0 | — |

  The ratchet equals the committed bars (`ci.yml:1652`). `scripts/ci/test_unsafe_count.py` tests the
  pipeline's arithmetic and its failure on unreadable input, not its coverage; it has no fixture
  for `#[unsafe(..)]`, `unsafe extern {` or a macro body (S1-11, S1-12 unfixed).
- **Dependencies:** no `kf-*` crate depends on a `kayfabe-*` crate. The only non-dev external
  dependency of the whole `kf-*` set is `libc 0.2.189` (kf-linux-raw). No serde in product code.
- **Release profile:** `overflow-checks = true` (`Cargo.toml:93-94`); `build_kf3.sh:24` builds
  `--release`. A panic on a named service thread aborts the process (`kf-qemu/src/failclosed.rs`);
  a panic inside an `extern "C"` entry aborts. So an arithmetic overflow is a VM stop, not a wrap.
- **Panic lints (non-test `--lib`):** `kf-mem` denies `arithmetic_side_effects`, `indexing_slicing`,
  `unwrap_used`, `expect_used`, `panic` in non-test code (`kf-mem/src/lib.rs:15`) and builds clean.
  `kf-chan`, `kf-gsp`, `kf-trap`: 0 `unwrap`/`expect` in non-test code, 1 `panic!` (`kf-trap/src/token.rs:349`).
  Warn counts (arith / index / trunc-cast): kf-chan 306/48/29, kf-gsp 96/30/7, kf-trap 58/36/20,
  kf-rm 307/275/23, kf-disp 347/121/74, kf-qemu 210/100/61, kf-host 32/36/12. Production
  `expect`/`unwrap`: kf-host 9 (7 are `objects.lock().expect`, i.e. a poisoned mutex aborts), kf-qemu 3,
  kf-rm 3, kf-disp 1. These counts are a work list, not findings: only `kf-mem` has turned them into a gate.

## 2. (3) Host addresses in safe code — the owner's question

**Answer: NOT confirmed. Safe code can name a host address in four places.** None is guest-chosen
(CODE READ); each breaks owner ruling §R ("no raw or disguised host or device address outside the
perimeter") and standing principle A.5/A.6.

| id | where | what the value is | where it goes | who bounds it | label |
|---|---|---|---|---|---|
| F-01 | `crates/kf-host/src/precomp.rs:53` (added 2026-10-22, after stage 1) | `ops.as_mut_ptr() as u64`: the host VA of a `Vec<u8>`, written by safe code into bytes 24..32 of an RM control's params | `raw_control_exact` → `NV_ESC_RM_CONTROL`; the host kernel reads and writes `count * op_size` bytes at that address | only safe code: `precomp_layout.rs` constants (`op_size 32`, `count` at 20, `pointer` at 24) and `ops.len()`; the `Indirect` funnel in `chardev_unsafe.rs`, which exists to mint and scrub such addresses, is bypassed (the pointer is a second-level one inside the payload) | CODE READ |
| F-02 (= S1-03, unfixed) | `crates/kf-qemu/src/display.rs:883-899` `pub struct FrameView { pub addr: usize, .. }`; `crates/kf-cuda/src/display.rs:308` `pub fn addr(&self) -> usize`; stored in `AtomicUsize` slots (`display.rs:1096`, `:1120`) | host VA of the page-locked console frame | `ffi_unsafe.rs:809` `data: f.addr as *mut u8` → `kf3.c:1059-1070` builds a pixman surface of `width × height`, `stride` from the same safe struct, no length | safe code only: `w*h <= MAX_PIXELS` (`display.rs:5149`), allocation `FRAME_SMALL`/`FRAME_MAX` (`:5609-5616`), frames never freed (`:5633`). The C check `stride < width` still compares bytes with pixels | CODE READ (bound verified by the device sub-review and re-read) |
| F-03 (= S1-04, unfixed) | `crates/kf-cuda/src/driver_unsafe.rs:685-856` (`mem_free(p: u64)`, `memcpy_h2d(dst: u64, ..)`, `memcpy_d2h(.., src: u64)`, `memset_d8`, `memcpy_d2d_async`), module is `pub mod driver_unsafe` | CUDA device addresses (`CUdeviceptr = u64`); under unified addressing a device address can be a VMM address (S1-05) | libcuda | callers: `kf-cuda/src/display.rs` (43 call sites, `at()` at `:1774` checks `off + len <= store`) and `walk.rs` (14). The `// SAFETY:` comments say "every caller sizes them" | CODE READ |
| F-07 | `crates/kf-linux-raw/src/mapping_unsafe.rs:1022-1026` `#[derive(Debug, Clone, Copy)] pub struct HostSpan { base: NonNull<u8>, len }` | a host span | `{:?}` prints the address to any safe caller; `Copy` with no lifetime lets a span outlive its mapping (`HostSpan::as_ptr` is `unsafe`, so the dereference is in the perimeter) | — | CODE READ |

Also still present, lower weight: the RM map cookie as `u64` in safe structs and in a log line
(`kf-host/src/lib.rs:1663`, `kf-qemu/src/mem.rs:411`, S1-15; a host physical/bus cookie that is never
dereferenced); `as_ptr() as usize` identity keys in `kf-util/src/lock.rs:880,923,1348` and
`trapwitness.rs:398` (atomic keys only, benign; they show the gate's blind spot); raw fds as `i32`
(`import_and_map(device, fd: i32, ..)`, `CompletionFd::raw()`, `kf3_broker_ready(h, fd, ..)`, S1-10).

**Why the CI gate did not catch F-01:** the host-pointer gate greps `\*mut |\*const |NonNull|transmute`
(`.github/workflows/ci.yml:693-694`). `.as_mut_ptr() as u64` matches none of them (S1-02, unfixed).

**Guest-scoped values (fine, checked):** GPU VAs, store/RAM offsets, BAR offsets, GPAs
(`kf-abi/src/submit.rs:1394 address()` is a guest GP entry address), `OverlayHook::submit(base, len)`
(guest BAR1 offsets handed to the C device).

**Minimal fixes.** F-01: build the op array inside `kf-linux-raw` (a second `Indirect` level that mints,
bounds and scrubs the inner pointer), or move `precomp.rs` into the perimeter as `precomp_unsafe.rs`
in an audited crate. F-02/F-03: merge the work on `origin/v3-sec-rawaddr` (17 commits, last
2026-10-04: opaque frame handle with a length; device memory behind handles; the address gate G1).
F-07: hand-written `Debug` that prints only the length; drop `Copy`.

## 3. Findings

No finding below is a confirmed guest breakout (**no CRITICAL**). The rows are ordered by severity. (2026-11-13)
"sub-review" names the notes file under `safe/`.

| id | sev | where | guest input → failure | minimal fix | label |
|---|---|---|---|---|---|
| F-05 (= S1-42) | **HIGH** (cross-tenant DoS from guest *userspace*; `V3_SECURITY_MODEL.md` §7 lists it "open", no limit is planned) | `crates/kf-qemu/src/chan.rs:5815-5865` `encoder_session`; decoded at `kf-rm/src/chanlink.rs:1574-1584` | A guest process sends GSS control `0x20808163` (NVENC session acquire, 4 zero bytes) in a loop. Each one is a host `raw_control(subdevice, ACQUIRE)`; the per-client counter only `saturating_add`s. One VM takes every GPU-wide NVENC slot; other VMs and host applications cannot encode | refuse past a per-VM cap before the host call (`NV_ERR_INSUFFICIENT_RESOURCES`) | CODE READ (re-read) |
| F-06 | **HIGH while the flag is off (shipped default); CRITICAL by the owner's rule when `KF3_GSS_NATIVE=1`** | `crates/kf-rm/src/gssnative.rs:65,296-345,445-472`; `crates/kf-qemu/src/chan.rs:95-105`; `crates/kf-host/src/lib.rs:1190` `pub fn raw_control_opaque` | With the env flag, any guest control `(cmd & 0xC000) == 0x8000` on class `0x2080` is carried to host RM with the guest's parameter bytes unchanged (≤ 64 KiB, 20 000 per boot, cap cleared by a device reset). No per-command allowlist. The guest then authors a host RM/GSP action (principle A.2). Which commands exist and whether any carries a pointer member is unknown (closed GSS-legacy; INFERRED) | delete the experiment from the product build, or restrict it to an explicit command+size allowlist; make `raw_control_opaque` non-`pub` | CODE READ; effect on the host INFERRED |
| F-02 | **HIGH** (latent host-memory read; rule §R) | §2 | The guest steers `width`/`height`; a safe-code logic error in `display.rs` (not found today) would make QEMU's console read VMM memory past the frame and show it on the host console/VNC | §2 | CODE READ |
| F-07a (= S1-45) | MEDIUM-HIGH (host BAR1 aperture is shared by tenants; §7 lists it "open") | `crates/kf-qemu/src/mem.rs:641` unbounded `mpsc::channel::<StoreView>()`; `:757-776` inline `openat` on the vCPU when the 4-node pool is empty; `:2855-2873` | Guest root writes the PRAMIN window register (BAR0) in a loop from several vCPUs with distinct windows. Each move is an RM map + mmap on the vCPU (sanctioned) and queues the replaced view (one fd + its host BAR1 aperture) for one reaper thread. vCPUs outrun the reaper: pending fds and BAR1 grow until `RLIMIT_NOFILE` or the host BAR1 is exhausted; other tenants' BAR1 maps then fail | bounded `sync_channel`; when full, release inline or refuse the move (counted); never `openat` on the vCPU | CODE READ; the rate race INFERRED |
| F-08 (= S1-44) | MEDIUM | `crates/kf-qemu/src/chan.rs:3957-4075` → `kf-host/src/channel.rs:1834 set_timeslice` | Guest kernel `SET_TIMESLICE` u64 reaches the host TSG unclamped: longest slice host RM accepts on the shared runlist | clamp to a per-VM ceiling | sub-review (notes not retained), not re-read |
| F-09 | MEDIUM | `crates/kf-gsp/src/boot.rs:2212-2232, 2495-2556, 2655-2660`; caller `kf-qemu/src/device.rs:4168-4181` | Guest root stops advancing the status-queue read pointer, sends one command and rings the doorbell N times. `policy.respond(cmd)` runs before `post`; `post` returns `QueueFull`; the command is not consumed ("answer-then-commit", by design); every doorbell re-runs `respond` with its side effects and prints a line | check reply space before `respond`, or make `respond` idempotent per `(sequence)` | CODE READ (re-read) for the order; the effect of a repeated `respond` INFERRED (policy bodies not read) |
| F-10 | MEDIUM (host RAM; not in §7) | `crates/kf-gsp/src/boot.rs:902,2545-2556` (`held: Vec`, no cap, `remove(0)`); `crates/kf-qemu/src/mem.rs:2210,2535` (`Inbox.q` unbounded `Vec`); `crates/kf-mem/src/vasmgr.rs:1166-1303` (`pending: Vec<Want>`); act queue `chan.rs:3129` (unbounded `mpsc`) | Guest root posts map/page-directory RPCs or re-arms invalidates faster than the VA/act thread settles them: four queues grow without a cap until the VMM is OOM-killed (host memory pressure) | stop servicing command doorbells past a cap (the pattern at `mem.rs:2417`, 512); coalesce `Want`s | sub-reviews ((notes not retained), (notes not retained), (notes not retained)); `held.push` re-read; sustainable rate INFERRED |
| F-11 | MEDIUM (in-guest DoS from guest userspace, R1.5; breaks "the drainer never blocks") | `crates/kf-rm/src/rmgraph.rs:1971-1989` (`resolve_pending_dups`: linear `find_map` over up to `MAX_PARKED = 2^18` on every Alloc/Dup and after every promotion); `:1466-1478`; `:1916,1962` | A guest process parks 2^18 dangling `DUP_OBJECT`s (no per-client quota). Every later RM alloc/dup of every process then costs a 2^18-entry scan on the drainer under the GSP mutex | index parked dups by source; per-client quota | CODE READ (re-read) for the loop; thread/lock per the device sub-review |
| F-12 | MEDIUM | `crates/kf-rm/src/chanlink.rs:1196-1208,1266,1272,1532-1536,2263` | The channel link runs before the object seat can refuse: its VA-space/TSG/ctxshare/alias maps grow on guest `(client, handle)` values with no cap | cap each map; record after the seat accepts | sub-review (notes not retained), not re-read |
| F-13 (= S1-86) | MEDIUM (owner rule "production is quiet on input-serving threads"; §7 "open, no limit") | drainer, under the `gsp` mutex: `kf-qemu/src/device.rs:3122,3133,4166,4178,3292`; `kf-gsp/src/boot.rs:2236,2435,2545`; act thread: `chan.rs:3157,3165,6678,7001`; doorbell path `chan.rs:6106,6192`; `kf-rm/src/chanlink.rs:981,1010`; `kf-mem/src/apply.rs:1043,1399`; `kf-qemu/src/display.rs:3087`; `kf-chan/src/user_lifecycle.rs:1179-1198` | One `eprintln!` per guest event (per RPC, per refused birth, per doorbell on a failing relay). A slow stderr sink blocks the drainer (and, through Rust's stderr lock, every thread that logs); a file sink grows without bound. There is no shared rate-limit helper; five sites hand-roll one | one helper (first N, then powers of two) for every guest-reachable site; per-RPC lines become counters | CODE READ by four sub-reviews; `device.rs:4166-4181` and `boot.rs:2545` re-read; the blocking sink INFERRED |
| F-14 | MEDIUM (in-guest stale translation, R1.1) | `crates/kf-mem/src/apply.rs:1071-1105`; `vasmgr.rs:1660-1669,1756-1782`; `batch.rs:2213-2233` | A map run fails part-way; the take-down of its placed pieces is itself refused by the host; the refusal sets only `first_refusal`, the invalidate still completes; the host mapping stays over a VA the walker never committed, so a later guest unmap emits no UNMAP | count a refused take-down as `unmap_refused` (hold the invalidate) and retry strays at `begin_refresh` | sub-review (notes not retained) CODE READ, not re-read; a host unmap refusal is INFERRED |
| F-15 | MEDIUM, UNVERIFIED | `crates/kf-mem/src/ledger.rs:504,527` (`VA_ALREADY_MAPPED → Mapped::HeldByHost`); `apply.rs:1041-1058,1084` | Guest userspace maps at a VA where host RM placed its own buffer in the twin's VA space. The row is acknowledged HELD (owner ruling (a), P6b), so guest GPU work at that VA addresses the host RM buffer, not guest memory. Safe only if every mapping host RM places in an unprivileged client's VA space is privileged or harmless — property P, the same as a local process; not shown for v3 | box test: enumerate what host RM maps in a twin VA space per family and read/write it from a guest channel; or refuse instead of HELD on user twins | CODE READ; host RM's placements INFERRED |
| F-16 | MEDIUM | `crates/kf-qemu/src/chan.rs:6947-7001` | Passthrough birth allocates the host channel + TSG before `plane.allocate_channel` checks the runlist budget and token collision; a guest loop costs a host alloc+free on the single act thread per attempt, plus a log line | charge the caps first (as `userquota.rs:31-44`) | sub-review, not re-read |
| F-04 (= S1-09) | MEDIUM (soundness, not guest-reachable) | `crates/kf-cuda/src/driver_unsafe.rs:1627` `pub fn view_bytes<T: Copy>(&T) -> &[u8]`, `:1658` `pub fn zeroed<T: Copy>() -> T` | Safe code in any crate can write `zeroed::<&u8>()` or `zeroed::<fn()>()` (instant UB) or read padding through `view_bytes`. The `// SAFETY:` text says "the caller's obligation" | make both `pub(crate) unsafe fn`, or bound `T` by a sealed trait implemented for the `abi.rs` types | CODE READ |
| F-17 | LOW-MEDIUM | `crates/kf-chan/src/host.rs:1685-1692`; `ring.rs:21,372-475`; `kf-qemu/src/chan.rs:7105` (`entries.max(1)`, no upper cap) | A guest *kernel* channel sets `GP_PUT` to the ring size and points every entry at one 256 KiB segment of no-ops: one pump call allocates, copies and decodes `entries × 256 KiB` on a worker without returning | cap bytes per pump call and re-ring; cap `entries` at birth | sub-review (notes not retained); the pump loop and the missing cap re-read |
| F-18 | LOW-MEDIUM | `crates/kf-chan/src/user_loop.rs:157-161,237-249`; `worker.rs:237` | Guest userspace leaves a packet open at a `GP_PUT` boundary and stalls its PBDMA: the worker's `idle()` returns `Spin`; one host thread at 100 % for the stall's length | bound the spin, fall back to the timed backstop | sub-review, INFERRED for the stall length |
| F-19 (= S1-81) | LOW today (MEDIUM once nothing dominates it) | `crates/kf-trap/src/shadow.rs:203-208` `is_overdue` | The late-invalidate tripwire is still called only from `kf-trap/src/tests.rs:409-418` | call it from the VA thread's status pass | MEASURED (grep) |
| F-20 | LOW | `crates/kf-chan/src/translated.rs:799-1128`, `TranslatedRing::new` (`ring.rs:182`) | The legacy deny-list rewriter (`strict` defaults false; S1-48, S1-85) is still `pub`; production uses only `new_tmode` (`chan.rs:7272`) | make it test-only | sub-review, CODE READ |
| F-21 | LOW | env reads: `kf-host/src/channel.rs:497` `KF3_NEGCTL_SKIP_CAP_BRACKET`; `kf-qemu/src/mem.rs:2932` `KF3_NEGCTL_TWIN_WINDOW`, `:1297`; `tspace.rs:86,107`; `chan.rs:1855,1863,1872,1929`; `device.rs:1080,4283` | Ten negative-control and injection switches that each remove a security boundary (the capability bracket, "no window in a twin", the carve-out refusal, the heap gate) are compiled into the product and read from the environment. Operator-controlled, so not a guest path; but a production VMM should not carry them (owner flag rules, 2026-10-09) | a cargo feature or a `KF3_DIAG_*` master switch that prints `PERTURBING DIAGNOSTIC ON` and refuses under a release build | MEASURED (grep), CODE READ of the reads |
| F-22 | LOW | `crates/kf-qemu/src/failclosed.rs:30-48`; `ffi_unsafe.rs:254` | `kf3-scanline` runs guest-programmed head timing but is not a service thread: a panic there is silent | add it to `is_service_thread` | sub-review; the panic itself INFERRED |
| F-23 | LOW (group, each a bounded-work or cap gap) | `kf-chan/src/user_turn.rs:620-631,1096-1103` (unbounded stream restarts, PR #360); `tmode.rs:1110-1232` (operand extent uncapped before `resolve`); `kf-mem/src/apply.rs:670-683` (O(F×L)); `batch.rs:875-877` (2^21 host calls per refresh budget); `kf-qemu/src/mem.rs:2394-2526` (`splits` uncapped); `chan.rs:7742-7766` (slot mutex held across ≤ 200 × 1 ms sleeps on the act thread); `kf-gsp/src/ring.rs:158-250` (`msg_size` up to the queue size); `kf-gsp/src/boot.rs:1682` (unchecked `gpa + i*stride`) | guest-steered work or a possible overflow panic on a service thread (VM stop, fail closed) | caps and `checked_add` as named in the notes | sub-reviews, not re-read |
| F-24 (= S1-69) | INFO | `crates/kf-cuda/src/walk.rs:425`; `kf-mem/src/apply.rs:354` | An unknown walk-report op is still applied as a MAP; every row check still applies, so no escalation | reject unknown ops | sub-review, CODE READ |
| F-25 | INFO | `crates/kf-qemu/src/ffi_unsafe.rs` | 24 safe `pub extern "C" fn` take a raw handle (S1-08); only C calls them | `pub unsafe extern "C"` | MEASURED |

**False positives / checked and safe** (so the coverage is visible):

- BAR0 trap path (`kf-qemu/src/device.rs:1528-1740`, re-read): atomics, eventfd writes, one fenced
  doorbell store, the PRAMIN exception. No lock, no allocation, no log on the vCPU. `kf-trap` takes
  no lock and allocates nothing per write; the userspace-mappable arm returns before the ring
  (`trap.rs:97-99`); a full ring poisons the guest's own device and never blocks.
- `kf-gsp` queue: geometry validated once at bind, pointers range-checked per pass, no double read found.
- `kf-rm` reassembly: `checked_add`, 64 KiB cap before any buffer, 64 continuations; decode errors
  become `NV_ERR_NOT_SUPPORTED`, never OK; handles capped at 2^18; parent walks at 64 hops.
- T-mode rewriter: every emitted address goes through `tspace_unsafe.rs` `WindowAddr`; the host-method
  set is an allow-list (`ttables.rs:237-264`). `SET_REFERENCE`, `NON_STALL_INTERRUPT` and `FB_FLUSH`
  pass the guest's data word; completion uses the fence, so no forged completion.
- USERD / error-notifier offsets (S1-43): `heap_gate` before any host call, `INCA_STRICT` hardwired
  (`chan.rs:6728-6755`, `tspace.rs`); the carve-out is refused.
- `kf-disp`: geometry capped by `MAX_PIXELS`, zero divisors guarded, cursor in i64, pushbuffer
  decode budgeted. `kf-broker`: `recv_bounded` passes no control buffer (no fd accepted), fixed
  24-byte records.
- No `kf-*` crate parses guest data with serde; no guest-derived string reaches a path or a command.

## 4. (D) Stage-1 delta, against master `921c39444`

MEASURED: `git merge-base --is-ancestor`: `v3-sec-nonpriv` and `v3-sec-audit-s1` are merged;
**`v3-sec-rawaddr` (17 commits ahead, last 2026-10-04), `v3-sec-perimeter` (21, 2026-10-10) and
`v3-sec-p0` (3, 2026-10-10) are not.**

| finding | on master | evidence |
|---|---|---|
| S1-01 lexical unsafe gates bypassable | **not fixed** | gate A still two `grep`s (`ci.yml` step at `:1652-1680`); gate C still `grep -nw unsafe` |
| S1-02 host-pointer gate sees type names only | **not fixed**; it missed F-01 | `ci.yml:693-694` |
| S1-03 console frame address as `usize` | **not fixed** | F-02 |
| S1-04 kf-cuda raw device addresses | **not fixed** | F-03 |
| S1-05 CUDA contexts can address the VMM | not re-checked (GPU kernels: review 3/4) | — |
| S1-06 v3 ledger walks the v2 crates | **not fixed** | `kf-linux-raw/tests/unsafe_naming.rs:41` `CRATES = ["kayfabe-linux-raw", "kayfabe-qemu-raw"]` |
| S1-07 `RawRegion` is `Copy` | **not fixed** | `kf-qemu/src/raw_unsafe.rs:5` |
| S1-08 safe `extern "C"` entries | **not fixed, grew 11 → 24** | F-25 |
| S1-09 `view_bytes` / `zeroed` | **not fixed** | F-04 |
| S1-10 fds as `i32` | **not fixed** | `driver_unsafe.rs:1745` |
| S1-11 ratchet undercounts | **not fixed** | §1 (43 attributes vs 24 then) |
| S1-12 gates have no known-positive fixtures | partly: `test_unsafe_count.py` tests the count pipeline only | §1 |
| S1-13 `kf3.c` outside the gates | not re-checked (C: other review) | — |
| S1-14 external dependency set unpinned by a gate | **not fixed** (no `deny.toml`; `--locked` only) | `ls`, `ci.yml:2302` |
| S1-15 RM map cookie in safe code and logs | **not fixed** | `kf-host/src/lib.rs:1663` |
| S1-16 CUDA handles / pinned buffers as `usize` | **not fixed** | `driver_unsafe.rs:1420` |
| S1-20 USER host channels refuse physical-mode work | not re-checked (hardware) | — |
| S1-21 identity windows in every twin | **fixed in code** | no `KF3_TSPACE` read remains; the only `map_window` callers are `tspace.rs:313,330` and the negative control `mem.rs:2958,2974` behind `KF3_NEGCTL_TWIN_WINDOW`; hardware gate not checked here |
| S1-22 other RM calls run as admin under a root VMM | partly: births, libcuda and the precomp query are bracketed; the rest not re-checked | `rmfacts.rs:121` |
| S1-23 Translated rings reachable | **fixed in code** (T-mode + perimeter) | `chan.rs:7272`; `tspace_unsafe.rs`; sub-review |
| S1-24 decoders reachable from guest userspace, no fuzz target | decoders bounded on read (§3); **no fuzz/property target in kf-rm, kf-gsp, kf-chan** | `Cargo.toml`s; `kf-mem/src/sim/fuzz.rs` is the only one |
| S1-25, S1-42, S1-44, S1-45, S1-82, S1-86 shared resources | **not fixed** (S1-82 not re-checked) | F-05, F-08, F-07a, F-13 |
| S1-26, S1-28, S1-29, S1-34 documents | not re-checked | — |
| S1-27 copy-then-check only gated for the PTX | not fixed as a gate; no double read found in kf-gsp / kf-chan snapshots (sub-reviews) | — |
| S1-40 RM companion size fields are the caller's | **not fixed** | `kf-host/src/lib.rs:1213` `params_size: payload.len() as u32` in safe code |
| S1-41 no host-side allowlist of controls/classes | **not fixed**; `raw_control_opaque` is `pub` | `kf-host/src/lib.rs:1190`; only channel classes are refused (`birth.rs:119`) |
| S1-43 USERD / notifier offsets can name the carve-out | **fixed** | `chan.rs:6728-6755`, `INCA_STRICT` |
| S1-47 std `Mutex` across an RM ioctl | not re-checked in kf-host; one more found on the act thread (`chan.rs:7742-7766`) | F-23 |
| S1-48, S1-85 deny-list rewriter | **fixed on the production path**, legacy code still `pub` | F-20 |
| S1-49 handle minting wraps | **not fixed** | `kf-host/src/lib.rs:1112` `wrapping_add` |
| S1-60 … S1-72 walker (PTX, CUDA) | not re-checked (other review), except S1-69 **not fixed** | F-24 |
| S1-80 hot-unplug lifetime | not re-checked (C) | — |
| S1-81 tripwire never called | **not fixed** | F-19 |
| S1-83 hostile-input instruments retired | partly restored: `kf-mem/src/sim/{fuzz,adversarial}.rs`; nothing for the RPC decoders | — |
| S1-84 release builds wrap on overflow | **fixed** | `Cargo.toml:93-94`; `failclosed.rs` |
| S1-87 … S1-91 | not re-checked | — |

## 5. Owner points and earlier-review items — checklist for this scope

| recorded point (source) | where it applies here | verdict | evidence |
|---|---|---|---|
| A.2 host verbs are authored, never forwarded (`OWNER_RULINGS.md` §A) | kf-host verbs, kf-qemu acts | HOLDS by default; **VIOLATED under `KF3_GSS_NATIVE=1`** | F-06; sub-review: every other host call is built from decoded scalars |
| A.3 completions are never forged; a refusal read as "done" is a forgery | kf-gsp replies, act replies, cache-op | HOLDS for RPC replies (`boot.rs:2484-2520`: default is a named refusal); one documented exception: a refused host cache flush still publishes idle/NV_OK (`device.rs:1961-1975`, `mem.rs:3560-3573`) | sub-reviews |
| A.4 no blocking on a vCPU or under a lock another thread blocks on; PRAMIN move is the one exception | `device.rs` trap path | HOLDS on the BAR0 path; the PRAMIN exception also does an inline `openat` when its pool is empty (`mem.rs:763-769`) and holds a std `Mutex` across the ioctl (`cpuwin.rs:914-1037`) | re-read; F-07a |
| A.5 VMM addresses are never guest-chosen or guest-visible | all | HOLDS (no guest value becomes a host address; no host address is written to guest memory in the code read) | §2 |
| A.6 / §R no raw or disguised host address outside the perimeter; every perimeter export validates its own inputs | kf-host, kf-qemu, kf-cuda, kf-linux-raw | **VIOLATED** | F-01, F-02, F-03, F-04, F-07 |
| §R(c) ratchet the perimeter's size and exports; §R gates 1-5 | CI | **VIOLATED** (the branches holding them are unmerged) | §4 |
| A.9 hostile-guest isolation; guest root harms only its own VM | resources shared with other tenants | **VIOLATED for availability** | F-05, F-07a, F-08 |
| §Q a passthrough twin maps only rows from the guest's page tables | `kf-qemu/src/mem.rs` twin life cycle | HOLDS in code; the HELD-BY-HOST acknowledgement is UNVERIFIED | S1-21 row; F-15 |
| §AB T-space hardwired; whole guest RAM only for privileged channels | `tspace.rs`, `chan.rs:7105-7115` | HOLDS (`Privileged::of` witness before a T-space channel) | CODE READ |
| §S stub privileged management; never forge GPU work | kf-rm `hoststub.rs:80-96` | HOLDS (designed NV_OK with zeroed params, no GPU work) | sub-review |
| "The drainer may never block; production is quiet on input-serving threads" (earlier ruling, 2026-10-09) | drainer under the `gsp` mutex | **VIOLATED** (per-RPC `eprintln!`; O(n) rescans) | F-13, F-11 |
| "Unchanged VAs stay accessible during a refresh; no unmap-then-remap" (2026-10-10) | kf-mem apply/batch | NOT re-verified here (the sub-review found no unmap-then-remap; it did not target it) | UNVERIFIED |
| "BAR1/BAR2 mappings only inside a refresh; PRAMIN is the exception" (2026-10-31) | `cpuwin.rs`, `mem.rs` retire | HOLDS as far as read: only `PraminPool` uses the off-vCPU `retire` | sub-review; `mem.rs:779-788` re-read |
| "No VMM VA pointers/integers and no unvalidated offset+length cross from safe code into `*_unsafe.rs`" (same note) | kf-cuda, kf-qemu FFI | **VIOLATED** | F-02, F-03 |
| "Passthrough spaces hold only guest-PT-leaf mappings; USERD sysmem fix absent" (2026-10-10) | twins | HOLDS in code (see §Q row) | — |
| "MMIO memcpy only for vidmem rings" (2026-10-16) | userpump snapshots | NOT-APPLICABLE to security; not reviewed | — |
| "Single-channel guest-userspace faults must not kill the guest and must be reported" (2026-10-22) | kf-chan relay errors | PARTLY: an out-of-ring `GP_PUT` is counted and silently ignored, no error to the guest (`userd_relay.rs:97-112`) | sub-review |
| "Flags: needed behaviour is hardwired; measurement flags never required; no correctness flags" (2026-10-09) | 109 `KF3_*`/`KF_*` names read in `crates/kf-*/src` (MEASURED) | **VIOLATED in spirit** for the ten boundary-removing switches | F-21 |
| "Deliver events at NVIDIA's granularity; never drop an edge" | not security; not reviewed | NOT-APPLICABLE | — |
| "Forward stripped capabilities only after correctness/isolation evidence" (2026-10-24) | `kf-mem/src/apply.rs` PTE-kind allowlist | HOLDS (kinds still go through the allowlist; not re-read line by line) | UNVERIFIED |
| Owner 2026-10-08: a guest integer overflow must panic and stop the VM | release profile, `failclosed.rs` | HOLDS, except `kf3-scanline` | §1; F-22 |
| `THE_CONSTRAINTS.md` §39(a) every guest-memory read is copied once and decided on the copy | kf-gsp, kf-chan | HOLDS in the code read; not a gate (S1-27) | sub-reviews |
| `THE_CONSTRAINTS.md` §41/§47 what a vCPU MMIO write may do; the doorbell is adversarial | `device.rs:1595-1740` | HOLDS | re-read |
| `THE_CONSTRAINTS.md` §58 no window in a space an unprivileged channel uses | twins | HOLDS in code | S1-21 row |
| Audit plan §2.4 "one table of every guest-steered resource with its cap and the cap's test" | — | **NOT BUILT** (stage 2 not started); new uncapped items: F-10, F-12, act queue | §3 |
| Audit plan §2.8 Clippy cast/arithmetic lints ratcheted on guest-facing crates | — | only kf-mem (§1) | MEASURED |
| `w301` audit (pre-v3 tree, STATUS ANSWERED 2026-08-14): §1.4 PREEMPT must not lie; §3.1 the reap must be reached; §3.2 pins released | v3 `chan.rs` | PREEMPT: `KF3_ASYNC_PREEMPT_REENABLE_NOACT` returns NV_OK with no host act, default off (`chan.rs:4413-4425`); the rest NOT re-checked against v3 | sub-review |
| Stage-1 audit, every S1-xx | §4 | see §4 | — |

## 6. Not reviewed, and why

- **All `*_unsafe.rs` bodies** (14 347 lines), `qemu/hw/misc/kf3/kf3.c`, the CUDA/PTX kernels, the
  broker's C side, `firmware/`: other reviews of this series. I read perimeter *signatures* only.
- **`kf-abi/src/generated/`** (≈ 45 k lines, generated), `kf-census`, `kf-crec`, `kf-oprom`,
  `kf-gop-image`, `kf-harness`, `kf-rawclient`, `kf-trace`, `kf-chip`, `kf-util`: not guest-reachable
  in the VMM, or tools; only the mechanical checks of §1 cover them.
- **Read by grep or by range only** (each sub-review's notes file lists its ranges): `chan.rs`
  1-2200, 5100-5420, 5880-6060, 7300-7700, 8200-9000; `userpump.rs`, `userrelay.rs`, `userva.rs`,
  `userstore.rs` bodies; `mem.rs` 845-915, 945-1335, 1650-1890, 1990-2200, 2560-3180; parts of
  `display.rs`; `kf-rm` `staticinfo.rs`, `authored.rs`, `memory_list.rs`, `sweep.rs`,
  `faultbuffer.rs`, parts of `rmrpc/mod.rs` and `chanlink.rs`; `kf-chan` `video_generated.rs`,
  `translated.rs` 1-720, `host.rs` 311-800; `kf-mem` `batch.rs` 1000-1286; `kf-core`, `kf-arch`
  by grep; `kf-host` beyond `lib.rs` control/mint paths, `precomp.rs`, `birth.rs` signatures.
  The user-ring relay in kf-qemu (`userpump.rs`, 4 500 lines) is the largest guest-userspace-facing
  body that nobody read line by line: it deserves its own pass.
- **Lock-order analysis** was done per file by the sub-reviews, not globally; no tool (loom, the
  repo's lock witness) was run.
- **Nothing was run on hardware**; every DoS scenario's rate ("the guest outruns the reaper") is INFERRED.
- **Method limits:** seven sub-reviews;
  I re-read the hops marked "(re-read)". Rows marked "not re-read" are the sub-reviewer's CODE READ.
  Disk on the box was 3 GiB free, so only `clippy --lib --no-deps` was built; no `cargo test`, no
  `cargo geiger`, no Miri.

## 7. Top 10

1. **F-05 HIGH** NVENC slots: a guest process can take every GPU-wide session (`chan.rs:5815`); no per-VM cap, none planned.
2. **F-06 HIGH/CRITICAL-if-enabled** `KF3_GSS_NATIVE=1` forwards guest control bytes to host RM (`gssnative.rs`, `kf-host lib.rs:1190`).
3. **F-02 HIGH** the console frame's host address is a `pub usize` in safe code and the C side gets no length (S1-03 unfixed; `v3-sec-rawaddr` never merged).
4. **F-01 MEDIUM** new: `kf-host/src/precomp.rs:53` writes a host pointer into an RM control from safe code, bypassing the `Indirect` funnel; the CI gate cannot see it.
5. **F-07a MEDIUM-HIGH** PRAMIN retire queue unbounded: guest root can pin fds and host BAR1 aperture (S1-45).
6. **F-03/F-04 MEDIUM** kf-cuda exports raw device addresses and two unsound generic fns as safe `pub` API (S1-04, S1-09).
7. **F-10 MEDIUM** four uncapped queues behind held RPC replies (`boot.rs:902`, `mem.rs:2210`, `vasmgr.rs:1166`, `chan.rs:3129`): VMM memory growth.
8. **F-13 MEDIUM** per-event `eprintln!` on the drainer under the GSP mutex and on other guest paths (S1-86); breaks the owner's "drainer never blocks" rule.
9. **F-11 / F-09 MEDIUM** the drainer does 2^18-entry rescans per RM call after a dup flood (`rmgraph.rs:1971`); a full status queue re-runs `respond` per doorbell (`boot.rs:2212`).
10. **F-14 / F-15 MEDIUM** kf-mem: a refused take-down can leave a stale host mapping; a guest VA that collides with a host-RM mapping is acknowledged HELD (isolation rests on property P, unverified).
