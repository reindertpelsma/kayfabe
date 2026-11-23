# Packaging and distribution: the QEMU release, the build kit, the command-line surface, trust and the manual

**STATUS: DESIGN-ONLY, 2026-11-20.** Records the owner's design discussion of 2026-11-20 with the coordinator, and the decisions taken in it (the decisions are recorded as `OWNER_RULINGS.md` §BC). Nothing in sections 1 to 12 is built unless a sentence says so. Every "today" statement was checked against `origin/next` at `7fb1c98a0` on 2026-11-20 with `git grep` or by reading the named file, and says so; a number that was not re-counted says where it came from. Related, not superseded: [`V3_STANDALONE_TRUST.md`](V3_STANDALONE_TRUST.md) (LIVE; section 8 here refines its attestation anchor), [`V3_KFROOT_ROOTLESS.md`](V3_KFROOT_ROOTLESS.md), [`V3_DRIVER_MATRIX.md`](V3_DRIVER_MATRIX.md), [`V3_FLAG_INVENTORY.md`](V3_FLAG_INVENTORY.md), [`V3_SECURITY_MODEL.md`](V3_SECURITY_MODEL.md) section 7.

Labels. **OWNER DECISION (2026-11-20)**: stated by the owner in the discussion, binding. **DESIGN**: a proposal that follows from a decision; the owner has not seen the detail. **CODE READ**: read in the tree on 2026-11-20, not run. **MEASURED**: names the run or record. **NOT RUN**: a mechanism that exists in code or in a manual and that nobody has exercised. **OPEN**: a question this document does not answer (section 16).

## 0. The goal in one paragraph

**OWNER DECISION.** A person who can already pass a GPU through to a VM with VFIO should be able to use kayfabe in four steps and no more: (1) download a release, or build it; (2) get a `qemu-system` binary that has the kayfabe device; (3) install their own guest and its NVIDIA driver; (4) start the VM with the kayfabe device where the VFIO device was. The only host requirements are KVM and access to the NVIDIA device nodes; no further root. Humans must not have to learn NVIDIA internals to run the project, and nothing a user needs may hide in an environment variable, a bench script or an internal document. The same repository must let anyone reproduce and audit every test without access to any private repository.

## 1. What is shipped: a QEMU build, and a kit that makes one

### 1.1 Decisions

1. **OWNER DECISION. The distributed product is a specific patched QEMU build** (a release binary). Alongside it ships a **build kit** that overlays the kayfabe device onto any QEMU source tree (for example the tree Proxmox builds from) and builds either a full `qemu-system-x86_64` or only a loadable module. This extends `OWNER_RULINGS.md` §AS (one patched binary is acceptable, a module is preferred where it works, the user flow is "compile QEMU, then use your own tools").
2. **OWNER DECISION. A module must match the exact QEMU build it is loaded into.** QEMU enforces that for its own modules (build identity); kayfabe does not work around it.
3. **OWNER DECISION. Configuration uses the mechanism QEMU provides natively for a module**: a user-creatable `-object` umbrella (this document uses the working name `kayfabe-gpus`, OPEN question Q2) plus `-device kf3-gpu,...`. It must not be a `vl.c` patch, and a user must be able to understand it from the QEMU manual's own vocabulary.
4. **OWNER DECISION. QEMU-specific code stays in `kf3.c`** so that a second front end (Cloud Hypervisor is the named one) can sit on the Rust core through the numbered C ABI (the kf3 ABI of `qemu/hw/misc/kf3/kf3.h`, `KF3_ABI` is 36 in `kf3.h` on 2026-11-20; see `OWNER_RULINGS.md` §AO for the hypervisor-independence goal).

### 1.2 What exists today (CODE READ)

⊘ **Correction 2026-11-23 (CODE READ; the table row "GPL and the release binary" below is the 2026-11-20 state, kept as written).** Task I7 landed on 2026-11-20 after this document: the guest-ingress allowlist of `kf-abi` was re-derived from NVIDIA's open-gpu-kernel-modules and kayfabe's own measurements, `LICENSE` exception 2 was narrowed to files that are not in the device's link closure, and `scripts/ci/qemu_license_closure.py` runs in the `rust-mutants-b` job of CI (`OWNER_RULINGS.md` §BD; `qemu_license_closure.py` run on 2026-11-23 at `next` `92a7f2782`: CLEAN, 19 workspace crates, findings=0). So the GPL exception is **no longer a standing blocker**; a counsel check before the first public binary is recommended, not blocking. The security gate (§P) row is unchanged, and whether a binary may be published is still the owner's decision (Q10).

| Piece | Today | Gap |
|---|---|---|
| Device sources | `qemu/hw/misc/kf3/`: `kf3.c` 2134 lines, `kf3.h` 289, `kf3_gop.h`, `kf3_guest_ram.h`, `meson.build` | none to overlay: the kit is already a copy |
| Overlay procedure | `scripts/bench/build_kf3.sh <qemu-tree> [<build-dir>]` runs `cargo build --release -p kf-qemu`, copies `*.c`, `*.h`, `meson.build` and `libkf_qemu.a` into `hw/misc/kf3/`, appends `subdir('kf3')` to `hw/misc/meson.build` and a `config KF3` stanza to `hw/misc/Kconfig` (`default y if TEST_DEVICES`), configures, runs `ninja` | it is a bench script that also installs per-revision binaries and holds a lock; the user-facing kit is a separate, smaller entry point (section 1.4) |
| QEMU version | 10.2.4 only; tarball pinned in `scripts/bench/qemu.pin`, fetched by `scripts/ci/qemu_source.py` | other versions are "not documented" (`docs/howto/building.md`); Proxmox's tree is a different QEMU version and carries its own patches: OPEN Q9 |
| Linking | `system_ss.add(when: 'CONFIG_KF3', if_true: [files('kf3.c'), kf3_dep])`: the device and the Rust static archive are linked **into** the binary | **no module**: nothing in the tree builds `kf3` as a loadable module; `OWNER_RULINGS.md` §AS says feasibility "is NOT yet studied" and that is still true. QEMU's module machinery covers a fixed set of directories; whether `hw/misc` devices can be made modules without a larger patch than today's two hunks is the first question of the feasibility study (OPEN Q1) |
| Type registration | `type_register_static` only (`kf3_register_types`); no `-object` type, no machine hook | the umbrella object (section 2) |
| QEMU-specific code only in `kf3.c` | the Rust core is reached through `kf3_realize(...)` and the numbered ABI (`crates/kf-qemu/src/ffi_unsafe.rs`); `kf-qemu` itself is named for QEMU but holds the plane code | an audit that no QEMU header type leaks into `kf-qemu`'s public surface, and a second front end as a proof: DESIGN-ONLY |
| GPL and the release binary | `LICENSE` exception 2 and `docs/howto/credits.md`: a QEMU binary built from this tree "is not yet distributable under the GPL" (`kf-abi/src/capability.rs` is nvproxy-derived, Apache-2.0 only, and `kf3` links `kf-abi`) | **a blocker for a public release binary, not changed by this document** (OPEN Q10) |
| Security gate | `OWNER_RULINGS.md` §P: "no public binary ships before every stage-2 blocker is closed or accepted by the owner"; `V3_SECURITY_AUDIT_PLAN.md` says no stage-2 findings document exists | **also a blocker, not changed here** (OPEN Q10) |
| Attested release build | `.github/workflows/release-build.yml` builds the Rust workspace and `kf-gop.efi` from one GitHub checkout and attests four files (`kayfabe-isolate-host`, `kayfabe-rm-ladder`, `kf-oprom`, `kf-gop.efi`) | the QEMU binary and the module are **not** among them |

**UPDATE 2026-11-21 (CODE READ, built, the workflow has NOT RUN on GitHub).** The user-facing build kit of section 1.4 exists as `scripts/release/build_qemu.py` and the attested QEMU release as `.github/workflows/release-qemu.yml` (manual dispatch, draft release only; `docs/CI_V3.md`, section *QEMU release workflow*). It builds only the full `qemu-system-x86_64` (no module: Q1 is still open), refuses every QEMU version except 10.2.4 by name, and takes the QEMU source as the pinned official tarball (anchor: its qemu.org signature, `scripts/release/qemu-release.pin`; `scripts/bench/qemu.pin` stays the cache key), a tarball you give, or a tree you give. Two corrections to this section's text: (1) `scripts/ci/qemu_source.py` and its pin are a cache key, and the anchor of section 8.1 for a QEMU tarball is the signature, because qemu.org publishes signatures and no checksum document; `kf_trust.py` has no signature anchor class, so the kit verifies with `gpg` itself and routes only the fetch of the signature file through `kf_trust.py`. (2) Cargo builds from a registry and from vendored sources give different archives (measured: the `kf_linux_raw` objects differ), so the kit vendors the crates for every build; the source tarball carries them. Measured on one machine: two builds in two directories give byte-identical tarballs and manifests; whether the GitHub runner's build equals a local one is not claimed. The two blockers of Q10 are unchanged: the workflow only ever makes a draft.

### 1.3 Design: the umbrella and the module

**DESIGN, not studied, no QEMU code was read for this.** The four-step story needs three pieces, each of which has a QEMU-idiomatic shape. These shapes come from QEMU's own conventions (configuration objects such as `memory-backend-*` or `iothread` that devices reference by id; `-global` for per-type defaults) and have not been tried on 10.2.4.

- **Per-GPU settings stay device properties.** `-device kf3-gpu,...` is already what the manual documents (section 2).
- **Global defaults** for every GPU: QEMU's generic `-global kf3-gpu.<property>=<value>` already applies a property to every instance of the type. `docs/howto/installing.md` describes it, but no script in the tree uses it (`git grep -- '-global kf3'` finds only that sentence): **NOT RUN**. Whether it is enough, or the umbrella object carries the defaults, is OPEN Q2.
- **The umbrella `-object`** is the place for settings that belong to the whole VM rather than to one device (the limits family, the broker socket directory, a default UUID policy, "all GPUs"). Whether an `-object` can *create* devices (the "kayfabe on, no GPU named, bind all GPUs" case of section 2) or only be referenced by id from each device is the open design question: the idiomatic QEMU role of an `-object` is a configuration holder referenced by id, and creating PCI devices from one needs a bus and slot rule. A fallback that stays inside QEMU's vocabulary is that libvirt, Proxmox or the launcher script writes one `-device kf3-gpu` per GPU and the umbrella carries the shared limits.
- **A loadable module** is `-module`-less in QEMU terms (modules are found by name from the module directory), so a user-facing flow is "install the module next to a stock QEMU of the exact same build, then `-device kf3-gpu`". Everything in section 1.2 about the exact match applies.

### 1.4 Design: the build kit

**DESIGN.** One entry point for a user: unpack or point at a QEMU source tree, run the kit, get either a full binary or a module (OPEN Q1 decides whether the second is possible). The kit is the user-facing slice of `build_kf3.sh`: overlay, configure with a documented flag list, build, print one line that names the QEMU version, the kayfabe revision and the result path. It never installs anything and takes no lock. The bench build (per-revision private installs, a shared build directory, the flock) stays a CI helper (section 11). Overlay hunks in the QEMU tree stay as small as today (one meson line, one Kconfig stanza); the kit refuses a tree whose `VERSION` is not on a list the kit knows and says which it knows, instead of building and failing later.

## 2. The command-line surface

### 2.1 Today, from the property table (CODE READ, `kf3_properties` in `qemu/hw/misc/kf3/kf3.c`)

27 `DEFINE_PROP` lines. Defaults are as declared; two of them differ from what the owner asked for or are about to change (flagged).

| Property | Default | Meaning |
|---|---|---|
| `gpu-minor` | 0 | which host GPU, by NVIDIA device minor (`/dev/nvidia<minor>`) |
| `fb-mb` | 8192 | guest VRAM in MiB (a fixed default, not derived from the host card) |
| `bar1-size`, `bar2-size` | 256 MiB, 32 MiB | the guest's BAR1 and BAR2 sizes |
| `bar1-overlays` | 64 (`KF3_BAR1_OVERLAYS_DEFAULT`) | Hopper and later: BAR1 doorbell views trapped at once |
| `msix-vectors` | 32 | |
| `guest-driver` | unset | declared guest driver version; unset = the host's own version (section 4) |
| `dummy-bar` | off | |
| `display` | off | virtual display heads |
| `gop`, `gop-efi` | off, unset | boot display driver, signed |
| `x11-dispsw` | off | experiment, needs `display=on` |
| `gpu-uuid` | unset = `auto` | `auto` (hash of VM id, host GPU UUID and PCI slot), `random`, `host`, or an explicit UUID (`crates/kf-rm/src/gpuuid.rs`) |
| `vm-id` | unset (falls back to QEMU's `-uuid`) | the VM identity `auto` derives from |
| `channel-budget` | 0 = derived from the host (free channels per runlist minus one eighth; refused below 256 or above 2048) | `V3_CHANNEL_BUDGET.md` |
| `user-view-keep` | 0 = 16 (max 256) | |
| `windows-support` | **on** | the one opt-out of the Windows profile (`OWNER_RULINGS.md` §AT) |
| `report-utilization` | **on** | the guest may read the physical GPU's utilisation (§AV) |
| `unmapped-reads-zero` | off | sparse-residency reads. **Open PR #507 changes this to `auto`** (on for Windows-profile guests); this table is `next` |
| `doorbell-ioeventfd`, `doorbell-ioeventfd-max` | off, 256 | doorbell fast path |
| `display-broker`, `display-broker-uid`, `display-broker-vram` | unset, -1, auto | the broker socket (absolute path; unset = off), one more accepted uid, GPU-copy rung |
| `display-max-fps` | 0 = a cap of 75 | 24..75; refused below, above, or without `display=on` |
| `x-gsp-observer`, `x-gsp-observer-seconds` | unset, 300 | diagnostic |

PCI placement needs no kayfabe property: the device is a QEMU PCI device (parent `TYPE_PCI_DEVICE`), so QEMU's own `bus=` and `addr=` apply, and the device reads its devfn (`Config::pci_devfn`) as an input to the `auto` UUID.

### 2.2 The owner's knob list against today

| Requested knob | Exists today? | Gap or note |
|---|---|---|
| GPUs to bind | yes, per device: `gpu-minor` | selection is by NVIDIA minor only, not by PCI address or UUID; no "all GPUs" |
| VRAM per GPU | yes: `fb-mb` | per device; a global default needs `-global` or the umbrella |
| GPU UUID mode manual / same / derived from VM id | yes: `gpu-uuid` explicit / `host` / `auto` with `vm-id` | mapping of the owner's "same" to `host` is my reading (OPEN Q2b); `random` is an extra |
| nvidia-smi utilisation visibility, default on | yes: `report-utilization`, default on | |
| DoS limits | partly | section 2.4 |
| PCIe index | yes, QEMU's `bus=`/`addr=` | |
| Broker socket path per GPU | yes: `display-broker` | per device |
| Headless display framerate | yes: `display-max-fps` (24..75) | |
| Headless display resolution | **no** on the device. The broker has `--resolution auto\|none\|WxH` and `--refresh auto\|none\|Hz` as broker flags (`broker/nvkvm_broker.c`) | the kf-disp default monitor is 1920x1080 at 60 Hz (`crates/kf-disp/src/edid.rs`); no property sets a mode: OPEN Q6 |
| Framelock with the broker | **no** property. The lock-step between kayfabe and the broker exists and is always on when a broker is connected (`OWNER_RULINGS.md` §AL; `crates/kf-disp/src/lockstep.rs`) | whether "framelock" means a user-selectable mode or what §AL already does: OPEN Q6 |
| Display OFF by default | yes: `display` and `gop` default off | |
| Windows support default on, switchable | yes: `windows-support` | |
| Global options applied to all GPUs | generic QEMU `-global`, **NOT RUN** | section 1.3 |
| Kayfabe enabled by a CLI argument; no GPUs named = all GPUs | **no**: the device exists when named; there is no umbrella and no enumeration | section 1.3, OPEN Q2 |
| No kayfabe argument = plain QEMU | yes by construction in a build that links the device: the only registration is `type_register_static`; nothing runs unless a `-device kf3-gpu` is given | CODE READ; the Rust static archive's own load-time behaviour was not audited |

### 2.3 Gaps, in the order the owner will meet them

⊘ **Correction 2026-11-23 (CODE READ, `next` `92a7f2782`):** item 5 below is built. `scripts/ci/doc_cli_reference.py` generates the property table of `docs/howto/cli-reference.md` from `qemu/hw/misc/kf3/kf3.c` and `python3 scripts/ci/doc_cli_reference.py check` fails when a `DEFINE_PROP` line has no row (run on 2026-11-23: `CLI_REFERENCE OK (27 properties)`; it runs in the `stable` job of `.github/workflows/ci.yml`, with `doc_cli_reference_mutants.py`). The text of item 5, and its count of 26 rows, is the 2026-11-20 state and is kept as written. Items 1 to 4 are unchanged.

1. The umbrella `-object` and its relation to `-global` (Q2).
2. All-GPUs-by-default (Q2).
3. A display mode: resolution, and what "framelock" selects (Q6).
4. A coherent limits family (section 2.4, Q7).
5. A **generated** property table and a test that fails on an undocumented property: today `docs/howto/installing.md` has a hand-written table of 26 rows against 27 `DEFINE_PROP` lines; **`x-gsp-observer-seconds` is the missing one** (counted 2026-11-20). `git grep -l DEFINE_PROP` over `scripts`, `tools`, `crates` and `ci` finds only two Rust tests that read the C file for other reasons (`crates/kf-qemu/tests/bar0_trace_gate.rs`, `crates/kf-qemu/tests/utilization_use_sites.rs`) and a patch file: no gate holds the table to the source.

### 2.4 The limits family: inventory first (CODE READ, `git grep`, 2026-11-20)

`git grep` for constants named `*MAX*`, `*CAP*`, `*LIMIT*`, `*BUDGET*`, `*QUOTA*` or `*BOUND*` in the non-test sources of `kf-qemu`, `kf-chan`, `kf-mem`, `kf-rm`, `kf-host`, `kf-disp`, `kf-gsp` and `kf-broker` finds **191** lines. Most are wire constants or test bounds, so the count is not the number of guest-reachable resources. The existing inventory of guest-steered resources is `V3_SECURITY_MODEL.md` section 7 (input to the plan's section 2.4 "one table of every guest-steered resource, with its cap and the cap's test", which has not been produced). The table below is what the tree enforces today; **no cap in it is user-settable except the ones marked "property"**.

| Resource the guest can steer | Current cap | Where | Settable by the operator |
|---|---|---|---|
| host channels per runlist (the number the guest is told) | derived from the host, 256..2048 | `kf-abi/src/chanbudget.rs`, `kf3.c` | property `channel-budget` |
| user-ring store views kept per channel | 16, max 256 | `kf-qemu/src/userstore.rs` | property `user-view-keep` |
| BAR1 doorbell overlays | 64 | `kf3.c` | property `bar1-overlays` |
| guest VRAM, BAR1, BAR2, MSI-X | 8192 MiB, 256 MiB, 32 MiB, 32 | `kf3.c` | properties |
| doorbell ioeventfds | 256 | `kf3.c` | property `doorbell-ioeventfd-max` |
| live RM handles, parked facts | 2^18 each (`MAX_LIVE_HANDLES`, `MAX_PARKED`) | `kf-rm/src/rmgraph.rs` | no |
| VA spaces per walk | 64 (`MAX_SPACES_PER_WALK`) | `kf-mem/src/vasmgr.rs` | no |
| VA extent of one batch / of all batches of a space | 64 GiB / 512 GiB (`BATCH_MAX_EXTENT`, `BATCH_MAX_TOTAL_EXTENT`) | `kf-mem/src/batch.rs` | no |
| CPU-window placements | 8192 (`MAX_CPU_WINDOW_PLACEMENTS`) | `kf-mem/src/cpuwin.rs` | no |
| display-SW twins | 16 per channel, 1024 per VM (`PER_CHANNEL_CAP`, `PER_VM_CAP`) | `kf-qemu/src/dispsw.rs` | no |
| NVENC sessions | 3 per VM (`ENC_SESSIONS_PER_VM`) | `kf-qemu/src/encquota.rs` | no |
| PRAMIN view retire queue | `RETIRE_QUEUE` | `kf-qemu/src/mem.rs` (`TrapPool`) | no |
| user-ring batch operations | 16384 (`MAX_OPS`) | `kf-chan/src/user_batch.rs` | no |
| T-mode pieces, ring segment | 4096 pieces, 256 KiB | `kf-chan/src/tmode.rs`, `ring.rs` | no |
| display heads, windows, frame, queue, pushbuffer | 8, 32, 3840x2160, 4096, 65536 | `kf-disp` | no |
| GSP large payload, fragments, held messages | 1 MiB, 64, 1024 | `kf-gsp/src/large.rs`, `boot.rs` | no |
| guest-repeatable log lines | rate-limited by `kf_arch::rate_eprintln!` (74 occurrences in `crates/`) | many | no |
| host BAR1 across VMs | this process only | `kf-qemu/src/cardbudget.rs` | no |

Still open in `V3_SECURITY_MODEL.md` section 7 and **not re-verified here**: runlist time (S1-44), host RM and GSP call rate, GPU time, twin-private host VRAM (S1-82), the shared walk budget (S1-61), and log volume on the `kf-qemu`, `kf-mem`, `kf-chan` and `kf-disp` sites. That document's rows about NVENC and the PRAMIN queue still say "unmerged"; the two files above are in `next`, so the document needs its own refresh (Q7).

**DESIGN.** One family of limits, set on the umbrella object (or per device where the resource is per device), every default equal to today's constant, every one printed at realize and refused by name past what the host can give (the pattern `channel-budget` already follows). Which constants become settable is a choice between "a user can hit this in normal use" (channels, views, overlays, NVENC sessions, display twins qualify) and "a hostile guest can hit this and a user never does" (handles, VA batch extents: these stay constants, listed in a generated table so a reader can see them). OPEN Q7 asks the owner which side each row falls on.

### 2.5 The no-environment rule

**OWNER DECISION.** A full-performance Windows or Linux run needs no environment variable. All user settings are on the QEMU command line. Debug and benchmarking environment variables may remain, and are secondary.

**Audit (CODE READ, 2026-11-20).** Method: `git grep` for `env::var`, `env::var_os`, `option_env!`, `env::vars` and `getenv` over `crates/kf-*`, `qemu`, `broker`, `cuda` and `firmware`, test files excluded by name (`*/tests/*`, `*tests.rs`), and a second `git grep` for string literals beginning `KF3_`, `KF_` or `KAYFABE_` (to catch names held in constants), `kf-harness` and `kf-rawclient` excluded as tooling. Result: 167 read sites and **120** distinct names with those prefixes, plus the system names below. A few reads inside `#[cfg(test)]` modules of product files are included. The classification below is by name and by the comment at the read site; it is **not** an exhaustive per-name review, and the lint (below) would pin the authoritative class per name.

| Class | Names (examples; all `KF3_` unless shown) | Effect |
|---|---|---|
| **System** | `DISPLAY`, `WAYLAND_DISPLAY` (`broker/nb_common.c`: choose the session backend), `SUDO_UID` (`broker/nvkvm_broker.c`: whose uid the broker accepts), `LISTEN_FDS`, `LISTEN_PID` (`broker/nvkvm_broker.c`: socket activation) | legitimate; not kayfabe settings |
| **Build and test harness** | `KF_FUZZ_*`, `KF_SOAK_*`, `KF_SIM_*`, `ADV_*`, `KF_MEM_RSS_TEST_CHILD`, `KF3_FAILCLOSED_CHILD`, `KF_GOP_IMAGE_*`, `KF_POST_TRACE_FIXTURE`, `KF_WALK_SERIAL`, `KF_TESTS_DUMP_RUNS`, `KAYFABE_NO_KVM` (lets `cargo test` run without KVM), `CARGO*`, `OUT_DIR` | not read by a running device |
| **Debug and measurement** | `KF3_RPC_TRACE`, `KF3_RPC_BODY_TRACE`, `KF3_BAR0_TRACE`, `KF3_BAR0_READ_TRACE`, `KF3_MAPLOG`, `KF3_PROF`, `KF3_DISPLAY_TRACE`, `KF3_DISPLAY_METHOD_TRACE`, `KF3_DISPLAY_WRITE_TRACE`, `KF3_USER_RING_TRACE`, `KF3_DIAG_*`, `KF3_DEBUG_CURSOR`, `KF3_DOORBELL_CPL` (the only read in `kf3.c`), `KF3_TCENSUS`, `KF3_TSHADOW`, `KF3_DBFAST_PROBE`, `KF3_COMPLETION_PROBE`, `KF_IOCTL_TRACE`, `KF_VAS_CENSUS`, `KF3_WORKER_PARK_INFINITE`, `KF3_PT_STALL_SNAPSHOT*`, `KAYFABE_TRAP_*`, `KAYFABE_VCPU_BLOCK_FATAL`, `KAYFABE_C_TRACE_CAP1*` | observe or abort; do not change what the guest is served (by name and comment) |
| **Negative controls and fault injection** | `KF3_NEGCTL_*` (9 names), `KF3_INJECT_STALE_USERD`, `KF3_INJECT_FWSEC_FAIL`, `KF_LIFECYCLE_MUTANT`, `KF3_INVALIDATE_CLEAR_OVER_ABSENT`, `KF3_DEBUG_FORCE_MIGRATABLE` (honoured only together with `KF3_DEBUG=1`, `chanlink.rs`) | deliberately break something for a test |
| **Behaviour switches with a default (product-affecting)** | default **on**, the variable turns it off: `KF3_NO_BATCHED_MAP` (batched map), `KF3_WIN_USER_RING_RELAY=0`, `KF3_PUT_CACHE=0`, `KF3_LAZY_OBSERVE=0`, `KF3_PT_NSI_RELAY=0`, `KF3_PT_NSI_MIN_INTERVAL_US`, `KF3_LIFECYCLE_MODE` | an operator can change what the guest is served from the environment |
| **Behaviour experiments, default off (product-affecting)** | `KF3_DISPLAY_SDR_COLOR`, `_ARMED_DEFAULTS`, `_BLANK_STATE`, `_HDCP_STATE`, `_CORE_AT_VBLANK`, `_LOADV`, `_HOTPLUG_EDID_SEEN`, `_HEAD_TIMING_EN_BASE`, `_CAPS_OVERLAY_*`, `KF3_TEAR_GATE`, `KF3_SW_SUBCH_INERT`, `KF3_USERD_RELAY_OFF`, `KF3_RELAY_GET_REFRESH`, `KF3_WIN_USER_RING_SHADOW`, `KF3_WIN_REFUSE_PT_FIFO_UPDATE`, `KF3_WIN_HIDE_UNTABLED_VIDEO`, `KF3_ASYNC_PREEMPT_REENABLE_NOACT`, `KF3_CARRY_ATOMIC_DISABLE`, `KF3_DBFAST_SPIN_US`, `KF3_NO_GUEST_VA_RESERVE`, `KF3_VRAM_ATTRS`, `KF3_DISPLAY_*_PROBE` | each is an experiment or a falsifier arm; not needed to boot as far as the inventory says (`V3_FLAG_INVENTORY.md`, which is itself partly stale: 25 of its 137 names no longer appear in the tree) |

What this means for the rule. The rule is satisfied **in practice** when no name in the last two rows is needed for a normal run; the 2026-11-15 flag cleanup removed the 18 flags of the Windows profile on that basis (`OWNER_RULINGS.md` §AT), but that change was not run on hardware (`V3_FLAG_INVENTORY.md`). It is satisfied **by construction** only when the product-affecting reads cannot be reached from a release binary's environment at all. DESIGN: each name in the last two rows gets one of three fates, decided per name: hardwire the shipped value and delete the read; turn it into a documented `x-`-prefixed device property (QEMU's own marker for an unstable property) if an operator really needs it; or compile it only into a lab build.

⊘ **Correction 2026-11-23 (CODE READ, `next` `92a7f2782`):** the lint below exists. `scripts/ci/env_allowlist.py` with `scripts/ci/env_registry.json` and the debt file `scripts/ci/env-allowlist-debt.json` runs in the `stable` job (run on 2026-11-23: `reads=127 names registered=127 offenders-on-baseline=51 findings=0`), and `env_allowlist_mutants.py` runs in CI. Its text and the sentence "What exists today: no such gate" are the 2026-11-20 state and are kept as written; the fates of the 51 baseline offenders are still to be decided per name.

**Allowlist lint (DESIGN).** A CI gate that (a) lists every environment read in the product sources (the same `git grep` as above, plus string literals with the three prefixes); (b) holds the list to a committed allowlist file that assigns each name one of the six classes above and, for the two product-affecting classes, a fate and a date; (c) fails on a new name, on a changed class, and on a product-affecting read in a source path the release binary links; (d) is itself under a mutant test, like the other `scripts/ci` gates. **What exists today:** no such gate. `git ls-files scripts/ci | grep -i env` finds `provider_env_mutants.py`, `test_vast_provider_env.py` and `test_vast_setup_base_env.py`, which test the Vast VM's own environment, not kayfabe's. The lint is named as "later (Stage 3)" in `OWNER_RULINGS.md` §AT and as "not done" in `V3_FLAG_INVENTORY.md`, and `V3_CLOCK_RULE.md` says the repository has none. Related and existing: `scripts/bench/windows/win_vm.sh --env-file` accepts only `KF3_NAME=digits` lines (`docs/howto/windows-guest.md`), and `scripts/ci/doc_freshness.py` treats any `KF3_`, `KF_`, `KAYFABE_` or `NVKVM_` name in a document as a reference that must appear in source.

### 2.6 The display broker (the non-headless case)

**OWNER ADDITION (2026-11-20).** The manual must tell how to start and run the broker, for the non-headless case only, as simply as the QEMU side and with **no environment dependency**. A headless VM needs no broker (`OWNER_RULINGS.md` §AS item 7). The user-facing page is a later job (section 11); this section is its input.

**Today (CODE READ at `origin/next` `7fb1c98a0`, unless marked; nothing here was run by this change).**

- **What it is.** A separate C program in `broker/`, built with `cd broker && make`; the binary is `nvkvm-display-broker` (`BIN` in `broker/Makefile`), next to the test tools `nvkvm-broker-testclient`, `nvkvm-broker-relay` and `nvkvm-broker-dmabuf-src`. The Makefile keeps only the backends whose libraries are present and prints which. Backends: `wayland`, `x11`, and `test` (not reachable from `auto`, by design, `broker/nb_common.c`). Build dependencies are listed in the Makefile header (Wayland: `libwayland-dev wayland-protocols`; X11: `libxcb1-dev libxcb-dri3-dev libxcb-present-dev`, with optional `libxcb-render0-dev` and `libxcb-xinput-dev`). No release-tarball packaging of it exists (Q11).
- **Flags** (the program's usage text): `--socket PATH`, `--socket-mode OCTAL` (default 0600), `--socket-group NAME`, `--socket-fd N`, `--no-peercred`, `--backend auto|wayland|x11|test`, `--size WxH` (default 1920x1080), `--title TEXT`, `--allow-user NAME`, `--allow-group NAME`, `--drop-user WHO`, `--fullscreen`, `--scale aspect|stretch|none`, `--persist`, `--clipboard off|guest-to-host|consent`, `--clipboard-trigger LIST`, `--refresh auto|none|HZ`, `--resolution auto|none|WxH`, `--present-mode`, `--trace-frames`, `--linear-only`, `--stats-line`. The broker serves one VMM connection and exits with it unless `--persist` is given. The broker does not create the socket's directory (`git grep mkdir broker/nvkvm_broker.c` finds nothing), so `/run/user/<uid>/` works and a new subdirectory under it does not unless the user makes it.
- **Environment reads in `broker/`** (`git grep` for `getenv`, 2026-11-20): `WAYLAND_DISPLAY` and `DISPLAY` (`nb_common.c:392-393`, choosing the backend under `auto`; also quoted in the error messages of `nb_session_wl.c` and `nb_session_x11.c`); `SUDO_UID` (`nvkvm_broker.c`, near line 3929: when the broker runs as root it adds that uid to the allowed peers); `LISTEN_FDS` and `LISTEN_PID` (`nvkvm_broker.c`: systemd socket activation; `LISTEN_FDS` also decides whether a missing `--socket` is an error); `NB_TEST_NO_TICK` (the `test` backend only). The environment dependency is larger than the `getenv` lines: the Wayland backend connects with `wl_display_connect(NULL)` (`nb_session_wl.c:5478`) and the X11 backend with `xcb_connect(NULL, ...)` (`nb_session_x11.c:1975`), and by those libraries' documented behaviour (not read in this tree) `NULL` means "read `WAYLAND_DISPLAY` and `XDG_RUNTIME_DIR`" and "read `DISPLAY`". These are the desktop session's own variables, present in any terminal of that session; the dependency is real only for a broker started from outside the session (a service, `sudo`, ssh).
- **QEMU side.** The socket is the device property `display-broker=<absolute path>` (unset = no broker). `display-broker-uid=<uid>` names one more uid accepted as the broker, needed only when the broker's uid differs from QEMU's: uid 0 and QEMU's effective uid at each connect are always accepted, the owner of the socket's directory is not (`kf3.c`, `crates/kf-broker/src/link.rs`). Symmetrically the broker accepts, by default, uid 0 and its own uid (or `SUDO_UID` when started as root through sudo), plus `--allow-user` and `--allow-group` (`nvkvm_broker.c`). `display=on` is opt-in (default off). `gop=on` is the boot display; the harness launchers always set both (`scripts/bench/display/interactive.sh`, `scripts/bench/diesweep/win_die.sh`). `gop-efi=<signed efi>` is needed for a Secure Boot guest (`kf3.c`: the boot display driver "signed", refused with `gop=off`; `docs/howto/windows-guest.md` describes the signing). `display-max-fps` is 24..75 and needs `display=on`.

**Documentation defects found (CODE READ, 2026-11-20).**

- `broker/README.md` is the imported nvkvm-pv README. It shows QEMU options that do not exist here (`-display nvkvm-broker,socket=...`) and advises `-display sdl` and `-display gtk` for an unconfined QEMU (its "Do you need this?" section and the example under "Run"); the kayfabe QEMU is built without SDL and GTK (`scripts/bench/build_kf3.sh` passes `--disable-gtk --disable-sdl`). It also tells the reader the broker is only for sandboxed VMMs, which is not the kayfabe reason for it (`OWNER_RULINGS.md` §AS item 7: the broker is the display piece, the replacement for Looking Glass).
- `docs/howto/display.md` is a manual page that mixes in design intent and history: "the owner's direction of 2026-10-01", "the design makes the broker the frame clock (not built yet)", a long note correcting `broker/README.md`, evidence tags, and the author's launcher `scripts/bench/display/interactive.sh` whose `prep` builds the broker from a pinned revision of the nvkvm-pv repository (`KF_NVPV_REV`) instead of from `broker/`. A reader following the page can end up running a different broker than the one in the tree.
- The two statements about the frame clock disagree: `display.md` says the broker as frame clock is "not built yet", while `broker/README.md` documents `CAP_TICK` and `EV_TICK` (the broker sends a tick per display refresh and the VMM may reply for lock-step frame requests) and `OWNER_RULINGS.md` §AL describes kayfabe's lock-step enforcement as live. What is built and what is only designed is Q12.

**TARGET (DESIGN-ONLY).**

1. **No environment needed.** Explicit flags for everything the broker reads from the environment, with the environment kept only as a documented fallback: a Wayland display name (working name `--wayland-display NAME`, an absolute path allowed so `XDG_RUNTIME_DIR` is not needed either), an X11 display (`--x11-display :N`), and an extra allowed uid (`--allow-uid UID`, replacing the use of `SUDO_UID`). Socket activation stays, because it is how a service manager hands over a socket. The names are working names; none exists today.
2. **One broker socket per GPU.** Display is per GPU, so each `kf3-gpu` names its own socket. The umbrella object (section 1.3) provides a **default path template with a `{gpu}` placeholder** and a per-GPU override (`display-broker=` stays the per-device property). How several heads of one GPU (up to 8 in `kf-disp`) map to broker windows was not read here (Q11b).
3. **A two-command quickstart.** Working binary name `kayfabe-broker` (the binary is `nvkvm-display-broker` today, Q11):
   ```
   kayfabe-broker --socket /run/user/$UID/kayfabe/gpu0.sock
   qemu-system-x86_64 ... -device kf3-gpu,...,display=on,display-broker=/run/user/$UID/kayfabe/gpu0.sock
   ```
   with the broker creating the socket's directory (mode 0700) so the example needs no third command, and `display-broker-uid` shown only for the case where the two uids differ. A user whose broker and QEMU run as the same user never meets a uid option.
4. **The page** (section 11) tells exactly this, names the build line (`cd broker && make`), says in one sentence that headless VMs skip it, and moves the sandbox rationale, the history and the frame-clock design out to internal documents.

## 3. The guest driver

**OWNER DECISION.** The guest driver version is **automatic by default**: the guest chooses it. A command-line pin stays available for security. **The invariant: the guest never dictates the host ABI.** Host commands are authored by kayfabe from the **host** driver's tables; a guest request whose layout the host's tables cannot carry is refused with a named reason.

### 3.1 Today (CODE READ unless marked)

- `guest-driver=` unset: the device defaults to the host's own driver version (`crates/kf-qemu/src/device.rs`, "defaulted to the host's"), then **re-selects at fn 1** from the version string the guest's driver sends, **only when** the two versions' pre-fn-1 surface is identical (`kf_abi::versions::pre_fn1_surface_differs`; `V3_DRIVER_MATRIX.md` section 4.2). A pair whose surface differs (that document names 580.x against 595.84, and 595.84 against 610) is **refused by name**. So "automatic" holds inside a branch and is **not** yet true across branches: a 580 guest on a 595 host needs the pin today. That is a gap against this decision (Q3).
- A **declared** version is never overridden, and the guest's own fn-1 string is cross-checked against it: a mismatch is a named refusal, never a guest answered with another release's layouts.
- Host side: `kf_abi::hostabi` carries every host-side struct from the one internal layout to the host's measured layout by field name. Outcomes per struct are `Carry::Same`, `Carry::Carried`, or an error: `Unmeasured` (host tag not measured), `Layout`, `Unexpressible` (the request sets a field the host's version lacks), `Transcode`. `kf-host` turns any of them into `HOST_ABI_REFUSED` and prints `kf-host: HOST-ABI REFUSED <what>: <reason>`, rate-limited (`crates/kf-host/src/lib.rs`). The measured-tag list is `tools/drivermatrix/tags.txt`: **30 tags** on `next` (535.309.01 to 615.71.09; the older `V3_DRIVER_MATRIX.md` text says 29).
- MEASURED (PR #522, merged 2026-11-19; hardware result by the coordinator on host 595.91.07, AD104, with a 595.91.07 Rocky 9.7 guest, raw logs not committed): the mechanism ran on a real 595.91.07 host; **7** `HOST-ABI REFUSED` lines came at VM start-up from kayfabe's own questions to rows that do not exist at that host version, all harmless, none a guest request, and no refusal of a 595-sized row after the fix. The owner's discussion also cites the same mechanism on a 580.159.04 host; this document has no record of that run beyond the owner's statement. #522 is a host-ABI coverage change and says in its title that it is not a CUDA fix.
- **Not measured:** a guest whose control layout differs from the host's, refused by this mechanism end to end. The code path (`host_control_carry`) was read, not exercised with a mismatched pair on hardware.

### 3.2 Design: a GPU-free Dg x Dh conformance matrix

**DESIGN.** `Dg` is the set of guest driver tags kayfabe serves, `Dh` the set of host tags it can ask. For every pair and every control (and RPC) kayfabe forwards, a generated table records one of:

1. **total translation**: every field the guest's layout needs is produced from fields of the host's layout, by name, with the sizes and offsets from the ogkm-derived tables;
2. **defined default**: a field the host lacks (or the guest lacks) is given a stated default, and the default is listed;
3. **named refusal**: the exact reason, as the `HostAbiError` text.

It is a **static proof of layout, size and field handling**, not of behaviour. It runs without a GPU, over the layouts that `crates/kf-abi` already generates from ogkm tags (`kf-abi/tests/driver_matrix.rs` and `fn1_negotiation_oracle.rs` are the nearest tests today). The compatibility table that falls out is **generated into the documentation**, so a user can see which guest driver works on which host driver without reading code. Sources of tags: `third_party/ogkm-580` (580.159.04) and `third_party/ogkm` (610.43.02) as submodules, 595.84 through `scripts/ci/fetch_oracles.sh` (pinned commit), and 15 more tags in the draft PR #351. Tags present in the matrix but with no oracle checkout on the CI runner are limited by what `fetch_oracles.sh` fetches: whether the matrix runs on all 30 tags in CI or only on the checked-out ones is Q3b. A pair outside the measured set stays refused by name (exact membership, no nearest neighbour, as `hostabi.rs` already rules).

## 4. Guests

### 4.1 Windows

**OWNER DECISION.** A Windows guest needs only the documented guest settings, delivered by **one tested user script that can run before the NVIDIA driver is installed**, and that script is tested in CI on a Windows runner. Today's documented settings are the rows of the table in `docs/howto/windows-guest.md`: `EnableGpuFirmware = 1` in the display adapter's class key, `EnableGpuFirmware = 1` in the `nvlddmkm` service key, and `Dwm` `OverlayTestMode = 5`. **That is three rows; the owner said four. OPEN Q4: which is the fourth?**

The script itself is being written by another agent as a two-phase script (before and after the driver install); it is not duplicated here. See `docs/howto/windows-guest.md` for the settings and the pull request that agent opens for the script (number not known when this was written). Not decided: whether the CI Windows runner is a GitHub-hosted runner (no GPU) or a Vast box.

### 4.2 Linux

**OWNER DECISION.** Any **unmodified** VFIO guest image boots with only the device swapped. **DESIGN:** an equivalence test that boots the same image on the `vfio` arm and on the `kf3` arm and compares the outcome; PR #527 adds the `arm` lane parameter (open when this was written, so it is not in `next`). The guest-driver rule of section 3 is what lets the image carry whatever driver it already has.

## 5. Bench scripts

**OWNER DECISION.** `scripts/bench/*` has exactly two purposes: **CI-test helpers** (heavy coupling to the box is intended, for determinism) and **optional guest-setup helpers**. It is never a requirement of the user manual. This sharpens `OWNER_RULINGS.md` §AS item 5. Today the manual breaks it: `docs/howto/installing.md` (13 distinct `scripts/bench/...` names), `attach-to-existing-vm.md` (11) and `windows-guest.md` (9) all send the reader to bench scripts, and `installing.md` states "There is no installer" and "run as root". The target page order is section 0's four steps, none of which names a bench script. A script that a user may still want (for example guest disk setup) moves out of `scripts/bench` or is marked optional in the page that names it.

## 6. Rootless

**OWNER DECISION.** The rootless run is **the test that proves no root is needed for the VM run**: any NVIDIA ioctl that needs root would fail it. `kfroot` is an explicit per-command approval and each one is audited ("are you sure users do not have to run this to get that app working rootlessly?"). Production has no `kfroot`. The job is to **reduce** `kfroot` use.

State on 2026-11-20 (CODE READ): `V3_KFROOT_ROOTLESS.md` says LIVE as code 2026-11-15, **NOT RUN ON A BOX**, and its section 8 says the payload (`vast_payload.py`, `vast_onstart.sh`) still starts every stage as root. `git grep -n kfroot` outside docs, archive and traces finds the word on 16 lines of `scripts/bench/box/windows_gate.sh`, 7 of `scripts/bench/windows/win_vm.sh`, 3 of `windows_apps.sh` and 1 of `windows_lane.sh` (27 lines in four Windows stage scripts), and in the mechanism, tests and CI wiring (`scripts/ci/kfroot.py`, `kfroot_mutants.py`, `test_kfroot.py`, `scripts/lib/kfroot.sh`, `ci.yml`). The owner's figure was about 26 lines in 7 files; the count above is what the grep gives. **An audit agent is on this; this document does not repeat it.** Its result is OPEN Q5.

The user-facing claim that no root is needed is already in the manual: `docs/howto/security.md` records a throwaway user whose only group was `kvm` running the 30-arm suite 30 of 30, the GPU gates 9 of 9 and a Windows guest at 92.55 fps against 93.09 as root (kayfabe `0a746117`, RTX 4070, host 595.91.07, one host; its record is on a branch, not on `master`, and the page states its qualifiers). That is the evidence the "needs only KVM and NVIDIA access" sentence of section 0 rests on.

## 7. Licences

**OWNER DECISION.**

- **Microsoft.** The Windows LTSC image the open lanes download is Microsoft's evaluation. A run that downloads it **prompts the user to accept Microsoft's terms** (with the link). The acceptance is recorded in the run log. A non-interactive accept flag exists for unattended runs. No acceptance, no download.
- **NVIDIA.** The user is assumed to have accepted NVIDIA's terms by having the driver. A run prints a **one-line notice with the link** and does not prompt. Kayfabe is not affiliated with NVIDIA and redistributes no NVIDIA binary.

Today (CODE READ): `docs/DOWNLOADS.md` is a draft research summary of the terms status of each download (not final, not legal advice). The unattended Windows answer files accept the Windows EULA on the installer's behalf (`<AcceptEula>true</AcceptEula>` in `scripts/bench/windows/autounattend.xml.in` and `propimage_autounattend.xml.in`); no prompt in the runner and no acceptance record in the run log was found (a `git grep` for terms-acceptance wording in `scripts` and `ci` finds only those files, the compliance checks of the app matrix and recipe data). `OWNER_RULINGS.md` §BA item 4 already forbids licence or expiry data in code, `state.json` or any gate; the prompt records acceptance of a download's terms, not a licence's end date, so it does not conflict, but the log line must not carry a product end date.

## 8. Trust and the public object store

**OWNER DECISION.** The default of a standalone run is **trust git only**. The non-trust-git mode is **dropped for third-party standalone** (it needs one's own Worker and store). This refines `OWNER_RULINGS.md` §AX: the explicit lift of §AX item 3 remains available to the operator who runs their own Worker, and is not part of what a third party is offered.

### 8.1 Anchors that count

1. The **official-origin checksum** (or signature) fetched from the upstream origin.
2. A **GitHub attestation produced in the same workflow run that builds, attests and uploads the object.** Builder equals uploader is fine: the attestation binds the bytes to the repository, the commit and the workflow. Cross-machine bit reproducibility is **not** required for it. The Worker already verifies GitHub OIDC (`ci/vast-broker/src/oidc.js`, used by the check and sweep routes with a fixed subject and workflow refs in `wrangler.toml`), so a CI worker may publish through it; extending the accepted subject and workflow refs to a publishing workflow is part of the open design (Q8). **A verifier of this anchor (the manual's `gh attestation verify`, a future Worker check or a `kf_trust.py` attestation class) must pin all three of: the signer workflow path (`reindertpelsma/kayfabe/.github/workflows/<release workflow>.yml`), the source ref (`refs/heads/master`) and a GitHub-hosted runner (`--deny-self-hosted-runners`).** The repository alone is not an identity: any workflow on any branch of it can attest arbitrary bytes. `docs/howto/building.md` and `release-qemu.yml` carry the three pins today.
3. A **local rebuild**: the user uses their own build. No comparison with the stored object is required.

A sha256 pinned in git for an opaque blob is a **cache key, not trust** (`OWNER_RULINGS.md` §AX item 2 already forbids it as an anchor, class `git-sha256-of-our-own-artifact`, enforced by `scripts/ci/trust_lint.py`). The attestation anchor is **not built**: `scripts/ci/kf_trust.py` `verify_attestation` refuses by name (`TRUST_ATTEST_NOT_IMPLEMENTED`).

### 8.2 Three kinds of public object

| Kind | What | How it is checked | Who may use it |
|---|---|---|---|
| **A** | exact mirror of an upstream file | the upstream official hash or attestation | anyone |
| **B** | our own build, for example `winapps-open.iso` | the attestation of the workflow run that built and uploaded it (8.1, anchor 2), or a local rebuild | anyone |
| **C** | a coordinator-built bundle | none a third party can check | our own workers only (git pin, deployment data) |

Today (CODE READ): `.github/workflows/bundle-publish.yml` ("Public Windows bundle", manual, publication disabled) builds twice and compares inventories, then uploads with R2 credentials held in the workflow's secrets; it does **not** attest and does **not** go through the Worker. `release-build.yml` is the attested workflow, for Rust binaries only. Reading of the two together (INFERRED): a kind-B object needs the attest-and-upload-through-the-Worker shape of `release-build.yml` applied to the bundle build. This is consistent with `OWNER_RULINGS.md` §BA item 2 (no input may have our store as its **only** source: a B object always has a local-rebuild alternative from the repository's recipe).

### 8.3 What a run prints, and the index

A download prints one short line: **what** it is, **where** it comes from, the **verification anchor**, and that kayfabe validates it. Nothing longer in the default output.

**DESIGN.** A machine-readable `INDEX.json` at the bucket root, **generated from the repository** (the trust manifest and the publication list), explains every object and folder: kind, source, anchor, size, which stage uses it. `docs/DOWNLOADS.md` is generated from the same index. Today `docs/DOWNLOADS.md` exists as a **hand-written** document with `STATUS: RESEARCH, 2026-10-31` (the Worker's `bandwidth.json` cites its tables); no `INDEX.json` exists (`git grep` finds none). Generating the one from the other replaces that document, so its hand-written parts (the SPECviewperf handling rules) need a new home: Q8b.

## 9. Reproducing the Worker

**OWNER DECISION.** The Worker (`ci/vast-broker`) must be reproducible by someone else. **DESIGN:** a "run your own CI" section, separate from the user manual, holding: a from-scratch deploy guide; an inventory of secret names and bindings (names only, never values); one script that creates the buckets and bindings; and an example images configuration. Our concrete object keys stay with the operator (`OWNER_RULINGS.md` §BA item 1).

Today (CODE READ): `ci/vast-broker/wrangler.toml` declares the Durable Object bindings `LEDGER` and `HOSTS`, the D1 binding `DB` (database `kayfabe-platform`, migrations in `ci/platform-api/migrations`) and the R2 bindings `EVIDENCE_BUCKET` and `LOGS_BUCKET`; the secret names it lists are `VAST_API_KEY`, `RUN_TOKEN_SECRET`, `OWNER_STOP_TOKEN`, optionally `BROKER_PATH_PREFIX`, `R2_READ_ACCESS_KEY_ID`, `R2_READ_SECRET_ACCESS_KEY`, `BOX_ACCESS_ID`, `BOX_ACCESS_SECRET`. `ci/vast-broker/DEPLOY.md` is "Owner deployment and independent cleanup" and records what is and is not deployed; it is not a from-scratch guide. The wrangler file also carries the project's own identifiers (public account and repository ids, the OIDC subject, hostnames), which a second operator replaces. No script creates the buckets; wrangler creates a missing R2 bucket during a plain deploy, as the file's own comment says.

## 10. Testing from the repository alone

**OWNER DECISION.** Everything is reproducible and auditable from the repository; no private repository is ever a dependency. Anyone can run: the GPU-free Rust tests; GPU tests on open Linux images; open-source applications (Blender, OBS) on them; the Windows open probes after the Microsoft prompt (section 7). Beyond that:

- **Choosing what to test.** An interactive chooser as an **interactive mode of the existing standalone runner**, because `OWNER_RULINGS.md` §AS says no new CLI. The existing runner is `scripts/ci/vast_payload.py --full` with its instruction file (`docs/howto/full-standalone-run.md`, `scripts/ci/standalone_full.py`). Not built.
- **Unreal:** point the runner at a folder holding the source or a built binary.
- **SPECviewperf:** the user points at a zip of their installed Program Files; the exact zip layout and how to produce it are documented, with a validator and **no pins** (the user brings their own).
- **Proprietary images are bring-your-own:** a manifest or config naming the path to the image and the app list. PR #526 (schema, validator, resolver), #527 (generic planner and the `arm` parameter) and #528 (Worker planner) are that schema and resolver; all three were open when this was written. No image letters A to G are part of the model (`OWNER_RULINGS.md` §BA item 1).
- **A VFIO reference target:** every script that can also run against a VFIO target does (arm `vfio`).
- **The full release gate** runs standalone on Vast, on cheap untrusted nodes.

## 11. Documentation

**OWNER DECISION.** User docs (install, run, troubleshoot, report) are separate from internal docs; AI-only notes stay out of code comments and live in separate internal files; the repository is cleaned so anyone can read it. `AGENTS.md` (the file the agents read) does not need to change for this.

Facts recorded 2026-11-20 (counted in the tree):

- 478 Markdown files under `docs/`. `docs/README.md` already defines the split: published = `docs/howto/` plus `docs/FAQ.md`, everything else internal.
- `docs/howto/` holds 34 files (including `SUMMARY.md`; the discussion said 35). Eleven are operator or CI pages by name (`vast-budget-settings`, `vast-check-selection`, `vast-provider-faults`, `vast-provider-retry`, `vast-rerun-auth`, `kfci-plan`, `gpu-vast-evidence`, `gpu-vast-windows`, `gpu-gate-reuse`, `ci-downloads`, `site-status`); with the image-building and standalone-run pages the owner's count of about 15 is plausible.
- Eighteen of the 34 pages contain internal evidence tags (`CODE READ`, `MEASURED`, "not yet run"); `recreate-the-proprietary-image.md` alone has 62 matching lines.
- `docs/howto/installing.md` opens with "There is no installer" and describes root bench scripts: it contradicts the target story of section 0.
- The freshness gate counts ogkm and nouveau source file names in LIVE documents as dead repository paths; the owner's estimate is that it will fail 53 documents by 2027-01-15 (backlog, not re-derived here). Open PRs #531 and #532 clear 21 documents; this change touches none of them.

**DESIGN.**

1. The manual is rewritten around the four steps of section 0, with operator pages moved to a "run your own CI" section (also section 9) and evidence tags moved to the internal documents that carry the evidence. That rewrite is a separate, later job; this change does not touch `docs/howto/`.
⊘ *2026-11-23: items 2 and 3 are built and run in CI: the property table (`scripts/ci/doc_cli_reference.py`), the environment allowlist (`scripts/ci/env_allowlist.py`) and the comment-hygiene gate (`scripts/ci/comment_hygiene.py`, baseline file `scripts/ci/comment-hygiene-debt.json`, run on 2026-11-23: `baseline=2070 new=0 stale=0`). The limits table of item 2 is not built.*

2. The property table is **generated from `kf3.c`**, and a test fails when a `DEFINE_PROP` line has no row (section 2.3 item 5). The environment allowlist (section 2.5) and the limits table (section 2.4) are generated the same way.
3. A gate for **date-stamped or agent-process notes in code comments** (for example the `★ 2026-10-03` and `⊘ [date, box id]` markers that fill `kf3.c`, `build_kf3.sh` and many Rust files). The marker style is the repository's own history, so the gate needs a baseline of existing debt and fails only on new lines, the way `scripts/ci/debt.py` ratchets other gates. Design notes and incident history move to internal documents, which already exist for most of them.

## 12. Order of work

⊘ **Correction 2026-11-23:** steps 1 and 2 of the list below are done (see the corrections in sections 2.3, 2.5 and 11); the build kit of step 5 exists since 2026-11-21 (the UPDATE in section 1.2), the module study does not; the rest stands.

**DESIGN, a suggestion, not a ruling.** Everything below is GPU-free except where it says so.

1. Generate the property table and add the undocumented-property test (smallest, visible, proves the generator approach).
2. The environment inventory committed as an allowlist, then the lint, with mutants.
3. The Dg x Dh matrix and its generated table.
4. The umbrella object, the global defaults and the all-GPUs rule, after Q2 and Q6.
5. The module feasibility study (Q1), then the kit.
   The broker flags and its README cleanup (section 2.6) are independent of it and can go first.
6. Trust: the attest-and-publish workflow for kind-B objects, `INDEX.json`, the generated `docs/DOWNLOADS.md`.
7. The Worker reproducibility section.
8. The manual rewrite, last, once the commands it describes exist.

## 13. What this document does not change

- No code, no CI gate and no published page. The only other files touched are `docs/OWNER_RULINGS.md` (§BC) and `docs/STATUS_AND_HANDOFF.md`.
- ⊘ *2026-11-23: the GPL-exception half of this bullet is outdated, see the correction at the top of section 1.2; the security-audit half stands.* It does not lift the GPL exception or the security-audit gate (section 1.2). A release binary stays blocked on both until the owner rules otherwise.
- It does not duplicate the Windows script (section 4.1), the kfroot audit (section 6) or the propimage configuration work (section 10): each is in flight elsewhere and is referenced.

## 14. Falsifiers

A design claim here is wrong if:

- the module study shows that a `hw/misc` device cannot be built as a module without a QEMU patch larger than today's two hunks (then section 1's module path needs the larger patch and the decision of §AS item 3 is reopened);
- `-global kf3-gpu.<property>` does not apply to a kayfabe device on 10.2.4 (then global defaults need the umbrella);
- a name in the last two rows of the section 2.5 table turns out to be needed by a normal Windows or Linux run (then it becomes a property or is hardwired on, and the rule is not met until it does);
- the Dg x Dh generator finds a pair whose cell is neither a total translation, a defined default nor a named refusal (that is a defect in the host-ABI carry, and it is the matrix's reason to exist);
- a graphical user following only the quickstart of section 2.6 cannot get a window (for example because a broker started outside the desktop session still needs a variable no flag covers);
- a user following only the four steps of section 0 cannot start a VM (the manual rewrite is not done until a person who has not read internal documents has done it).

## 15. Consistency notes against earlier rulings

- `OWNER_RULINGS.md` §AS item 6 ("a tested libvirt XML snippet and Proxmox `args:` example") is still unchecked; the umbrella design should be read together with it.
- §AX item 3 (the explicit trust lift) is kept for the operator and dropped for a third party's standalone run.
- §AW and the memory note "no sha pins for standalone" are superseded in the way §AX already records: default standalone verifies against anchors committed in git.
- §BA item 2 ("no input may have our object store as its only source") holds for kind-B objects because of the local-rebuild alternative (section 8.2); it does **not** hold today for kind-C objects, which §BA item 2 already lists as a known gap.

## 16. Open questions

**Q1. Can the device be a loadable module?** Study QEMU 10.2.4's module machinery for `hw/misc`: what meson change, how much larger than the two hunks of today, and what exact-build match the loader enforces. Decides section 1.3 and the form of the kit.

**Q2. The umbrella object.** The working name `kayfabe-gpus`; whether the name should instead describe the VM-wide settings; whether it only holds defaults and limits (referenced by id) or also creates the devices ("no GPU named = all GPUs"); whether `-global` alone covers global defaults. Q2b: does the owner's "same" UUID mode mean the host GPU's UUID (`host`) or one UUID shared across a VM's GPUs?

**Q3. Automatic guest driver across branches.** Today a pair whose pre-fn-1 surface differs is refused and needs the pin. Is the target to remove that (re-select the whole chain per pair, which is real work: nine-field init args, MCTP elements) or to keep the pin as the answer and document it? Q3b: how many tags can the matrix cover in CI, given which oracle trees the runner fetches?

**Q4. The fourth Windows setting.** `windows-guest.md` has three rows. Which is the fourth: Secure Boot or TPM handling, BitLocker prevention, the boot display driver (`gop-efi`), a service or Defender setting, or another registry value?

**Q5. Rootless audit result.** Which of the 27 `kfroot` lines are removable, which are box setup, which are production-irrelevant. Agent a811eb9c is on it; this document takes its result when it exists.

**Q6. Display mode.** What are the user's resolution and framerate knobs on the QEMU side (a property, an `-object` field, or only the broker's flags)? What does "framelock with the broker" select beyond the lock-step of §AL that is already always on?

**Q7. The limits family.** Which rows of section 2.4 become settable, where they live (device or umbrella), what the property names are; and a refresh of `V3_SECURITY_MODEL.md` section 7 so the table is verified, not inherited.

**Q8. The attested-bundle workflow.** Which workflow builds, attests and uploads a kind-B object; how the Worker's accepted OIDC subject and workflow refs extend to it (today a fixed subject and two fixed workflow refs); where the object lands (the public bucket) and what in git states the expected attestation identity. Q8b: where the hand-written terms and handling rules of `docs/DOWNLOADS.md` go once the file is generated.

**Q9. Proxmox and other trees.** Which QEMU versions the kit supports; what to do about a distribution's patch set that touches `hw/misc/meson.build` or `Kconfig`.

**Q11. Broker packaging and names.** The binary name (`nvkvm-display-broker` today, `kayfabe-broker` in the quickstart), whether it ships in the release tarball and how it is built there (it needs different libraries from the QEMU build), and whether the broker's remaining nvkvm-pv naming (the `nvkvm_*` source files, the title default `nvkvm`) is renamed. Q11b: how multiple heads of one GPU map to broker windows or sockets.

**Q12. Frame clock and framelock.** What is built of "the broker as the frame clock" (section 2.6, the `display.md` versus `broker/README.md` disagreement), and what "framelock mode" would add for the user. Together with Q6.

**Q13. The resolution knob.** The broker's `--resolution` is only a suggestion to the guest and the device has no mode property (Q6): whether the user's knob lives on the broker, on the device or on both.

⊘ **Correction 2026-11-23:** the GPL half of Q10 is answered in fact by task I7 (2026-11-20, `OWNER_RULINGS.md` §BD, see the correction in section 1.2); the stage-2 security gate (§P) is still open, and the decision to publish is the owner's.

**Q10. When may a binary be published?** The owner's goal is a release binary; the GPL exception (`kf-abi` capability tables, re-derivation task I7 of `V3_SWEEP_AND_INSTALL.md`) and the stage-2 security gate (§P) are both still open. Neither is decided here.
