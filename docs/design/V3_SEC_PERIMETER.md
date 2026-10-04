# V3 security perimeter: gates, size ratchet and export table (design, rev 2)

**STATUS: LIVE, 2026-10-04 (C8), rev 2.** C0-C8 have landed on `v3-sec-perimeter`: gates G1 and G3-G7 in `.github/workflows/perimeter.yml` with their known positives, the nightly `kf3-link.yml` and `perimeter-mutants.yml`, rule (a) on the free files (a1, a3-a11), and these docs. P2 and C11 wait on the lanes named in §10. Each section records, in a ⊘ **As built** block above or below what it corrects, where the implementation measured something else. The hardware merge bar at C7a-C7c is the coordinator's to schedule. The paragraph below is the design pass this replaced, kept as written.

**STATUS was: DESIGN, 2026-10-04, rev 2.** This is the read-only design pass for branch `v3-sec-perimeter`, revised after an adversarial review. Rev 1 is superseded in place: this text replaces it, and §R of this doc lists what changed and why. It turns `docs/OWNER_RULINGS.md` §R (origin/master `789dee9f`, from line 493) into commits that can be implemented one at a time.

- **Closes:** S1-01, S1-06, S1-11, S1-12, S1-13 (tier 1) and S1-14.
- **Narrows:** S1-40, the size-field *value*. The size-field *location* stays open until P2.
- **Recorded per lane:** S1-02, S1-03, S1-04, S1-05, S1-07, S1-08, S1-09, S1-10 and S1-16.
- **Home:** `docs/design/V3_SEC_PERIMETER.md` (this file). STATUS becomes LIVE when C1 lands.

**Base.** origin/master `789dee9f`, which is code-identical to `e4fb0190` (`789dee9f` changed docs only). All line numbers are at `789dee9f`.

**Lane branches at the time of writing (2026-10-04):**
- `v3-sec-nonpriv`: 9 ahead.
- `v3-broker`: 46 ahead.
- `v3-dispsw-exp`: 22 ahead.
- `v3-sec-rawaddr`: equals `e4fb0190`.
- `v3-viommu`: pushed, equals `789dee9f`.
- `v3-p1p2`: not pushed.
- `v3-sec-p0`: 2 ahead (THE_CONSTRAINTS only).

**Labels used below:**
- **NOW** lands on this branch, in files no lane edits.
- **P2** lands on this branch after the named lane merges.
- **LANE:<branch>** is recorded for that lane and not edited here.

---

## R. What rev 2 changed (2026-10-04 review findings, each measured or cited)

The measurements behind this table were taken on 2026-10-04 with rustc 1.99.0 in scratch directories (see the facts list below).

| # | rev 1 | problem | rev 2 |
|---|---|---|---|
| R1 | G1 used `RUSTC_WORKSPACE_WRAPPER` | It runs for workspace members only. Root `Cargo.toml:6` excludes `archive` and `firmware`, so a path dependency under either is not a member. Measured: such a dependency compiled an `unsafe` block, the wrapper never ran for it, `cargo metadata --no-deps` did not list it, and the lexer excludes `archive/`. | `RUSTC_WRAPPER`, which sees every unit, classified by `CARGO_MANIFEST_DIR`; M8 |
| R2 | G1 forbid + G2 deny layout as the compiler half | Under `-D unsafe_code`, an outer `#[allow(unsafe_code)]` on a fn in a safe file compiles its `unsafe` block (measured), so G2's security rested on lexer rule L3. `-F unsafe_code --cap-lints warn` compiles an `unsafe` block with rc 0 and no output (measured). | **G1 = compiler location gate:** `--force-warn unsafe_code` on every repo unit, with the location of every diagnostic checked (§1.2). G2 becomes P2 hygiene. |
| R3 | "nested cargo builds inherit the environment" | False. `kf-gop-image/build.rs:117-121` (filter at `build/support.rs:41-51`) and `kayfabe-isolate-host/build.rs:275-294` strip every `RUSTC*`/`CARGO*` variable. Measured: 0 nested units reached the wrapper. | Separate wrapper passes for firmware and the standalone packages (§1.2) |
| R4 | Wrapper classified by the root file's realpath | Cargo accepts `[lib] path` pointing into another crate's `src/` (measured), so a package can borrow a class U path. | Classify by `CARGO_MANIFEST_DIR`; every target root must be under its own package (M9) |
| R5 | Lexer excluded `*/target/` | The pattern also matches `src/target/**` inside a class U crate. `ci.yml:695` has the same blind spot. | Enumerate with `git ls-files`; L0 checks file-set parity with the compiler's dep-info |
| R6 | "a lexer bug cannot let unsafe into a safe unit" | Four shapes are lexer-only (§1.2): `include!` of a perimeter file (measured: attributed to the included file, no expansion chain); exported-macro bodies (measured: invisible under `-F` and under force-warn); code no pass compiles; doctests. | Stated, with fixtures |
| R7 | Generator: `--document-private-items` only, exported Self types only, no trait or alias kinds | Missed, all measured: `#[doc(hidden)]` items and modules; trait definitions and their provided methods (visibility `default`); blanket impls written in the file (`for: generic`); impls for foreign Self types; `cfg(not(debug_assertions))` items (absent in both profiles). Type aliases have no kind. | `--document-hidden-items`; module-based membership; trait, alias, static and union kinds; cfg rule; a reach check (§3.2) |
| R8 | E3 needs refs that resolve | Vacuous rows pass. Tests guarded by `require_kvm!` (`kvm_gate.rs:111-119`) return early in CI. A `ui:` file can fail for an unrelated reason. Trait-impl rows are unbound. | E3b-E3e, plus a nightly mutation gate (E3c) |
| R9 | §3.1 example: `CharDevice::ioctl` OK | Request-agnostic: argument bytes can carry pointer fields that no `Indirect` patched (exports lens, finding ii). | OPEN. Rule (a) applies once a typed RM layer exists; that layer needs class U (§5.2). |
| R10 | a2 "closes" S1-40, lands NOW | The `SizeSpec` offset and `Fixed(n)` are caller-declared, and the kf-host test pins call sites. a2 also breaks `v3-broker`'s `drm.rs:304` at merge. | S1-40 narrowed; a2 lands after `v3-broker` |
| R11 | C4 line-1 allows in all 13 files | Edits `driver_unsafe.rs` and `ffi_unsafe.rs`, which belong to `v3-sec-rawaddr` (do not edit). Breaks the broker and nonpriv merges. Not needed for security after R2. | P2 |
| R12 | `code` ceiling with slack | Contradicts the exact-equality rule at `ci.yml:866-870` ("rather than as silent slack"). | Exact |
| R13 | Stored `OPEN = <n>` header | A second source of truth; every lane edits the same line. | Computed; a new OPEN row carries a dated reason |
| R14 | VALIDATES set derived from `crate::m` | Misses names re-exported at the crate root: `crate::HostOffset` is defined at `bounds.rs:31`, `crate::HostPageSize` at `page_size.rs:75`. | rustdoc item→file map |
| R15 | RawRegion: `UserdView` re-resolves | The liveness check sits at the caller. | Lease inside `raw_unsafe.rs` accessors |
| R16 | M5: direct proc-macro dependencies | Misses proc-macros reached through a re-export. | M7: exact external set and edges; M6 pins features |
| R17 | kf3-c on `ubuntu-latest`; edits `build_kf3.sh` | The runner's compiler is unmeasured (measured on gcc 15.2.0, clang 21.1.8, Ubuntu 26.04). `build_kf3.sh` is a bench tool. Names-only link closure. | Container pinned by digest; parse, do not edit; K4 (prototypes) and K5 (depfile) |

---

## Measured facts this design relies on (2026-10-04, rustc/rustdoc 1.99.0, scratch dirs under `/tmp/claude-0`, deleted)

**Compiler flags (measured 2026-10-04):**
- Under `-D unsafe_code`, a file-level `#![allow(unsafe_code)]` works. Under `-F` it is E0453.
- A local macro with `unsafe` in its body errors when called from a safe file under `-D`.
- Under `-D unsafe_code`, an outer `#[allow(unsafe_code)]` on a fn in a safe file lets its `unsafe` block compile.
- `-F unsafe_code --cap-lints warn` compiles an `unsafe` block with rc 0 and prints nothing.
- `--force-warn unsafe_code` still reports under each of the following, in any order: `#![allow]`, `#[allow]`, `-A unsafe_code`, `-A warnings`, `--cap-lints allow`.

**What `unsafe_code` reports.** It names each of these separately: `unsafe` blocks, `unsafe fn` declarations, `unsafe` methods in impls, `unsafe trait`, `unsafe impl`, `unsafe extern` blocks, `#[unsafe(no_mangle)]`, `#[unsafe(export_name)]`, `#[unsafe(link_section)]` and `global_asm!`.

**JSON diagnostics.** Each diagnostic carries the macro expansion chain, for example def-site `a_unsafe.rs`, call-site `safe.rs`.

**What the compiler cannot see (measured 2026-10-04):**
- `include!("x_unsafe.rs")` inside a safe module is reported at `x_unsafe.rs` with no expansion chain.
- `unsafe` arriving through another crate's exported macro is silent under `-F` and under `--force-warn`.

**Wrappers and cargo (measured 2026-10-04, cargo 1.99.0):**
- `RUSTC_WRAPPER` sees non-member path packages, with `CARGO_MANIFEST_DIR` set. `RUSTC_WORKSPACE_WRAPPER` does not see them.
- Cargo accepts a `[lib] path` outside the package directory.
- `cargo check` writes dep-info (`.d`) listing every module file and every `include!`d file.

**The force-warn prototype over origin/master** (`cargo check --workspace --all-targets --locked`, fresh target, 4 cores, 45.4 s cold, 2026-10-04):
- 304 in-repo units in 37 packages, 1031 `unsafe_code` diagnostics.
- Exactly 5 outside `*_unsafe.rs`, all in `crates/kayfabe-doorbell`: `hostverb.rs:121` (reported twice, lib and test) and `tests.rs:2119`, `:2123`, `:2138`.
- Deduplicated counts:
  - kf-linux-raw: 65 blocks, 8 `unsafe impl`, 1 `unsafe` method.
  - kf-cuda: 73 blocks, 2 `unsafe impl`, 1 extern block.
  - kf-qemu: 23 blocks, 13 `unsafe fn`, 4 `unsafe` methods, 6 `unsafe impl`, 25 `no_mangle`.
- These equal the old ratchet's 75 / 75 / 46 (`ci.yml:1522`), except one kf-linux-raw block. That gap is consistent with the `cfg(target_arch = "aarch64")` arm at `mapping_unsafe.rs:895-902`, which an x86_64 compile does not see.
- 0 nested-build units reached the wrapper.

**rustdoc JSON (`format_version` 61 at 1.99.0, measured 2026-10-04):**
- A private item is `{"restricted": …}` to its own module.
- `#[doc(hidden)]` items and modules are absent without `--document-hidden-items`.
- Trait-definition methods have visibility `default`.
- A blanket impl written in the file has `blanket_impl: null`, `for: {"generic": …}`.
- `cfg(not(debug_assertions))` items are absent in the dev and release profiles.
- A macro-generated item's span is its invocation site.
- Impls inside `const _: () = {…}` and inside fn bodies are present.
- Enum variant fields have visibility `default`.
- kf-linux-raw perimeter: 110 safe crate-visible functions and 1 unsafe one.

**Mutation and dependencies (2026-10-04):**
- `cargo mutants --list` over kf-linux-raw's `*_unsafe.rs` files: 475 mutants (cargo-mutants 27.1.0).
- The kf-qemu normal+build closure: 18 `kf-*` members plus `libc`. Enabled features: `libc` {default, std}, `kf-oprom` {alloc, default}. Custom builds: {kf-gop-image, libc}.
- Proc-macros in the lock: {serde_derive}.
- `Cargo.lock` holds 28 external packages.
- No symlinks are tracked, and no doctest fence contains `unsafe`.

The lens measurements of rev 1 (gates, exports, hwaddr) still stand where cited.

---

## 0. The model, and the one source of truth

What §R says, as rules a machine can check:

- `*_unsafe.rs` is the audit perimeter.
- Inside it, a function whose precondition it cannot check is an `unsafe fn` (rule a).
- Code that produces addresses hardware will dereference is perimeter material (rule b).
- The perimeter's size is ratcheted (rule c).
- `kf3.c` is perimeter and compiles in CI (rule d).
- §R names five gates. This branch builds gates 1, 3, 4 and 5; gate 2 (S1-02) belongs to `v3-sec-rawaddr`.

**`scripts/ci/perimeter.toml`** (new) is read by every check below. It replaces `AUDITED` (`ci.yml:1522`) at retirement.

```toml
format = 1
toolchain = "1.99.0"          # must equal rust-toolchain.toml; the wrapper execs only this rustc
rustdoc_format = 61
crates = [
  # class U: may contain `unsafe`, only in <crate>/src/**/*_unsafe.rs
  { path = "crates/kf-linux-raw",      class = "U", kf3 = true },
  { path = "crates/kf-qemu",           class = "U", kf3 = true },
  { path = "crates/kf-cuda",           class = "U", kf3 = true },
  { path = "crates/kayfabe-linux-raw", class = "U", role = "grader" },
  { path = "crates/kayfabe-cuda",      class = "U", role = "grader" },
  { path = "firmware/kf-gop",          class = "U", role = "guest-firmware" },
  # class P: compiled -F; its src/**/*_unsafe.rs files are perimeter by rule (b). Empty today.
]
exempt = [ { path = "crates/kayfabe-doorbell", bound = "debt.py frozen",
             reason = "frozen v2 model outside kf3 (dependencies.py:27); 1 unsafe fn + 3 blocks" } ]
standalone = [ # tracked packages that are not root-workspace members, and how G1 compiles them
  { path = "firmware/kf-gop",          runs = ["--lib --tests --target x86_64-unknown-linux-gnu",
                                               "--bins --target x86_64-unknown-uefi",
                                               "--bins --target x86_64-unknown-uefi --features debugcon"] },
  { path = "crates/kf-abi/gen",        runs = ["--all-targets"] },
  { path = "crates/kayfabe-abi/gen",   runs = ["--all-targets"] },
]
build_scripts = { "crates/kf-gop-image/build.rs" = "<sha256>", "crates/kf-gop-image/build/support.rs" = "<sha256>",
                  "crates/kayfabe-isolate-host/build.rs" = "<sha256>", "firmware/kf-gop/build.rs" = "<sha256>" }
c_files = ["qemu/hw/misc/kf3/kf3.c", "qemu/hw/misc/kf3/kf3.h", "qemu/hw/misc/kf3/kf3_gop.h"]
[kf3]
root = "kf-qemu"
external = { libc = ["default", "std"] }          # package -> exact enabled feature set
member_features = { "kf-oprom" = ["alloc", "default"] }   # any other kf3-closure member: no features
custom_build = ["kf-gop-image", "libc"]
[external]          # whole lock, every dep kind; regenerated and reviewed at landing (28 today)
packages = [ "...exact name list..." ]
edges = [ "...member -> external (kind), from cargo metadata resolve..." ]
proc_macros_reachable = ["serde_derive"]          # today only via trybuild (dev)
[cfg]               # predicates permitted on items in perimeter files (generator matrix axes)
allowed = ['test', 'target_arch = "x86_64"', 'target_arch = "aarch64"', 'target_os = "linux"']
[skip_guards]       # macros whose test does not run in CI; see E3d
macros = ["require_kvm"]
[mint]              # §5.3
names = [ ... ]
[mint.baseline]
```

**Where the gates run:**
- New `.github/workflows/perimeter.yml` with three jobs:
  - `perimeter`: lexer, manifest, metadata, sizes, mint, exports, K4.
  - `compiler-location`: G1.
  - `kf3-c`: G7.
- New `.github/workflows/kf3-link.yml` (nightly) and `.github/workflows/perimeter-mutants.yml` (nightly, E3c).
- No `ci.yml` edit until C11. The regions lanes edit (`ci.yml:205-215`, `:1479`, `:1522`) see zero conflict.
- `scripts/ci/test_unsafe_gates.py` runs in the existing discover step (`ci.yml:213`) without an edit there.
- `scripts/ci_gates.sh:128` reads only `jobs.stable`. Extend it to the `perimeter.yml` jobs, and add the three scripts to its heavy list (`:129`).
- The owner adds `perimeter`, `compiler-location` and `kf3-c` as required checks. That is a repository setting, not a commit.

---

## 1. S1-01: the gate rewrite

### 1.1 Summary

| gate | replaces | checks | authority | known positives |
|---|---|---|---|---|
| **G1 compiler location** | gate A's intent (`ci.yml:1525-1549`), gate C's purpose | every `unsafe_code` diagnostic, forced on for every repo unit, is located in a class U `src/**/*_unsafe.rs` of the unit's own package, or in the exempt path | rustc itself, after expansion and `cfg`. Immune to allow attributes, `-A` and `--cap-lints` (measured 2026-10-04). | W1-W12 |
| G2 deny layout | — | **P2 hygiene** (§1.3): an earlier local error. Not a security control. | — | — |
| **G3 lexer** | gate C (`ci.yml:678-718`), gate B (`:1550-1576`), ratchet regex (`:1873`) | L0-L10 over tokens of tracked files | sole authority for the four lexer-only shapes (§1.2); fast first pass and counter otherwise | F1-F16 |
| **G4 manifest/metadata** | gate A's greps; extends `dependencies.py` | M1-M9 (`tomllib`, `cargo metadata`) | exact tables | MF1-MF9 |
| **G5 exports** | the v2 ledger (S1-06) | E1-E14 (§3) | rustdoc JSON plus the G1 log | EF1-EF17 |
| **G6 sizes** | `AUDITED` numbers | §2, exact | lexer counts, cross-checked against G1 counts | SF1-SF4 |
| **G7 kf3.c** | nothing (S1-13) | §8 | compiled against pristine QEMU 10.2.4 with `-Werror`; link and prototype closure | K1-K5 |

### 1.2 G1: the compiler location gate

**New files:** `scripts/ci/rustc_location_wrapper.py` and `scripts/ci/compiler_location.sh`.

**The wrapper**, set as `RUSTC_WRAPPER` (it sees every unit, members or not):

1. It refuses (exit 97, named message) if:
   - `RUSTC_BOOTSTRAP` is set;
   - `argv[1]`'s realpath is not the pinned toolchain's rustc (`rustup which rustc --toolchain <toolchain>`, so nothing can be interposed by `RUSTC` or a configured workspace wrapper);
   - an in-repo unit's arguments contain `--cap-lints`, any `-Z`, or no `--error-format=json`.
2. A unit is in the repo iff `CARGO_MANIFEST_DIR`'s realpath is under the checkout. Units outside the repo (registry, git) are exec'd unchanged.
3. Every in-repo unit gets `--force-warn unsafe_code`. Units not of class U and not exempt also get `-F unsafe_code`, so any `allow` fails early with E0453.
4. It streams rustc's stderr through unchanged, and appends to `$KF_WRAPLOG`:
   - one record per unit: package, manifest dir, crate name, `--test` or not, `--crate-type`, root `.rs`, rc;
   - one record per `unsafe_code` or E0453 diagnostic: message kind, primary file and line, and the file of every `expansion` call-site.

**The verdict** (`perimeter.py location --log`). A diagnostic passes iff one of these holds:
- its primary file **and** every call-site file are `*_unsafe.rs` files under `<P>/src/`, where `P` is the unit's own class U package;
- or it lies under an `exempt` path and `debt.py frozen` passed in the same job.

Every other diagnostic fails with `file:line kind`. Requiring the primary file to be in the unit's own package makes a cross-package `include!`/`#[path]` of a perimeter file fail by compiler evidence. A same-package `include!` stays lexer-only (L4).

**`compiler_location.sh`, in order:**
1. Start from `env -i` with an allowlist: `PATH`, `HOME`, `CARGO_HOME`, `RUSTUP_HOME`, `RUSTUP_TOOLCHAIN`, `KAYFABE_ISOLATE_IMAGE_STUB` (the aarch64 pass, as `ci.yml` does), `CARGO_TERM_COLOR`. Assert that `rustc -vV` reports the pinned version.
2. Run `perimeter.py manifest`, so M4 runs first.
3. Require that the target dir `$RUNNER_TEMP/kf-loc` does not exist. Copy the wrapper and `perimeter.toml` to `$RUNNER_TEMP/kf-gate/` (mode 0555) and verify their hashes against the checkout.
4. Run `debt.py frozen`.
5. Run `cargo check --workspace --all-targets --locked` for `x86_64-unknown-linux-gnu`. If it builds, use `--all-features`; measure that at implementation, and otherwise name the features left out.
6. The same for `--target aarch64-unknown-linux-gnu`, with `KAYFABE_ISOLATE_IMAGE_STUB=1` as at `ci.yml:1912-1916`.
7. Run each `standalone` package's `runs` with `--manifest-path`, under the same wrapper. Firmware uses the firmware job's shapes (`ci.yml:2006`, `:2031`).
8. Run `perimeter.py reached --log`. Every member and standalone target from `cargo metadata` must appear, with the right flags. Targets skipped by `required-features` are named, and the lexer covers them.
9. Run `perimeter.py location --log`.
10. Run `perimeter.py depinfo`: L0 (§1.4) over the `.d` files.
11. Require `git status --porcelain` to be empty. A build step that changed the checkout fails by name.
12. Print `UNITS= DIAGNOSTICS= OUTSIDE=0 EXEMPT=5`.

⊘ **As built at C3 (2026-10-04)**, four details the text above leaves open, each with a known positive:
- The passes are data: `perimeter.toml` `[[location.pass]]` (x86_64 with `--all-features`, aarch64 with the image stub) plus each `standalone` row's `runs`. Every pass adds `--locked --keep-going`, so one refused unit does not hide the rest.
- Each pass runs from its package's own directory. Cargo reads `.cargo/config.toml` by the current directory, not by `--manifest-path`; run from elsewhere, a committed config is invisible and W5/W11 pass for the wrong reason (mutation `cargo run outside the package dir`, killed).
- The wrapper keeps cargo's jobserver descriptors open for rustc (`close_fds=False`).
- Step 11 compares `git status --porcelain` before and after the build (identical to "empty" on a fresh CI checkout, and usable on a developer's tree).
- Beyond W1-W12: **W13** an `allow(unsafe_code)` in a crate with no workspace lints fails with E0453 only because of the wrapper's own `-F`; **W14** a unit compiling an untracked file (L0); **W15** a class U perimeter file no unit compiles (L0, unreached).

In the job, `Swatinem/rust-cache` runs with `cache-targets: false`. Targets `x86_64-unknown-linux-musl`, `x86_64-unknown-uefi`, `aarch64-unknown-linux-gnu` and `aarch64-unknown-uefi` are installed, as in the existing jobs (`ci.yml:63`, `:1899`).

**Why it holds (measured 2026-10-04, see the facts list):**
- force-warn cannot be lowered by any source attribute or command-line lint flag, including `--cap-lints`.
- It reports every unsafe kind, including attributes and `global_asm!`.
- Macro call-sites are visible.
- The fresh target dir plus the reached-count prove the run happened. The gates lens measured a warm run invoking a wrapper zero times.
- Build scripts that run nested cargo strip the wrapper (`kf-gop-image/build.rs:117-121`, `kayfabe-isolate-host/build.rs:275-294`). Those packages are compiled again in steps 5 to 7 under the wrapper. `kayfabe-isolate` is a workspace member, and firmware is a standalone package.

**Shapes rustc cannot see, so the lexer is the sole authority** (each has fixtures):
1. `unsafe` in code that no pass compiles: other `cfg`s or features, and `required-features` targets. L1 is token-level and cfg-blind.
2. Exported macro bodies. Expansions in another crate are silent (measured 2026-10-04). L5.
3. A same-package `include!`/`#[path]` of a perimeter file into a safe module. It is attributed to the included file (measured 2026-10-04). L4.
4. Doctests. They are compiled by rustdoc, not seen by `RUSTC_WRAPPER`. L10.

External crates' exported macros and proc-macros are controlled by M7.

**Cost.** 45.4 s cold for the x86_64 pass (measured prototype, 2026-10-04). Budget about 2.5 min for the whole job, including the aarch64 and standalone passes (unmeasured).

**The first run finds** the 5 doorbell diagnostics measured on 2026-10-04 (exempt by the hash-bound row, `scripts/ci/debt.py:32-37`) and nothing else.

**Known positives.** `compiler_location.sh --selftest` runs first, in a temporary workspace (`KF_ROOT`, its own `perimeter.toml`):
- **W1** The S1-01 crate (empty `[lints]`, `[dependencies.x] workspace = true`, `unsafe` in a safe fn) must fail.
- **W2** The control: the same crate as class U, with the block in `src/x_unsafe.rs`, must pass.
- **W3** W2 with the block in a file not named `_unsafe` must fail.
- **W4** An outer `#[allow(unsafe_code)]` on a fn in a safe file of a class U crate must fail. Measured to pass under `-D` alone (2026-10-04).
- **W5** `RUSTFLAGS`-style `--cap-lints warn` injected through the fixture's `.cargo/config.toml` must fail: the wrapper refuses, and force-warn would still report.
- **W6** Reusing the target dir must fail the reached-count.
- **W7** A non-member path package under an excluded directory, holding `unsafe`, must fail: wrapper record plus M8.
- **W8** A package whose `[lib] path` points into a class U crate's `src/` must fail: M9, and the location rule.
- **W9** A macro defined in a perimeter file and expanded in a safe file must fail on its call-site.
- **W10** A fixture build script that edits a tracked file must fail the clean-tree check.
- **W11** A fixture `RUSTC` that is not the pinned rustc must make the wrapper exit 97.
- **W12** A standalone package with `unsafe` in a safe file must fail.

### 1.3 G2: the deny layout (P2, after `v3-sec-nonpriv`, `v3-broker`, `v3-sec-rawaddr` and `v3-dispsw-exp` merge)

With G1 compiler-authoritative, the deny layout only moves the error from CI to a developer's local build. It needs edits to files that lanes own, so it waits.

**When it lands, as one commit:**
- `crates/kf-{linux-raw,qemu,cuda}/Cargo.toml` (for example `kf-qemu/Cargo.toml:33-38`) change from `unsafe_code = "allow"` to:

  ```toml
  [lints.rust]
  unsafe_code = "deny"
  unsafe_op_in_unsafe_fn = "deny"
  missing_docs = "warn"
  [lints.clippy]
  undocumented_unsafe_blocks = "deny"
  missing_safety_doc = "deny"
  multiple_unsafe_ops_per_block = "warn"
  ```
- `#![allow(unsafe_code)]` goes on line 1 of each then-existing `*_unsafe.rs` that uses `unsafe`. L3 permits it there and nowhere else.
- The `multiple_unsafe_ops_per_block` sites get `clippy-debt.json` entries (21 measured at base `789dee9f`).
- M3 switches to the new tables.

### 1.4 G3: the lexer

**New file:** `scripts/ci/rslex.py`. It handles:
- nested block comments;
- raw strings with hashes;
- byte and C strings;
- char literals versus lifetimes;
- raw identifiers;
- doc comments, kept as text for L10.

It fails closed on an unterminated literal or comment. It scans **`git ls-files '*.rs'`** minus `archive/` and `third_party/`: 720 tracked files at base. There is no `target/` or `mutants.out/` exclusion, because nothing under them is tracked.

**Rules:**
- **L0. File sets.**
  - No tracked symlinks (`git ls-files -s`, mode 120000; 0 today).
  - Every `.rs` path in any G1 dep-info file that lies inside the checkout must be a tracked, scanned file, or a file under that unit's `OUT_DIR`.
  - A `.rs` file under `OUT_DIR` is allowed only as a non-class-U unit's input (0 today).
  - Every `*_unsafe.rs` under a class U `src/` must appear in at least one dep-info file, or it is reported as unreached.
- **L1.** The keyword `unsafe` appears in code only in `*_unsafe.rs` under a class U crate's `src/`, or under an exempt path.
- **L2.** `*_unsafe.rs` files exist only under a class U or P crate's `src/`, at any depth.
- **L3.** The identifier `unsafe_code` appears in code only as `#![forbid(unsafe_code)]` or `#![deny(unsafe_code)]` in a crate root, or as `#![allow(unsafe_code)]` as the first tokens of a `*_unsafe.rs` in a class U crate. Today there is one use: `firmware/kf-gop/src/lib.rs:15`.
- **L4.** In class U and P crates' `src/`: no `#[path`, no `include!(` and no `#[macro_use]` on a module declaration. Each `*_unsafe.rs` is declared by exactly one out-of-line `mod <stem>;` in its parent, and contains no out-of-line `mod`. `include_bytes!` and `include_str!` are allowed. Today: 0 hits. The `#[path]` uses at `kf-gop-image/src/lib.rs:42` and in `kf-rm/tests/` are in forbid crates.
  - ⊘ **Corrected at C1 (2026-10-04): 2 hits, not 0.** `firmware/kf-gop/src/bin/kf-gop-test/main.rs:24` and `:26` declare `mod efi_unsafe;` and `mod port_unsafe;` with `#[path]` naming `src/efi_unsafe.rs` and `src/port_unsafe.rs`: the test application reuses the driver's perimeter modules. L4 permits exactly that shape and nothing wider: a `#[path]` on an out-of-line `mod x_unsafe;` whose target is a perimeter file of the same crate with the stem `x_unsafe`. The module is still that perimeter file, and the compiler attributes its diagnostics to it, so G1's verdict is unchanged. Fixture: `test_F4_a_perimeter_file_reused_under_its_own_stem_is_allowed_and_a_rename_is_not`.
- **L5.** No `#[macro_export]` macro in any class U crate has `unsafe` in its body, including macros defined by macros. Today the only class U export is `require_kvm!` (`kvm_gate.rs:111`, a safe file).
- **L6.** No `safe` qualifier inside an extern block.
- **L7.** Counts per kind feed §2.
- **L8.** Caller-obligation SAFETY comments inside safe fns of perimeter files: an exact count that only goes down. There are 7 sites at base, all LANE:
  - `driver_unsafe.rs`: `memcpy_h2d` `:684/:690`, `memcpy_d2d_async` `:792/:800`, `memset_d8` `:812/:819`, `memcpy_h2d_async` `:1026/:1039`, `view_bytes` `:1560/:1561`, `zeroed` `:1591/:1592`;
  - `ffi_unsafe.rs`: `write_err` `:63/:68`.
  - ⊘ **As built at C2 (2026-10-04).** "Caller obligation" is a phrase family, not the bare word: `every/each/all caller(s)`, `caller(s) size/bound/must/ensure/guarantee/pass/promise`, `caller's obligation`, `the caller passed`, `the caller (`. The bare word matched 60 sites, most of them describing the call (*"the CALLING thread"*, *"borrowed … by the caller in this same expression"*). The family finds exactly the 7 kf3 sites above, plus 7 in the grader crates and firmware (`kayfabe-cuda` 5, `kayfabe-linux-raw` 1, `kf-gop` 1). The baseline is the exact SET in `perimeter.toml` `[l8]`, keyed `<file>::<Type::>fn`, so a fix in one fn cannot be traded for a new site in another. Fixture: `test_F10_a_new_caller_obligation_comment_in_a_safe_fn`.
- **L9.** Mint sites (§5.3).
- **L10.** No `unsafe` token in a doc-comment code fence (untagged, `rust`, `no_run`, `should_panic`, `editionNNNN`) outside the perimeter. Today: 303 fences, 0 hits.

**Fixtures** (`scripts/ci/test_unsafe_gates.py`). Pure Python; the keyword is assembled from fragments.
- F1-F12 as in rev 1:
  - F1: a line-start dereferenced block;
  - F2: `:12://` in a string;
  - F3: controls `// unsafe`, `"unsafe"` and `r#unsafe`;
  - F4: `#[path]` and `include!` in class U;
  - F5: `*_unsafe.rs` in a forbid crate, at depth 1 and depth 2;
  - F6: an outer allow, an inner allow in a safe file, an allow not at line 1, and `cfg_attr(…, allow(unsafe_code))`;
  - F7: the eight kinds the old ratchet misses;
  - F8: an exported macro with `unsafe` in its body;
  - F9: an out-of-line `mod` in a `*_unsafe.rs`;
  - F10: a new L8 site;
  - F11: a new L9 site;
  - F12: an unterminated raw string.
- New:
  - **F13** A `src/target/x_unsafe.rs` in a class U crate is scanned and counted.
  - **F14** A tracked symlink fails.
  - **F15** `unsafe` in a `///` rust fence fails; in a `text` fence it passes.
  - **F16** `#[macro_use] mod x_unsafe;` fails.

### 1.5 G4: manifest and metadata

New file `scripts/ci/perimeter.py` with subcommands `manifest` and `metadata`. M6 lives in `dependencies.py`.

- **M1.** Members come from `cargo metadata --no-deps`, not the `crates/*/Cargo.toml` glob (`ci.yml:1527`). Every member outside class U and exempt has `tomllib(...)["lints"] == {"workspace": True}` exactly.
- **M2.** `[workspace.lints.rust].unsafe_code == "forbid"` (root `Cargo.toml:91-92`).
- **M3.** Class U manifests match exact tables. Until P2-G2 these are the current ones, so deleting `undocumented_unsafe_blocks` (for example `kf-qemu/Cargo.toml:38`) fails. Standalone packages are pinned too: `forbid` at `crates/kf-abi/gen/Cargo.toml:33` and `crates/kayfabe-abi/gen/Cargo.toml:33`, and the firmware table.
- **M4.** No committed `.cargo/config*` sets `rustflags`, `rustdocflags`, `build.rustc`, `build.rustc-wrapper` or `build.rustc-workspace-wrapper`. No workflow `env:` sets `RUSTFLAGS`, `CARGO_ENCODED_RUSTFLAGS`, `CARGO_TARGET_*_RUSTFLAGS`, `RUSTC`, `RUSTC_WRAPPER`, `RUSTC_WORKSPACE_WRAPPER` or `RUSTC_BOOTSTRAP`. Defence in depth: G1's env allowlist already ignores them.
- **M5.** No member or standalone package has `proc-macro = true`.
- **M6 (S1-14)** (`dependencies.py`). kf-qemu's normal+build closure, from `cargo metadata --format-version 1 --locked`:
  - every package is a root-workspace member whose `manifest_path` is `crates/kf-*/Cargo.toml`, or is `libc`;
  - custom-build targets are exactly {kf-gop-image, libc};
  - enabled features equal `[kf3].external` and `member_features`, so `force-host-page-size` (`kf-linux-raw/Cargo.toml:14`, which gates `page_size.rs:106`) cannot be switched on for kf3;
  - each rule gets a unit test in `test_dependencies.py`.
  - New lane crates such as `kf-broker` pass by rule; no list edit is needed.
- **M7.** The set of external packages in the resolved graph (all dependency kinds) equals `[external].packages`. The member→external edges equal `[external].edges`. The proc-macros reachable from any member equal `proc_macros_reachable`. No `git` sources. This closes proc-macros reached through a re-export, and new crates with exported macros.
- **M8.** Every package with `source == null` is a root member or listed in `standalone`. This closes non-member path packages (R1).
- **M9.** Every target's `src_path` realpath lies under its own package directory (R4).
- **M0** (added at C1, 2026-10-04). The toolchain pin agrees everywhere: `perimeter.toml` `toolchain`, `rust-toolchain.toml` `channel`, and every `toolchain:` line in `.github/workflows/*.yml`. The location wrapper execs only the pinned rustc, and the rustdoc JSON format is pinned to it, so a bump that misses one of them fails by name.
- Also: every tracked `Cargo.toml` outside `archive/` and `third_party/` is a member or `standalone`; `build_scripts` hashes match.

**Fixtures:**
- MF1-MF7 as in rev 1:
  - MF1: the S1-01 manifest;
  - MF2: a class U manifest missing `undocumented_unsafe_blocks`;
  - MF3: a member outside `crates/`;
  - MF4: a `.cargo/config` with rustflags, and a workflow env with `--cap-lints`;
  - MF5: a proc-macro member and a direct proc-macro dependency;
  - MF6: the closure gains an external crate, or gains a build script;
  - MF7: a doorbell file hash changes.
- **MF8** A metadata fixture where a non-proc-macro dependency re-exports a proc-macro.
- **MF9** A fixture with a non-member path package (M8) and a borrowed `src_path` (M9).

---

## 2. The perimeter size ratchet (rule c)

**File:** `scripts/ci/perimeter/sizes.tsv`. One row per perimeter file:
- every class U or P `*_unsafe.rs`;
- every VALIDATES module (§3.5);
- the three C files.

| column | counts |
|---|---|
| `code` | lines holding at least one code token, outside `#[cfg(test)]` items. **Exact.** For C files: lines with a token after comments are stripped. |
| `blocks`, `unsafe_fn`, `unsafe_method`, `unsafe_impl`, `unsafe_trait`, `extern_blocks`, `extern_items`, `unsafe_attrs`, `asm` | per kind, including test code (parity with the old ratchet) |
| `macro_unsafe` | per `macro_rules!`: `unsafe` tokens in the body × invocations in the crate |
| `exports` | this file's rows in `PERIMETER_EXPORTS.md`; 0 for C and VALIDATES rows |
| `reason` | as below |

⊘ **As built at C2 (2026-10-04).** `exports` is a column from C2 on, 0 until the table lands at C5. The first landing has no base `sizes.tsv`, so every row carries `YYYY-MM-DD: baseline — …`; from then on the diff rule applies. The lexer reproduced every number below on `789dee9f` (kf-linux-raw's one `unsafe fn` is a method, as the compiler also says), plus `macro_unsafe` = 85 for kf-cuda (`opt!` 1 × 21, `sym!` 2 × 32). The C files' `code` is 780 / 52 / 22 code lines (999 / 83 / 44 total). The grader crates and firmware are counted too.

**Baseline.** The rev 1 lexer baseline at `e4fb0190` stands:
- kf-linux-raw 75: 66 blocks + 8 `unsafe impl` + 1 `unsafe fn`;
- kf-cuda 75: 73 blocks + 2 `unsafe impl`, plus 1 extern block and 2 macros invoked 53 times, which the old ratchet did not count;
- kf-qemu 46: 23 blocks + 13 `unsafe extern fn` + 6 `unsafe impl` + 4 `unsafe fn`, plus 25 `#[unsafe(no_mangle)]`;
- one `asm!` at `mapping_unsafe.rs:901`;
- C files: `kf3.c` 999, `kf3.h` 83 and `kf3_gop.h` 44 total lines.

**Compiler cross-check.** For each perimeter file and kind, the union of G1's x86_64 and aarch64 diagnostic counts, deduplicated by (file, line, column, call-site), must be ≤ the lexer's count. The lexer is cfg-blind, so it can only be higher. A compiler count above the lexer's is a lexer bug and fails by name. Measured at base (2026-10-04): equal per kind, except the one-block x86-only gap the aarch64 pass is expected to close.

**Semantics.** Every column is exact equality, matching the repo's own convention at `ci.yml:866-870` and `:1887`. There is no slack. A decrease needs only the new number; `perimeter.py sizes --update` writes decreases.

**Raising a number.** `perimeter.py sizes --base <ref>` compares against a base: the PR base; on a push, the merge-base with `origin/master`; on master, the first parent. The job needs `fetch-depth: 0`. For every row that rose, and every new row, the `reason` cell must differ from the base's and match:

`YYYY-MM-DD: ` + one `<column>+<delta>` token for each risen column + `— ` + the explanation

Per-crate totals are printed, not stored.

**Fixtures:**
- SF1: a raise with no reason, or with a missing token, fails.
- SF2: `code` +1 or −1 without a TSV change fails.
- SF3: a perimeter file without a row fails, and so does a stale row.
- **SF4:** a compiler count above the lexer's fails.

---

## 3. The export table (§R gate 3)

### 3.1 Location and grammar

The table is **`docs/design/PERIMETER_EXPORTS.md`**. There is one `##` section per perimeter file, and rows are **sorted by item**, which keeps lane inserts apart. There is **no stored OPEN count**: the gate prints it.

```
# Perimeter exports — the reviewed table (OWNER_RULINGS §R gate 3)
STATUS: LIVE, <date>. Generated inventory must equal this table (scripts/ci/perimeter.py exports).
<!-- perimeter-exports format=2 rustdoc-format=61 -->

## crates/kf-linux-raw/src/chardev_unsafe.rs
| item | vis | kind | checks | tests | status |
|---|---|---|---|---|---|
| `CharDevice::ioctl` | pub | safe fn | nonempty; max-size; ioc-size; patch-bounds; no-overlap; region-recheck; scrub | … | OPEN: 2026-10-04: request-agnostic; argument bytes may carry kernel-dereferenced address fields no Indirect patched (finding ii); closes with the typed RM layer (§5.2) |
| `Indirect::describing` | pub | safe fn | zero-len; range-in-region; overflow | range-in-region=t:crates/kf-linux-raw/src/chardev_unsafe.rs::<fn>; … | OK |
```

**Columns:**
- **item:**
  - free fn: `name`;
  - inherent method: `Type::method`;
  - trait-definition method: `Trait::method`;
  - trait-impl method: `<Type as Trait>::method`;
  - impl itself: `<Type as Trait>`;
  - trait: `trait Name`;
  - type, alias, const, static: by name;
  - macro: `name!`;
  - auto-trait fact: `Type: Send` / `Type: Sync`.
- **vis:** `pub`, `pub(crate)`, `pub(in …)` or `default`.
- **kind:**
  - mechanical: `safe fn`, `unsafe fn`, `safe extern fn`, `unsafe extern fn`, `trait`, `unsafe trait`, `trait impl`, `unsafe trait impl`, `type alias`, `static`, `const`, `macro`, `auto trait`;
  - reviewer-assigned for types: `owning handle`, `borrowed view`, `FFI struct`, `plain data`.
- **checks:** labels separated by `;`.
- **tests:** `label=ref` pairs. A ref is one of:
  - `t:<path>::<fn>` (E3d applies);
  - `ui:<path>#<code>` — the `.stderr` must contain the error code, for example `E0603`;
  - `box:<path>` — never sufficient alone;
  - `equiv:<mutant>` — an accepted equivalent mutant, quoting cargo-mutants' description.
- **status:**
  - `OK`;
  - `OPEN: YYYY-MM-DD: <reason>`;
  - `LANE:<branch>: <reason>`.

### 3.2 The generator

Per crate and per target, `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`:

```
RUSTC_BOOTSTRAP=1 CARGO_TARGET_DIR=$RUNNER_TEMP/kf-rustdoc cargo rustdoc -p <crate> --lib --locked [--features <matrix>] \
  -- -Z unstable-options --output-format json --document-private-items --document-hidden-items
```

The results are unioned by (file, item).

`RUSTC_BOOTSTRAP` reaches this step's dependency check units too. The target dir is separate and no gate verdict uses that build. G1's wrapper refuses to run under `RUSTC_BOOTSTRAP`.

The generator stops unless `format_version == 61`. The bump instructions in `rust-toolchain.toml` gain one line naming this constant.

**Membership is by module, not by span.** Walk the module tree from the crate root. Perimeter modules are the out-of-line modules whose file is `*_unsafe.rs`, plus their inline children. Every item they contain is a perimeter item. Every impl whose span lies in a perimeter file is a perimeter item, whatever its Self type: generic, foreign or local (R7). That covers impls inside `const _` and fn bodies (measured present, 2026-10-04).

**Exported means:**
- visibility `public`, `crate`, or `restricted` to a module path outside the item's file (a private item is `restricted` to its own module, measured 2026-10-04);
- a trait-definition method, provided or required, of an exported trait;
- a trait-impl method of a perimeter impl, unless the trait is a marker or `automatically_derived` `Debug`/`PartialEq`/`Eq`/`PartialOrd`/`Ord`/`Hash`;
- an enum variant field takes the **enum's** visibility (fields report `default` and are public by construction, measured 2026-10-04).

**Auto-trait facts.** For every type classified `owning handle` or `borrowed view`, or found address- or fd-carrying by E6/E7, the generator emits `Type: Send` and `Type: Sync` rows from rustdoc's synthetic impls. A type that becomes `Send` or `Sync` changes the row set.

**`unsafe impl`.** These are taken from the G1 log (`implementation of an unsafe trait` at the impl's file and line), because rustdoc 61 reports `is_unsafe == false` for `unsafe impl Send`.

**Reach check (E1b), the generator's own known positive:**
- every `*_unsafe.rs` of a covered crate has at least one item in the union;
- every item name the lexer sees in a perimeter file (fn, struct, enum, union, trait, type, const, static, `macro_rules!`; outside `#[cfg(test)]`, at item level, including inside impls and inline modules) appears in the union for that file.

That closes `#[doc(hidden)]`, generator `cfg` gaps and unreached files.

**cfg rule (E1c).** `cfg`/`cfg_attr` predicates on items in perimeter files must be built from `[cfg].allowed` and matrix features only. `debug_assertions` is banned there, because rustdoc never sees its negation (measured 2026-10-04). Base: `mapping_unsafe.rs:888`, `:895`, `:903` use only `target_arch`, so the rule lands green.

**Crates covered.** The class U crates in the kf3 graph, plus any crate holding class P files. The grader crates and firmware are not linked into kf3 (`dependencies.py:27`; root `Cargo.toml:6`). The table says so in one line, and `sizes.tsv` still counts them.

**Why rustdoc JSON, and not `syn` or a hand parser:** it is the compiler's own item graph after `cfg` and macro expansion, it resolves visibility, and the gates lens measured 0.2 to 5.5 s per crate once dependencies are checked (2026-10-04).

### 3.3 Gate rules

- **E1** Inventory equals the table, per file section. A missing row fails with "add a row" and prints a skeleton; a stale row fails with "remove the row". **E1b** reach check. **E1c** cfg rule.
- **E2** Mechanical columns (kind, vis) equal the generator's values.
- **E3** An `OK` row with kind `safe fn`, `safe extern fn` or `trait impl`, or any trait-definition method row, has a non-empty checks cell, and every label has a `t:` or `ui:` ref that resolves to a non-`#[ignore]` test.
  - **E3b** A `t:` test body names the item: method or function identifier as a token.
  - **E3c** (nightly, `perimeter-mutants.yml`) `cargo mutants` over perimeter files. An `OK` fn row must have zero MISSED mutants in its body, apart from `equiv:` entries. On PRs, `--in-diff` restricted to perimeter files. Measured scope (2026-10-04): 475 mutants in kf-linux-raw perimeter files; run time unmeasured, budgeted at 1-2 h with `--jobs 4`.
  - **E3d** A `t:` ref whose body invokes a `[skip_guards]` macro (`require_kvm!`, `kvm_gate.rs:111-119`; KVM is forced absent in CI per `ci.yml:105`) does not count as CI evidence.
  - **E3e** A `ui:` ref's `.stderr` contains the named error code.
- **E4** Every `unsafe fn` and every `unsafe trait` in a perimeter file, whatever its visibility, has a `# Safety` heading.
- **E5** An `owning handle` has no `Copy`/`Clone` impl and no non-private field. A type with `Drop`, an `OwnedFd` field, or a field of another owning perimeter type must be classified `owning handle`.
- **E6** Address-carrying types:
  - A type is address-carrying if a field type contains `raw_pointer`, `function_pointer`, `NonNull` or another address-carrying type. It also is if it has an integer field named `ptr`, `addr`, `base`, `*_ptr` or `*_addr`, or is a tuple newtype over an integer whose type name contains `Ptr`, `Addr`, `Span`, `Handle` or `Base`.
  - Such a type is never `plain data`, and has no non-private field unless it is an `FFI struct` (`repr(C)` plus a reason).
  - A `type alias` to an integer, raw pointer or fn pointer in a perimeter file is a row. Examples: `CUdeviceptr = u64` (`driver_unsafe.rs:41`, S1-04), `OverlayFn` (`raw_unsafe.rs:129`), `IoeventfdFn` (`:181`).
- **E7** Fd-carrying types (an `OwnedFd`/`BorrowedFd`/`RawFd` field, `i32` in a `*Fd` type, a field `fd`/`*_fd`) are never `plain data`.
- **E8** A `borrowed view` has a lifetime generic.
- **E9 (S1-08)** Safe `extern "C"` fns with a raw-pointer parameter: an exact baseline of 11 that only goes down. They are in `kf-qemu/src/ffi_unsafe.rs` at `:296`, `:304`, `:313`, `:376`, `:423`, `:431`, `:467`, `:476`, `:533`, `:570` and `:609` (verified 2026-10-04).
- **E10** A new row, or one changed to OPEN or LANE, has a dated reason (diff-aware, same base rule as §2). The printed OPEN count is informational.
- **E11** Validation dependencies (§3.5).
- **E12** Generic exports. A safe export with a type parameter, `impl Trait`/`dyn Trait` argument, or callback carries the labels `untrusted-impls` and `panic-safe`. Each label has a test: an adversarial impl (for example an `AsRef` whose length changes between calls) and a panicking callback. S1-09's `view_bytes<T: Copy>` is the class this catches.
- **E13** Traits. A safe trait defined in a perimeter file whose method results a perimeter memory-safety argument relies on must be an `unsafe trait`, or sealed. The row states which, and the reliance.
- **E14** Statics in perimeter files are rows. An address-carrying static type follows E6.

E3 to E9 and E12 to E14 bind `OK` rows. An `OPEN` or `LANE` row records the violation in its reason.

⊘ **As built at C5 (2026-10-04)**, where the text above left a choice or a measurement moved it:
- **`unsafe impl` kinds come from the tokenizer**, not the G1 log: the perimeter job has no G1 log, and the tokenizer's `unsafe impl`/`unsafe trait` sites are exact by file and line and cross-checked against the compiler by SF4 (§2).
- **E9's baseline is 12, not 11**: the 11 in `kf-qemu/src/ffi_unsafe.rs`, plus kf-cuda's `completion_hostfn` (`driver_unsafe.rs`), a safe `extern "C"` fn taking `*mut c_void`.
- **E10 is stricter than diff-aware**: every `OPEN`/`LANE` status carries `YYYY-MM-DD: ` by grammar (`LANE:<branch>: YYYY-MM-DD: …`), so no row can lose its date later; an OK row that goes back to OPEN at the base comparison is printed.
- **E11 is derived by the tokenizer**: every same-crate `src/` file that defines, at module level and outside `#[cfg(test)]`, an item a perimeter file's code tokens name. It finds root re-exports by definition, which is what R14 asked of rustdoc. The derived set is the design's expected six VALIDATES files (`bounds.rs`, `cache.rs`, `geometry.rs`, `ioctl.rs`, `page_size.rs`, `view.rs`) plus five USES (`census.rs`, `error.rs`, `ioctltrace.rs`; kf-qemu `chan.rs`, `device.rs`).
- **A trait impl is a row when its Self type is exported, foreign or generic.** An impl for a type private to the file cannot be reached from outside it.
- **E1b leaves out non-exported `macro_rules!`**: rustdoc documents only exported macros.
- **E13 and E14, mechanically**: an OK safe-trait row carries the check `sealed` or `no-reliance`; an OK static whose type carries an address fails.
- **The first table: 441 rows, OK 0, OPEN 133, LANE 308** (the x86_64 inventory at `789dee9f`; the CI union over both targets is the authority). No row is OK yet: none has had its mutation run (§3.4 step 2). LANE rows are the files a lane edits (`perimeter.toml` `[exports.lanes]`).
- **The cargo self-test has 29 cases**: EF1-EF17 as above, plus EF2b (a test that never names the item), a `pub(super)` item visible outside its file, a derived `Clone` as a row, a wrong mechanical kind (E2), an undated OPEN row (E10), a safe trait OK without `sealed` (E13), an OK static of an address-carrying type (E14), and a `ui:` ref whose `.stderr` lacks its error code (E3e) with its control.

### 3.4 Initial content

1. `perimeter.py exports --skeleton` prints every row with its mechanical columns filled and status `OPEN: <date>: unreviewed`.
2. A row becomes `OK` only when E3 to E3e hold **and** a local `cargo mutants --file <f> --re <fn>` run showed zero missed. Otherwise it stays `OPEN: unproven`.
3. Lane files get `LANE:` rows.
4. `CharDevice::ioctl` is OPEN (R9).
5. `raw_unsafe.rs` and `ffi_unsafe.rs`, which have no unit tests (`wire_mirror.rs` checks layouts only), are `OPEN: no test shows <label>`.

Expected scale: about 216 functions (kf-linux-raw 111, measured 2026-10-04: 110 safe, 1 unsafe), plus the trait-definition and trait-impl method rows (96 `default`-visibility fns in kf-linux-raw's perimeter files, measured, most of them derived or marker and excluded), about 49 types, 6 constants, the type aliases and the auto-trait facts. The generator's count is the authority.

### 3.5 Validation dependencies

A `## Validation dependencies` section lists every same-crate, non-perimeter file that defines an item named in a perimeter file's code tokens. Root re-exports are followed through the rustdoc JSON: for example `crate::HostOffset` → `bounds.rs:31`, `crate::HostPageSize` → `page_size.rs:75`. The section must equal the derived set.

**Roles:**
- **VALIDATES:** a perimeter memory-safety argument relies on the item. The file joins `sizes.tsv` and gets perimeter review.
- **USES:** anything else.

**Expected VALIDATES entries in kf-linux-raw:** `bounds.rs` (`:78-101`), `geometry.rs` (`:51-65`), `ioctl.rs` (`:59`), `page_size.rs` (`:89`, `:106`), `cache.rs`, `view.rs` (`:42-98`).

**Renaming.** §R puts validation functions in `_unsafe` files, and a VALIDATES file is perimeter that `ls` no longer enumerates. **P2:** rename them to `*_unsafe.rs`, with `pub use x_unsafe as x;` keeping their paths. They are not lane-edited, but their `mod` lines (`kf-linux-raw/src/lib.rs:272-290`) are adjacent to lines `v3-broker` and `v3-sec-nonpriv` edit.

Validation that happens **in another crate's caller** (for example `kf-trap/src/bar1db.rs:106-124` before `OverlayHook::submit`) is the anti-pattern §R forbids. The export's row is OPEN.

### 3.6 Self-tests that need cargo

`scripts/ci/selftest_perimeter_cargo.py` is deliberately not named `test_*.py`. It builds a fixture crate under rustdoc, and each case must fire:
- EF1-EF10 as in rev 1:
  - EF1: a new `pub fn` with no row, and a stale row;
  - EF2: empty checks, a label with no test, and a ref that resolves to nothing;
  - EF3: an `unsafe fn` with no `# Safety`;
  - EF4: a `Drop` type that derives `Clone`;
  - EF5: `plain data` on a raw-pointer type;
  - EF6: an address-carrying type with a `pub` field;
  - EF7: a `borrowed view` with no lifetime;
  - EF8: an E9 entry above the baseline;
  - EF9: an unlisted dependency;
  - EF10: a `format_version` mismatch.
- **EF11** A `#[doc(hidden)] pub fn`, and a `#[doc(hidden)] pub mod`.
- **EF12** A `pub trait` with a provided method that has no row.
- **EF13** A blanket impl written in a perimeter file, and an impl of a perimeter trait for `Vec<u8>`.
- **EF14** A `cfg(not(debug_assertions))` perimeter item (E1c).
- **EF15** An OK row citing only a skip-guarded test (E3d).
- **EF16** A generic export without `untrusted-impls` (E12).
- **EF17** A `type alias` with no row, and a variant field of a pub enum treated as public.

Also, for the nightly job: a fixture with a vacuous test must yield a missed mutant (E3c).

⊘ **As built at C6b (2026-10-04):** `scripts/ci/selftest_mutants.py` runs cargo-mutants over a one-function fixture twice. With a vacuous test, its OK row fails E3c (4 missed). With a test that pins the behaviour, the row passes. `perimeter.py mutants` maps each MISSED mutant by its own line (a function's span starts at its doc comment) to the fn's row, normalizing generic arguments (`<X as From<E>>::from` ↔ `<X as From>::from`). The nightly matrix covers kf-linux-raw, kf-qemu and kf-cuda; run times are unmeasured.

---

## 4. Proposal (a): unchecked preconditions become `unsafe fn`, or are checked inside

### 4.1 NOW (files no lane edits)

The free files are:
- `chardev_unsafe.rs`, `kvm_unsafe.rs`, `vcpu_unsafe.rs`, `window_unsafe.rs` and `signal_unsafe.rs`;
- `epoll_unsafe.rs`, `affinity_unsafe.rs` and `sysconf_unsafe.rs`, which already check their inputs.

None of them appears in any lane diff. Every test below must fail on today's code.

| # | item | change | test |
|---|---|---|---|
| a1 | `CharDevice::ioctl` (`chardev_unsafe.rs:565`) | **A host overrun the exports lens confirmed.** `declared == 0` is accepted with any non-empty buffer (`:606-616`), while the kernel's generic handlers write fixed sizes (Linux `fs/ioctl.c`). Refuse `declared == 0` unless the request is in a named `LEGACY_SIZES` table (`FIONREAD`→4) and `arg.len()` is at least that. No signature change. Every kf3 caller builds its request with `ioctl::readwrite(..., arg.len())` (for example `kf-host/src/event.rs:143`), and `v3-broker`'s DRM requests are sized (`0xC024_6443`, `0xC020_6441`), so no caller changes. | `…::a_sizeless_request_outside_the_legacy_table_is_refused`; `…::a_legacy_request_with_a_short_buffer_is_refused` |
| a3 | `ioctl_arg` (`kvm_unsafe.rs:737`) | Check `_IOC_DIR == NONE && _IOC_SIZE == 0` inside; take `BorrowedFd<'_>`. | an `_IOR`-encoded request is refused before any syscall |
| a4 | `KvmVm::set_memslot` (`:480`) | Private `fn`; its only caller is `KvmMemslot::install` (`:666`). | `ui:crates/kf-linux-raw/tests/ui/set_memslot_is_private.rs#E0624` |
| a5 | `KvmVm::adopt` (`:240`) | Return `Result<Self>` via `confirm_is_a_vm` (`:381`). Its caller is `tests/kvm_vm_discovery.rs:158`. | `adopt` of a `/dev/null` fd returns `Err`, without `/dev/kvm` (no skip guard) |
| a6 | `VcpuExit::Mmio.len` (`vcpu_unsafe.rs:509`) | Bound by `data.len()` (8). A larger value becomes a refused exit. Keep the field type, because `kf-chan/tests/common/mod.rs:105` matches on the variant. | unit test of the narrowing fn |
| a7 | `GuestWindow::userspace_addr_at` (`window_unsafe.rs:504`) | `unsafe fn` with `# Safety`. The call at `kvm_unsafe.rs:494` gains one block, itemised in sizes. | E4 |
| a8 | `place`/`place_device_view`/`restore` via `fixed_map` (`:342-391`) | **A SIGSEGV from safe code the exports lens confirmed.** Required property: after any placement error, the range is the anonymous filler again, or the window is poisoned (`WindowPoisoned` from every accessor; `Drop` leaks the range). Mechanism: map at a kernel-chosen address, then `mremap(MREMAP_FIXED\|MREMAP_MAYMOVE)`. Fallback: `MAP_FIXED`, then a `MAP_FIXED_NOREPLACE` anonymous re-plug, then poison. ⚠ Whether NVIDIA device mappings survive `mremap` is unverified (box bar). The fallback's concurrent-reader window is OPEN. | a `#[cfg(test)]` fault seam: re-plugged, then `read_into` works; a foreign mapping planted in the gap poisons the window, and `Drop` leaves it |
| a9 | `place_device_view(writable=false)` (`:288-312`) | Refuse it. The only caller passes `true` (`kf-qemu/src/mem.rs:354`). | refused by name |
| a10 | `place(Backing::SharedFile)` (`:203-232`) | `fstat`; refuse if offset + len > `st_size`. Truncation after placement is OPEN (needs `F_SEAL_SHRINK`). | a short memfd is refused |
| a11 | `signal_unsafe.rs` | Delete the empty banner (`:237-239`). `install_break_handler` is OPEN: SIGUSR1 is QEMU's `SIG_IPI` (qemu-10.2.4 `include/qemu/main-loop.h:32`), with no kf-* caller. | — |

⊘ **As built at C7b (2026-10-04), a7-a10**:
- **a8 keeps the single `MAP_FIXED` on the success path; the mremap mechanism was not adopted.** The PRAMIN window move places through these doors on the vCPU, and OWNER_RULINGS (line 19) fixes that move at one host map plus one `mmap`; mremap would add a syscall to every move. The failure path is the design's own fallback: after a failed `MAP_FIXED`, re-plug the filler with `MAP_FIXED_NOREPLACE`, which can displace nothing, and poison the window if that cannot succeed. A file-backed `MAP_FIXED` (a memfd, a device node) clears the old pages before the file's `mmap` handler runs, so a refusing handler leaves a gap the re-plug fills. A failed anonymous `MAP_FIXED` keeps the old mapping, the re-plug meets `EEXIST`, and the window is poisoned: fail-closed, and rare (ENOMEM). From poisoning on, every accessor (`read_into`, `write_from`, `store_u32`, the placement doors, `host_span`, `userspace_addr_at`) returns `WindowPoisoned`, and `Drop` leaks the range. ⚠ OPEN: a foreign anonymous mapping that merged into the filler's VMA cannot be told from the filler. The fault seam is `window_unsafe.rs`'s `#[cfg(test)] mod seam`.
- **a9** returns `Unsupported` ("a read-only device view inside a guest window").
- **a10** compares `fstat`'s size with `offset + len` (`OutOfRange`, `object_len` = the file size).
- **a7**: `userspace_addr_at` is a `pub(crate) unsafe fn`; `set_memslot` gains the one block.
- **Counts:** window_unsafe.rs blocks 6 → 15. Five are new in production code: the address computation split from the `MAP_FIXED` block, its `mmap`, the re-plug's `mmap` and `munmap`, and `fstat`, against the one block they replace. Five are in tests. `unsafe_method` +1. kvm_unsafe.rs blocks 6 → 7. The old ratchet in `ci.yml` (`AUDITED`) moves 75 → 86 in the same commit: until C11 retires it, a commit that adds `unsafe` must move it. This is the one `ci.yml` line this branch edits before C11.
- **Tests**, each failing on the code before C7b: `a_failed_placement_re_plugs_the_filler_and_the_window_stays_readable`, `a_foreign_mapping_planted_in_the_gap_poisons_the_window`, `a_read_only_device_view_is_refused_by_name` and `a_short_memfd_is_refused`. The memslot test (KVM-gated) calls the now-unsafe fn through one closure.

⊘ **As built at C7a and C7c (2026-10-04):**
- **a1**: `LEGACY_SIZES = [(FIONREAD, 4)]`. A short buffer for a legacy request is `IoctlSizeMismatch { declared: 4, … }`, and any other sizeless request is the new `RawError::SizelessIoctl`.
- **a3**: `ioctl_arg` takes `BorrowedFd<'_>` and refuses (`Unsupported`, "a by-value ioctl") any request whose direction or size field is non-zero. The two crate-private `as_raw` accessors became `borrow_fd`.
- **a4's test is a `compile_fail,E0624` doctest** on `KvmMemslot::install`, not a trybuild row. `v3-sec-rawaddr` edits `tests/compile_fail.rs`'s `REQUIRED_ROWS`, and a doctest keeps that file out of this branch. The table row for `set_memslot` is gone, because it is no longer exported.
- **a5**: `adopt` returns `Result` and runs `confirm_is_a_vm`. Its one caller, `tests/kvm_vm_discovery.rs`, unwraps it.
- **a6**: `mmio_exit` returns `VcpuExit::Unhandled { reason: EXIT_MMIO }` for a length above 8. The `VcpuExit` type is unchanged.
- **a11**: the empty banner is deleted. `install_break_handler` stays OPEN.
- **Mutations (each applied alone; the crate's tests or doctest rerun):** 12, all caught. They were the a1 refusal and its short-buffer arm; a3's whole check and its direction-bit half (caught after one more case, `_IOW` with size 0); a4 public again (the doctest); a5 unconfirmed; a6 unbounded; a8 without re-plug, without poison, and with `Drop` unmapping a poisoned window; a9; a10.

### 4.2 After `v3-broker` merges: a2 (S1-40, narrowed)

The design is as rev 1: a required `SizeSpec`; the funnel computes, checks and writes the size field; and `Indirect::len()`'s caller contract (`chardev_unsafe.rs:365-377`) is removed.

**Why it waits.** It changes `Indirect::new` (`:266`), and `v3-broker`'s `drm.rs:304` (on its branch) calls `Indirect::new(ptr_at, params)`. Landing first breaks broker's merge.

**What it closes, honestly:**
- It closes the **value** half: no caller can state a size that disagrees with the region.
- The **location** of the size field (`SizeSpec::InArg { at, … }`) and `Fixed(n)` stay caller-declared, so a wrong offset still lets RM use a caller-written value. `kf-host/tests/rm_size_fields.rs`, which pins kf-host's declared offsets to kf-abi's generated layouts, is a check of call sites, not of the boundary.
- S1-40's status: **narrowed**. The `Indirect` rows can be OK on their own checks. The location half closes with §5.2.

**Caller edits.** `kf-host/src/lib.rs:880`, `:936`, `:995`, `:1073` and `:1865`, one line each. Callers already write the computed values (`:868`, `:924`, `:988`, `:1064`; `limit: len - 1` at `:1855`).

### 4.3 P2 (this branch, after the named lane merges)

**After `v3-broker`** (it edits `mapping_unsafe.rs`, `host_fd_unsafe.rs`, `raw_unsafe.rs`, `ffi_unsafe.rs`):
- **`adopt_fd`** (`host_fd_unsafe.rs:25`) → `unsafe fn`.
- **`HostSpan::within`** (`mapping_unsafe.rs:945`) → `unsafe fn`.
- **`MappedRegion::addr_at`** (`:790`) → `unsafe fn`.
- **`HostSpan`** → `HostSpan<'a>`, or an `Arc` keep-alive (today `Copy` with no lifetime, `:931-983`).
- **`VolatileRegion::copy_out`** (`:1153-1168`) → Relaxed atomic word loads, and the `Sync` argument (`:1010-1019`) restated.
- **`stitch`** (`:528`) and **`map_fixed_in`** (`:1315`): the a8 gap handling.
- **`reprotect`** (`:449`) takes `&mut self`.
- **`request_huge_pages`** (`:666`): refuse shared or device backings.
- **`MappedRegion::map`**: refuse `SharedFile` access past end-of-file.

**After `v3-broker`, `v3-dispsw-exp` and `v3-sec-rawaddr`** (`ffi_unsafe.rs`):
- `dev` (`:57`) and `write_err` (`:63`) → `unsafe fn`.
- The 11 E9 entries → `unsafe extern "C"`, with `# Safety` naming `h`.
- `OverlayHook::submit` (`raw_unsafe.rs:167`) checks overflow-safely against the lengths captured at `adopt`.
- Consider `pub(crate) mod ffi_unsafe` (`kf-qemu/src/lib.rs:15`): `#[no_mangle]` keeps C linkage. Measure at implementation, since `wire_mirror.rs` imports from it.

**After `v3-sec-nonpriv`, `v3-broker` and `v3-dispsw-exp`** (kf-host):
- **`CharDevice::fd_number`** (`chardev_unsafe.rs:529`) returns `BorrowedFd`. The complete caller list:
  - `kf-host/src/lib.rs:455`, `:1459`, `:1885`;
  - `kf-host/src/event.rs:139`, `:151`;
  - `kf-qemu/src/device.rs:358`;
  - `kf-harness/src/bin/kf-gate2.rs:61`, `kf-gate3.rs:235`, `kf-gate4.rs:333`, `kf-gate5.rs:93`, `kf-gate6.rs:102`, `kf-gate7.rs:70`, `kf-gate8.rs:364`.
- **The typed RM layer (§5.2), then rule (a) on the funnel:**
  - `CharDevice::ioctl` becomes `pub unsafe fn`, with the precondition "every kernel-dereferenced address field in `arg` is covered by a patch".
  - Typed safe verbs sit above it, in a class U file.
  - `v3-broker`'s `drm.rs` calls move into `drm_unsafe.rs` (B10).
- **P2-G2** the deny layout (§1.3).
- **VALIDATES renames** (§3.5).
- **S1-16:** put `kvm_unsafe`, `vcpu_unsafe` and `affinity_unsafe` behind a feature that kf-qemu does not enable, which shrinks kf3's linked perimeter. Their users are `kf-chan/tests/common/mod.rs:9-105` and `kf-linux-raw/tests/kvm_vm_discovery.rs`. M6 then pins the feature off, and the G5 matrix includes it.

### 4.4 LANE

**LANE:v3-sec-rawaddr** owns `kf-cuda/src/driver_unsafe.rs`, `display.rs`, `walk.rs` and the kf-qemu Kf3Frame. The rev 1 table stands:
- S1-04: raw-address methods become a `DeviceSlice`;
- `launch_args` (`:749-782`) and `graph_exec_kernel_set` (`:1204-1248`) are checked against a per-kernel table, and `launch_raw` becomes private;
- `launch_host_signal` (`:945-957`) and `completion_hostfn` (`:1477`);
- `pinned_free` (`:982`) and `PinnedBuf` (`:1345-1402`);
- the owning handles (`:1308-1527`) become non-`Copy`;
- `open_soname` (`:294`);
- `ExportedAllocation` (`:1618`) and `import_and_map` (`:1686`);
- S1-09 (`PlainBytes`);
- `write_err`.

Add: the `CUdeviceptr` alias (`:41`) is a `type alias` row (E6).

---

## 5. Proposal (b): producers of hardware-dereferenced addresses

### 5.1 Ranked producers

The ranking is the hwaddr lens's, at `789dee9f`. Changes from rev 1 are marked.

| rank | producer | where | disposition |
|---|---|---|---|
| 1 | NVOS02 pinned extent, `limit = len - 1` in safe code; **and** every NVOS pointer field written by safe code into `arg` | `kf-host/src/lib.rs:1855`, `:1865`; `kf-abi/src/generated/nvos.rs:248`, `:1093` (`pub u64`) | **Changed.** Value half: a2, after broker. Location half and unpatched pointers: §5.2. Until then the `CharDevice::ioctl` row is OPEN. |
| 2 | compose kernel arguments | `kf-cuda/src/display.rs:124-155`, `:302-341`, `:435-446`; `kf-disp/src/scanout.rs:313` | LANE:v3-sec-rawaddr, LANE:v3-broker |
| 3 | walker kernel arguments | `kf-cuda/src/walk.rs:1042-1075`, `:1700`, `:1948`; `kf-qemu/src/device.rs:358` | LANE:v3-sec-rawaddr; P2 optional `kf-mem/src/vasmgr.rs:214` |
| 4 | Translated CE operands | `kf-chan/src/translated.rs:687-850`; `kf-qemu/src/chan.rs:492-523` | LANE:v3-p1p2 |
| 5 | GPU PTEs via NVOS46 | `kf-host/src/channel.rs:294-361`; `kf-mem/src/ledger.rs:13-29`, `:303-305`, `:405-430` | LANE:v3-p1p2; P2: `HostVas::map` checks inside |
| 6 | passthrough births | `kf-host/src/channel.rs:588-650`, `:1048-1070`; `kf-qemu/src/chan.rs:2743-2799` | LANE:v3-sec-nonpriv, v3-p1p2, v3-dispsw-exp |
| 7 | identity windows | `kf-qemu/src/mem.rs:1790`, `:1793` | LANE:v3-p1p2 (S1-21) |
| 8 | `MAP_FIXED` sources | `window_unsafe.rs:203-312`; `kf-qemu/src/mem.rs:344-395`, `:1699-1736` | NOW: a8 to a10. LANE:v3-p1p2: typed fds, `pramin_plan`. |
| 9 | BAR1 overlay | `kf-trap/src/bar1db.rs:106-124`; `kf3.c:430` | P2 `OverlayHook::submit`; kf3.c follow-up K1 |
| 10 | host ring `gp_entry`/`fence_words` | `kf-chan/src/host.rs:41-52`, `:338-404` | P2: class P worked example |
| 11 | hand-coded offsets | `kf-host/src/channel.rs:257-259`, `:1053-1057` | LANE:v3-sec-nonpriv / v3-p1p2 |
| 12-13 | FB layout, doorbell tokens | `kf-chip/src/bar0.rs:360-425`; `kf-host/src/lib.rs:713-791` | no action |

### 5.2 Classes P and U for producers (corrected)

**Class P** (a forbid crate whose `*_unsafe.rs` files L2 permits) works for a producer whose output is a **type** the perimeter consumes: private fields, a checking constructor, plus L9. Rank 10 is the worked example.

**Class P cannot close rank 1.** The funnel takes raw bytes. To refuse unpatched pointer fields, it needs each request's layout from a source it can trust. A layout type defined in kf-linux-raw with a public constructor can be minted by any safe caller, and a forbid crate cannot call an `unsafe fn` constructor. The typed RM layer therefore has to be compiled where `unsafe` is allowed.

**P2 target:** `crates/kf-host/src/rm_unsafe.rs` as kf-host's only perimeter file, with kf-host class U. It holds:
- every NVOS builder (02/21/33/34/46/47/54/64);
- the pointer and size-field layout per structure, derived from kf-abi's generated layouts and compiled into the file;
- the only calls to the then-`unsafe` `CharDevice::ioctl`.

This adds a fourth class U crate. `perimeter.toml` is the list (§9 supersedes the "exactly THREE crates" sentence). kf-host's `lib.rs` is edited by three lanes, so this is P2.

### 5.3 Mint sites (L9)

- **Names:** as in rev 1 (`Nvos02ParametersWithFd` … `graph_exec_kernel_set`).
  - ⊘ **As built at C2 (2026-10-04).** Rev 1's list was not in hand; the list in `perimeter.toml` `[mint]` was reconstructed from §5.1's producers: the NVOS 02/21/33/34/46/47/54/64 parameter structs, `Indirect`, `WindowVa` (no site on master yet; `v3-p1p2`), `KfArgs`, `Desired`, `launch_raw`, `launch_args`, `graph_exec_kernel_set`. Aliases introduced by `use … as` and `type … =` are followed to a fixpoint per crate, and their uses count. Baseline 81 sites in 10 files (`kf-host/src/lib.rs` 46).
- **Scope:** code tokens in `src/` of kf3-graph crates, outside `#[cfg(test)]`.
- **Not counted:** perimeter files and kf-abi.
- **Counted exactly, and only goes down:** every other site, **plus** every `use … <name> as …` rename and every `type … = <name>`.
- **Baseline:** rev 1's table, recomputed by the lexer at landing.
- **Known limit:** a value produced by type inference (`Default::default()`, or a kf-abi helper returning the struct) never writes the name. L9 is a ratchet; type privacy (§5.2) is the control.
- `v3-broker` adds an `Indirect` site in a safe file (`drm.rs:304`) (B11).

---

## 6. The §2.1 minors

| minor | disposition |
|---|---|
| **RawRegion `Copy` with no lifetime (S1-07)** | LANE:v3-p1p2. **Corrected:** the liveness check belongs **inside `raw_unsafe.rs`**. `RawRegion` becomes non-`Copy` and holds an `Arc<BlockLease>`. Every load and store checks the lease under its read side, and `kf3_ram_del` takes the write side and waits out live leases, off the vCPU. A copy that safe code keeps cannot outlive the block. Re-resolving in `UserdView` (caller-side) is not the control. Test: register, view, `ram_del`, and the next load is refused by name. Fix `store_u32`'s doc or its store (`raw_unsafe.rs:48`, `:54`). |
| **`view_bytes`/`zeroed` (S1-09)** | LANE:v3-sec-rawaddr: a sealed `unsafe trait PlainBytes`, with trybuild rows `#E0277`. E12 flags the class. |
| **Raw fds as `i32` (S1-10)** | `import_and_map` (`driver_unsafe.rs:1703`) and `ExportedAllocation.fd`: LANE:v3-sec-rawaddr. `fd_number`: P2, full caller list in §4.3. `BackendFd` (`raw_unsafe.rs:100-121`): LANE:v3-p1p2, the same lease. |
| **Ratchet blind spots (S1-11)** | NOW, via C2: per-kind lexer counts cross-checked against the compiler's (§2). The `sym!`/`opt!` signature table against `cuda.h` is LANE:v3-sec-rawaddr. |

---

## 7. S1-06: the export table becomes the ledger

Confirmed at `789dee9f`: `crates/kf-linux-raw/tests/unsafe_naming.rs:41` lists the v2 crates; `walk()` returns silently on a missing directory (`:175-177`); the v2 tree satisfies the floor (`:274-288`). No lane edits the file.

C5 deletes it and replaces it (⊘ done at C5, 2026-10-04: the v2 ledger's `sandbox_unsafe.rs` and `spawn_unsafe.rs` rows have no v3 counterpart, since kf-linux-raw has neither file):

| ledger part | replaced by |
|---|---|
| `SOUNDNESS_CRITICAL_SAFE_FILES` | §3.5, derived |
| row: `chardev_unsafe.rs` (Indirect size field) | **narrowed** by a2 (after broker). The location half stays OPEN until §5.2. |
| row: `kvm_unsafe.rs` `adopt` | closed by a5 |
| row: `vcpu_unsafe.rs` MMIO length | closed by a6 |
| rows: `sandbox_unsafe.rs`, `spawn_unsafe.rs` | no v3 counterpart; recorded in the C5 message |
| the books-balance count | the computed OPEN count, plus E10 |
| crate list | M6 plus the kf3 closure; a missing directory is an error |

---

## 8. kf3.c in CI (rule d, S1-13)

### 8.1 Tier 1: the `kf3-c` job, on every push

**New files:**
- `scripts/ci/kf3c.sh`;
- `scripts/ci/qemu-10.2.4.tar.xz.sha256`, computed once by the implementer **and** checked against the upstream `.sig` (record the signer's fingerprint in the file's comment);
- `scripts/bench/build_kf3.sh` is **not edited**. `kf3c.sh` extracts the `CONF_FLAGS=` assignment from it (`:55-58`), refuses if the extraction is empty, and adds `--disable-download` (verify the flag at implementation; the release tarball carries its subprojects). `provision_bench_tree.sh:82` checks no checksum; recorded as a follow-up.

**Runner.** `container: ubuntu:26.04@sha256:<digest>`, because the lens measured gcc 15.2.0 and clang 21.1.8, which are Ubuntu 26.04's. `ubuntu-latest`'s compilers are unmeasured, and gcc-only flags differ by version. If a container is not wanted, measure on the chosen runner in a non-required job first.

**Recipe:**
1. `apt-get install ninja-build libglib2.0-dev libpixman-1-dev pkg-config python3-venv clang gcc`.
2. Fetch the tarball, verify the sha256, and cache it with `actions/cache` keyed on the hash file.
3. `tar -xJf … --exclude='qemu-10.2.4/roms'`.
4. Configure with the extracted `CONF_FLAGS` (11 s, measured 2026-10-04). `--enable-pixman` is required (`kf3.c:690-691`).
5. Generate headers with `ninja $(ninja -t query libsystem.a.p/hw_misc_edu.c.o | awk '/\|\|/{print $2}')`: 354 headers, 3.4 s. kf3.c and edu.c are both in `system_ss` (`qemu/hw/misc/kf3/meson.build:8`; qemu `hw/misc/meson.build:2`), so edu.c's flags are kf3.c's.
6. Take edu.c's `compile_commands.json` entry and compile:

   ```
   cc $FLAGS -Werror -Wextra -Wno-unused-parameter -Wno-sign-compare -MD -MF $T/kf3.d -c <repo>/qemu/hw/misc/kf3/kf3.c -o $T/kf3.o
   ```

   This produces an object, because gcc's flow-dependent warnings need codegen.
7. Run `clang -fsyntax-only -Werror -Wno-unknown-warning-option` and `clang --analyze -Xanalyzer -analyzer-werror`, after filtering gcc-only flags (the rev 1 list). Measured 2026-10-04: rc 0, 0 reports.
8. **Link closure.** The `kf3_*` symbols in `nm -u kf3.o` equal the names following `#[unsafe(no_mangle)]` in `crates/kf-qemu/src/ffi_unsafe.rs`. Measured 25 = 25.

**Known positives:**
- **K1** A temp copy with an undeclared identifier must fail.
- **K2** `cc -E` output contains `kf3_ov_apply`.
- **K3** Dropping one Rust export name in memory fails the closure check.
- **K5** (new) `kf3.d` lists `<repo>/qemu/hw/misc/kf3/kf3.h` and `kf3_gop.h` and no other `kf3*.h`. This proves the in-tree headers were compiled.
- **K4** (new, runs in the `perimeter` job; `kf3.h` includes only `<stdint.h>`/`<stddef.h>`, `:7-8`):
  - generate C prototypes from kf-qemu's rustdoc JSON for every `extern "C"` `no_mangle` fn, using a fixed type map (`*mut c_void`→`void *`, `u32`→`uint32_t`, …);
  - `cc -fsyntax-only -Werror` with `kf3.h` (prototypes at `:38-82`) followed by the generated prototypes;
  - a mismatch is a "conflicting types" error, which the link closure (names only) cannot see;
  - known positive: one parameter type flipped in memory must fail.

**Estimated cold run:** 2 to 3 minutes (container plus apt), unmeasured on a runner.

⊘ **As built at C6 (2026-10-04):**
- **The tarball:** sha256 `821b545b…0746`, 141 137 724 bytes. The same bytes were verified against `qemu-10.2.4.tar.xz.sig`: a good signature from Michael Roth, primary key fingerprint `CEAC C9E1 5534 EBAB B82D 3FA0 3353 C9CE F108 B584`, recorded in `scripts/ci/qemu-10.2.4.tar.xz.sha256`.
- **The container:** `ubuntu:26.04@sha256:3595d7fc…804e`, the 26.04 image index on 2026-10-04.
- **`--disable-download`** is accepted by 10.2.4's configure (`configure:756`).
- **Two more known positives run every time:** **K0**, a planted unused variable must fail the strict build, which proves `-Werror` is in force; and a foreign `kf3.h` path planted in the depfile list must fail K5.
- **Measured locally** against an existing 10.2.4 build directory with pixman forced on (gcc 15.2.0, clang 21.1.8): kf3.c is clean under gcc `-Werror -Wextra`, clang `-Werror` and the analyzer, and the closure is 25 = 25. Mutations: dropping `-Werror`, a closure that always passes, and a K5 that always passes are each caught by K0, K3 and K5. Disabling K2 is not caught: K2 is itself the positive control that the preprocessed unit is kf3.c's.
- **Tier 2** (`kf3-link.yml`) runs `build_kf3.sh` unmodified into a fresh, hash-checked tree, then checks `-device help` for `"kf3-gpu"` through a file, never through a pipe into `grep -q`.

**Lanes this job will compile:** `v3-broker` (+444 lines of `kf3.c`), `v3-dispsw-exp` (+11), `v3-viommu` (realize). Tell them before the owner makes the job required (B7, D2, V1).

### 8.2 Tier 2: `.github/workflows/kf3-link.yml`

As in rev 1:
- **Triggers:** nightly, dispatch, and pushes touching `qemu/**`, `crates/kf-qemu/**` or `scripts/bench/build_kf3.sh`.
- **Steps:** `bash scripts/bench/build_kf3.sh <fresh tree> <fresh build>`, then `-device help > out.txt` and `grep -q kf3-gpu out.txt`. Never pipe into `grep -q`; nvkvm-pv's `release.yml:218-219` records that failing under `pipefail`.
- **Never cache the tree:** `build_kf3.sh:42-43` patches it.
- **Estimate:** 7 to 10 min.
- It needs the `x86_64-unknown-uefi` target for `kf-gop-image`'s build script.

### 8.3 aarch64 (P2)

The same recipe on `ubuntu-24.04-arm` with `--target-list=aarch64-softmmu`.

---

## 9. Docs

**`docs/design/THE_CONSTRAINTS.md` §13 (`:107-112`).** Replace the item. `v3-sec-p0` inserts at `:122` and `v3-sec-nonpriv` edits `:556`; neither collides.

> 13. **Memory safety is a perimeter, not a keyword** (owner, 2026-10-03, `OWNER_RULINGS.md` §R).
> - **The perimeter.** A file named `*_unsafe.rs` is the audit perimeter: it may hold raw pointers, unvalidated offsets and lengths, and validation code.
> - **Exports check their own inputs.** Every item a perimeter file exports to safe code checks overflow, range in the real allocation, alignment and lifetime. A precondition left to callers is an `unsafe fn` (rule a). `unsafe {}` appears only where Rust requires it.
> - **Address producers are perimeter.** Code that produces an address hardware dereferences is perimeter material even as safe Rust (rule b).
> - **Size and kf3.c.** The perimeter's size is ratcheted (rule c), and `kf3.c` is inside it (rule d).
> - **Where unsafe may appear.** Crates that may hold `unsafe` are listed in `scripts/ci/perimeter.toml` (class U). CI compiles every unit in the repository with the `unsafe_code` lint forced on, and fails unless every reported use lies in a `*_unsafe.rs` file of its own class U crate. Shapes the compiler cannot see (exported macros, `include!`, code no build compiles, doctests) are checked by the tokenizer.
> - **Where it is checked.** `.github/workflows/perimeter.yml`; the export table `docs/design/PERIMETER_EXPORTS.md`; sizes `scripts/ci/perimeter/sizes.tsv`.
> - ⊘ **Superseded 2026-10-03 (§R):** *"exactly THREE crates may opt out … Host addresses cross safe code only as the opaque `kf_linux_raw::HostSpan`, backend fds only as `kf_qemu::raw_unsafe::BackendFd`"*. The list moved to `perimeter.toml`. Both tokens are `Copy` with no lifetime; their rows are OPEN.

**`docs/audits/2026-10-03-v3-stage1.md`.** Fold each status into the finding's own `- **Status:**` line, in the closing commit:
- **S1-01 (C3):** "closed by replacement": the compiler location gate (forced lint, located per diagnostic, fresh target, reached-count), the tokenizer for the four lexer-only shapes, and M7/M8. Residual: `unsafe` arriving from an external crate's macro is invisible to the compiler; M7 pins the external set.
- **S1-06 (C5).**
- **S1-11 (C2).**
- **S1-12 (C1, C3).**
- **S1-13 (C6; tier 2 nightly).**
- **S1-14 (C1).**
- **S1-40:** "narrowed (value half)" at a2.
- The bridge gate's unanchored filter (`ci.yml:827-828`) is an observation for §3.1's gate table.

**`docs/design/V3_SECURITY_AUDIT_PLAN.md`** (`v3-sec-nonpriv` edits `:36`, `:168`):
- §2.1 (`:53-62`) names `perimeter.yml` (G1-G7);
- §2.2 (`:64-75`): the inventory is `PERIMETER_EXPORTS.md`; a test per precondition is E3 plus E3c;
- Miri and Kani stay stage 2;
- §4: update the status cells.

**S1-83.** `perimeter-mutants.yml` (E3c) is its first instrument. The perimeter checkers are the first fuzz targets.

---

## 10. Commit order and expected conflicts

| commit | content | why green | hardware merge bar? |
|---|---|---|---|
| C0 | this design (STATUS: DESIGN) | docs | no |
| C1 | `rslex.py` (git ls-files), `perimeter.toml`, `perimeter.py lex/manifest/metadata` (L1-L6, L10, M1-M5, M7-M9; M3 at current tables), M6 in `dependencies.py`, `test_unsafe_gates.py`, job `perimeter` | L1 parity 0 hits; 0 symlinks; 0 doctest fences; external set and edges generated | no (touches no product source; assert it on the diff) |
| C2 | `sizes.tsv` (exact), L8, L9 | baselines generated | no |
| C3 | `rustc_location_wrapper.py`, `compiler_location.sh` with `--selftest` (W1-W12), L0, job `compiler-location` | measured: 0 diagnostics outside the perimeter except the 5 exempt ones | no |
| C5 | generator, `selftest_perimeter_cargo.py`, `PERIMETER_EXPORTS.md` (OPEN unless mutation-proved), §3.5, `sizes.tsv` exports column, delete `kf-linux-raw/tests/unsafe_naming.rs`, K4 | generated from the tree | no |
| C6 | `kf3c.sh`, sha256 and signature note, job `kf3-c` (container), `kf3-link.yml` | `kf3.c` at master compiles clean (measured with the pinned compilers) | no |
| C6b | `perimeter-mutants.yml` (nightly E3c) | nightly only | no |
| C7a | a1 | no signature change | **yes** (every RM call goes through the funnel) |
| C7b | a7 to a10 | — | **yes** (placement; measure `mremap` of device views) |
| C7c | a3 to a6, a11 | tests run without `/dev/kvm` | yes |
| C8 | THE_CONSTRAINTS §13, audit plan; STATUS: LIVE | docs | no |
| P2-a2 | a2 plus kf-host's 5 lines plus `rm_size_fields.rs`; S1-40 narrowed | after `v3-broker` | yes |
| P2-* | §4.3, §5.2 (`rm_unsafe.rs`, kf-host class U), P2-G2, VALIDATES renames, S1-16 feature | each after its lane | yes for code |
| C11 | retire `ci.yml` gate C (`:678-718`), gates A and B, the ratchet and `AUDITED` (`:1137-1890`); anchor the bridge filter (`:827-828`). Keep the host-pointer gate (`:752-783`) for `v3-sec-rawaddr`. | after one green merge cycle, and after `v3-sec-nonpriv` and `v3-broker` (both edit `AUDITED`) | no |

C4 of rev 1 is gone: its content is P2-G2.

**Order relative to the lanes.** Land C1 to C6 early: CI only, so the lanes see the gates before they merge. C7 needs the box merge bar at the exact commit (§R, "Testing before master"). This lane uses no boxes; the coordinator schedules that run.

**What lanes will meet at merge after C1 to C5** (loud, by design):
- sizes rows and table rows for their new perimeter files and exports;
- E9 for new safe `h` entries;
- L9 for new mint sites;
- M7 when the external set changes.

**Expected conflicts:**

| with | where | textual | semantic (loud) |
|---|---|---|---|
| v3-sec-nonpriv | `ci.yml` `AUDITED` | none until C11 | `capability_unsafe.rs` sizes row and table rows (N1); `capability.rs` is a VALIDATES candidate (N4) |
| v3-sec-nonpriv | `kf-cuda/Cargo.toml` (+4 at `[dependencies]`, `:8`) | none (M3 pins `[lints]`, `:35-42`) | new edge `kf-cuda → kf-linux-raw`: M6 passes by rule |
| v3-broker | `kf-linux-raw` `mod` lines, `Cargo.toml` members | none (no deny layout now) | `drm_unsafe.rs`/`unixsock_unsafe.rs` rows and sizes (B1); E9 (B5); `kf-broker` passes M1/M6 by rule (B6); `kf3.c` compiled (B7); `drm.rs` validation is VALIDATES or moves (B10); L9 `Indirect` site (B11); P2-a2 then needs B9 |
| v3-dispsw-exp | `ffi_unsafe.rs`, `kf3.c`, `kf3.h`, `clippy-debt.json` | none | `kf3_realize` row (D1); `kf3.c` compile and K4 (D2) |
| v3-sec-rawaddr | `driver_unsafe.rs`, `display.rs`, `walk.rs`, `ffi_unsafe.rs` Kf3Frame | **none: this branch no longer edits them** | LANE rows; its S1-02 gate reuses `rslex.py` and `perimeter.toml` (R11) |
| v3-p1p2 / v3-viommu | not pushed / equals master | — | `WindowPoisoned` in `mem.rs` (P7); the RawRegion lease (P1); `kf3.c` realize compiles under K1-K5 (V1) |

---

## Appendix A: per-lane follow-ups (record; do not edit here)

⊘ **Added at C8 (2026-10-04), from what the implementation changed or found; these supersede the matching items below where they differ:**
- **Every lane that adds `unsafe` to kf-linux-raw before C11:** `ci.yml`'s `AUDITED` is now `kf-linux-raw:86`, after a7-a10. Add your blocks to 86, and give each new or risen `sizes.tsv` row its dated reason.
- **v3-sec-rawaddr:**
  - **R17.** Your hand-written `Debug` impls for `GuestWindow` and `RunMapping` become export-table rows at merge (E1 names them).
  - **R18.** Your perimeter ratchet (`b2021f43`) and `sizes.tsv` count the same files. Keep one, exact per kind, so the counts do not disagree.
  - **R19.** `THE_CONSTRAINTS.md` §13: this branch rewrote only its first line, and your appended paragraph should merge cleanly. Read the merged item once.
  - **R20.** S1-12: this branch's status bullet sits between Found and Check; your Status-line edit stands. The host-pointer gate's known positive is yours.
  - **R21.** kf-cuda's `completion_hostfn` is the twelfth E9 entry.
- **v3-broker:**
  - **B12.** `RawError` gains `SizelessIoctl` and `WindowPoisoned` mid-enum, away from your `BadSocketPath` at its end.
  - **B13.** Your three new safe `h` entries raise E9 from 12 to 15 unless they become `unsafe extern "C"` (B5).
  - **B14.** `drm.rs`'s `Indirect` site is a new `[mint.baseline]` row (B11).
  - **B15.** `CharDevice::ioctl` now refuses a sizeless request outside `LEGACY_SIZES` (a1). Your DRM requests are sized, so none is affected.
- **v3-p1p2:**
  - **P9.** `GuestWindow` can now return `WindowPoisoned` from every accessor (a8). `mem.rs`'s `place_view`, `place_ram` and `sink` should treat it as device-fatal (P7).
  - **P10.** `place_device_view(writable = false)` is refused (a9), and `place(SharedFile)` past end-of-file is refused (a10).
- **v3-sec-nonpriv:** **N5.** The `AUDITED` line conflicts by number only: add your `capability_unsafe.rs` blocks to `kf-linux-raw:86`.

**v3-sec-rawaddr:**
- R1 to R14 as in rev 1:
  - R1: S1-04, `DeviceSlice`; `CUdeviceptr` stops being public;
  - R2: launch argument tables;
  - R3: the hostfn fd lifetime;
  - R4: `pinned_free`;
  - R5: non-`Copy` handles;
  - R6: `open_soname`;
  - R7: `ExportedAllocation` and `import_and_map`;
  - R8: `PlainBytes`;
  - R9: clear L8's six sites;
  - R10: Kf3Frame length and `write_err`;
  - R11: the S1-02 gate on `rslex.py` and `perimeter.toml`, with known positives;
  - R12: split the `multiple_unsafe_ops_per_block` sites;
  - R13: compose bounds into the perimeter, plus a proptest;
  - R14: flip rows with dated reasons.
- **R15** The S1-02 gate should also flag string literals naming the process's own memory file (`/proc/self/mem`, `/proc/<pid>/mem`) outside the perimeter. That is safe-Rust access to host memory that no compiler gate sees.
- **R16** At P2-G2, add the line-1 allow to `driver_unsafe.rs` and `ffi_unsafe.rs`, or let this branch do it after the merge.

**v3-p1p2:**
- **P1** (corrected) The RawRegion lease lives inside `raw_unsafe.rs`, checked by every accessor; `kf3_ram_del` waits out leases.
- P2 to P8 as in rev 1:
  - P2: `BackendFd`, with the same lease;
  - P3: `Desired` gets private fields; `HostVas::map` re-checks; `reserved` becomes required;
  - P4: `WindowVa` minted in a perimeter file (L9 −7);
  - P5: S1-21 and S1-23;
  - P6: `pramin_plan`'s store-length check;
  - P7: `WindowPoisoned` is device-fatal;
  - P8: typed fds.

**v3-sec-nonpriv:**
- N1: `capability_unsafe.rs` rows and sizes row.
- N2: typed USERD/GPFIFO slices (S1-43); generated fields for `reserve_va` and `alloc_context_dma`.
- N3: move the `AUDITED` reasoning into `sizes.tsv` reasons.
- **N4** `capability.rs` (622 lines, safe): classify it VALIDATES or USES.

**v3-broker:**
- B1: `drm_unsafe.rs` and `unixsock_unsafe.rs` rows and sizes rows.
- B2: `host_fd_unsafe.rs` additions get rows.
- B3: `MappedRegion::host_span` waits for `HostSpan<'a>`.
- B4: classify `BrokerHooks`.
- B5: the three new `h` entries become `unsafe extern "C"`.
- B6: `kf-broker` manifest is exactly `[lints] workspace = true`.
- B7: `kf3.c` +444 lines clean under §8's flags.
- B8: `LayerPlan` carries a typed store slice.
- B9: `drm.rs:304` passes a `SizeSpec` after P2-a2, and its `:281-292` check is deleted.
- **B10** `drm.rs`'s request allowlist and size-field check (`nvidia_ioctl`, `nvidia_import`) are validation the perimeter relies on. Move them into `drm_unsafe.rs` (required anyway when `CharDevice::ioctl` becomes `unsafe fn`), or list `drm.rs` as VALIDATES.
- **B11** The `Indirect` site in `drm.rs` raises the L9 baseline with a reason.

**v3-dispsw-exp:**
- D1: `kf3_realize` row (ABI 13).
- D2: `kf3.c` and `kf3.h` compile, and K4 passes.
- D3: new NVOS mentions raise L9 with a reason.
- D4: `chan.rs` USERD and notifier offsets (S1-43).

**v3-viommu:**
- V1: realize edits compile under `kf3-c`.

**kf3.c, whoever next edits the overlay block:**
- K1: `kf3.c:430-431` becomes `len > lim || off > lim - len`.
- K2: S1-03 stride units at `:697`.
- K3: `:795-798`, document or `dup()` the fd the EventNotifier takes ownership of.

**Bench tooling (whoever owns `provision_bench_tree.sh`):**
- verify the QEMU tarball against `scripts/ci/qemu-10.2.4.tar.xz.sha256` (`:82` downloads without a check).

---

## Appendix B: facts relied on, and where they come from

| fact | source |
|---|---|
| force-warn immunity, kinds reported, macro chains, `include!` attribution, external macros silent, `-D` outer-allow and `-F` cap-lints-warn behaviour | measured 2026-10-04, rustc 1.99.0, scratch |
| `RUSTC_WRAPPER` vs `RUSTC_WORKSPACE_WRAPPER` on non-member path packages; borrowed `[lib] path` accepted; dep-info contents | measured 2026-10-04, cargo 1.99.0, scratch workspaces |
| 304 units, 1031 diagnostics, 5 outside (doorbell), per-kind counts, 45.4 s | measured 2026-10-04: force-warn prototype over a `git archive` of origin/master `789dee9f` |
| nested builds strip `RUSTC*` | `kf-gop-image/build.rs:117-121`, `build/support.rs:41-51`, `kayfabe-isolate-host/build.rs:275-294`; and 0 nested units in the measured log |
| rustdoc JSON facts (format 61, doc(hidden), trait methods, local blanket impls, `debug_assertions`, variant fields, item counts) | measured 2026-10-04, rustdoc 1.99.0 |
| 475 mutants in kf-linux-raw perimeter files | `cargo mutants --list`, cargo-mutants 27.1.0, 2026-10-04 |
| kf3 closure, features, custom builds, proc-macros, 28 external packages | `cargo metadata --locked` at `789dee9f` |
| lane hunk positions | `git diff $(git merge-base origin/master origin/<lane>) origin/<lane>` on 2026-10-04 |
| G1's rev 1 trap measurements, kf3.c recipe and its 25 = 25, a1 and a8 probes | gates, hwaddr and exports lenses (rev 1, 2026-10-04) |

⊘ **Measured since, on GitHub runners (2026-10-04, branch `v3-sec-perimeter`):**
- **`--all-features` builds the workspace.** The x86_64 location pass uses it, and the whole gate passed in run 37168007181: 625 units, 2303 `unsafe_code` diagnostics, 0 outside the perimeter, 10 exempt (doorbell's 5 in each root pass).
- **The aarch64 pass closes the one-block gap.** SF4 prints no difference for `kf-linux-raw/src/mapping_unsafe.rs` blocks in that run. The differences it does print are expected: kf-cuda's 3 macro-body blocks (their 85 expansions count against `macro_unsafe`), extern items, and inline `asm!`, none of which the lint reports.
- **`--disable-download` is accepted by 10.2.4's configure.** The `kf3-c` job passed once `bzip2` was installed.
- **Runner timings:** `perimeter` about 1 min, `compiler-location` about 2 min, `kf3-c` about 1.2 min, `kf3-link` 3 min 47 s.
- **Not chosen:** a8 does not use mremap at all (§4.1, as built), so the first item below is moot for this branch.

**Not verified:**
- whether NVIDIA device mappings survive `mremap` (a8);
- runner timings for every job, and E3c's run time;
- the runner's compilers, if no container is used;
- whether `--all-features` builds the workspace (C3);
- whether `--disable-download` is accepted by the 10.2.4 configure;
- whether QEMU's alias clipping contains a wrapped overlay offset;
- the exact OPEN baseline (generated at C5);
- the aarch64 pass closing the one-block cross-check gap at `mapping_unsafe.rs:895-902`.
