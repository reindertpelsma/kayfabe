# The support-matrix seam audit — what is already admitted, and what is a retrofit

> **Scope.** The owner's question, verbatim in intent: *"check if the last work/codebase is still
> not conflicting with later supporting (so it does not become a retrofit)"*, plus *"check the
> internal nvidia semantics — we did not creep in assumptions that break across versions."*
>
> ⊘ **No hardware was run for this note.** Every status is a source read of `/workspace/nvkvm-rs`
> at **`f760a4b`** plus the two vendored driver trees, and is labelled `[read]`. Where a claim
> rests on somebody else's run, the run is named. The VMM axis and the Cloud-Hypervisor / QEMU
> adapter seam are **out of scope** — another agent owns them.
>
> ⚠⚠ **OPERATIONAL TRAP, found by this audit falling into it — `rg` is a SHELL FUNCTION here,
> not a binary.** `which rg` resolves nothing; `command -v rg` prints `rg`. So a bare `rg …`
> works, but **`timeout N rg …` execs a nonexistent binary** and dies with
> `timeout: failed to execute process: No such file or directory`. Paired with `2>/dev/null` —
> which is how one naturally writes a grep whose misses are uninteresting — the failure is
> **indistinguishable from a clean zero-hit result**: empty stdout, no error, and the pipeline's
> exit status comes from the `head` at the end. One claim in §0.4 of this note was committed on
> exactly that void evidence; re-run with `grep -rn`, the conclusion happened to hold, but it was
> not earned when it was written. ⇒ **never `timeout … rg`; and an empty result from a search
> whose stderr was discarded is evidence of NOTHING.** State the instrument check, do not assert
> it — this note asserted one it had not run.
>
> ⚠ **The tree moved during this audit** (`6f0077a` → `f760a4b`, two other agents pushing).
> Every `file:line` below was re-verified at `f760a4b`. Re-grep before quoting at a later
> revision; `shim.rs` in particular shifted ~29 lines mid-audit and two subagent reports
> carried the pre-shift numbers.

---

## 0. ★★★ What this audit REFUTES — including its own brief

The brief is wrong in four places, and three of them are wrong in the *favourable* direction.
Leading with them because a flattering audit is worth less than a true one.

### 0.1 ⊘ "Axis 5 (Guest OS) is NEW, and the axis with ZERO coverage." **FALSE, twice over.**

It was recorded as an owner directive on **2026-07-27** (`docs/design/four_axes_of_variation.md:5-8`),
given a code home on **2026-07-29** (`crates/kayfabe-abi/src/guest_os.rs`, 430 lines), and audited
again on **2026-07-31** (`docs/design/compatibility_matrix.md` §2.4). It is not the axis with the
least coverage — it is the **best-documented axis in the tree**, with a typed refusal
(`ClientKindRuleUnknown`), a `GuestOs::Windows` variant that exists precisely so the seam is one
grep away, and a self-proving lexical gate (`tests/tests/guest_os_axis_gate.rs`) that proves it can
fire per token and deleted one of its own tokens for containing its escape word. `[read]`

The axis with genuinely zero coverage is **axis 4 (guest kernel version)** — see §5.

### 0.2 ⊘ "Axis 6 (Host OS = Linux only) is NEW." **FALSE — and it is a decision, not a relaxation
we are receiving.** `four_axes_of_variation.md` §1.1 records the owner's 2026-07-27 ruling that
Windows **host** is out of scope and stays out, with the reasoning (Hyper-V GPU-PV already exists;
smaller audience). The whole tree carries exactly **one** `cfg(target_os)`
(`crates/kayfabe-linux-raw/src/lib.rs:258`). `[read]` Axis 6 costs nothing.

### 0.3 ⊘ "A trait with one impl is untested as a trait — check `HostClasses`, `Arch`/`MockArch`,
the boot FSM." **The named priors have been CLOSED.** `Arch` has three chip impls
(`Ga10xArch`, `Ad10xArch`, `Gh100Arch`); `HostClasses` has three
(`crates/kayfabe-chips/src/host_classes.rs:88,121,168`); and the boot FSM — the task-#121
*"unhooked IMPLEMENTATION"* — now has **two genuinely different orderings** behind `BootSequence`
(`crates/kayfabe-gsp/src/seq.rs:106` falcon/secure-booter, `crates/kayfabe-chips/src/gh100.rs:378`
Hopper FSP), with Ada selecting one and Hopper the other. `[read]` That prior is resolved. The
defect that replaced it is a *different* shape and is §2.1 below.

### 0.4 ⊘ "A hardcoded RPC function number is an axis-2 defect." **REFUTED by measurement of the
two trees.** Diffing `rpc_global_enums.h` between 580.159.04 and 610.43.02: **no function number
and no event number changed.** 610 only *appends* — one function
(`CTRL_GPU_SET_MIGRATION_BLOCK 229`, moving `NUM_FUNCTIONS` 229→230) and seven events
(`0x1023`…`0x1029`, moving `NUM_EVENTS` `0x1023`→`0x102a`). `[read]` Every id the port uses
(`GSP_RM_CONTROL 76`, `GSP_RM_ALLOC 103`, `INVALIDATE_TLB 200`, `GSP_INIT_DONE 0x1001`, …) exists
unchanged at both tags. The enum is append-only across the band, so hardcoding is **safe here**.

⊘ And the two bounds that *did* move (`NUM_FUNCTIONS`, `NUM_EVENTS`) are hardcoded **nowhere** in
`crates/`. `[read]` — `grep -rn 'NUM_FUNCTIONS\|NUM_EVENTS' crates/ --include='*.rs'`, exit 1;
instrument check, same command shape, `ENCODED_FOR_MAJOR` → 3 hits, exit 0. See the ⚠ below for
why that instrument check is spelled out rather than assumed.

### 0.5 ★ The one place the brief was too GENEROUS

The brief lists axis 3's floor as "Turing and newer" as though Turing were the easy end. It is not
the easy end — it is a **fourth** page-table format family. See §2.2. That is the single most
consequential finding in this note.

---

## 1. The re-aimed axis 2 — the vGPU interoperability band

The owner narrowed axis 2 mid-audit to NVIDIA vGPU's own policy: exact match, same major branch
across minors, and n‑1 (previous major or previous LTS branch). ⊘ Arbitrary mismatch is an extra.

⚠ **The branch → version mapping is UNMEASURED here.** Which numeric releases constitute a major
branch, and which are LTS, comes from NVIDIA's published interoperability tables. This note has not
read those tables and **does not reconstruct them** from the two versions vendored under
`research_clones/`. Everything below is therefore written in terms of *branch*, and the only
concrete pair used is the one both vendored trees make checkable: 580.159.04 and 610.43.02.

### 1.1 Do we detect the GUEST driver version? ★ **YES — and we then ignore it.**

The guest sends its own `NV_VERSION_STRING` unprompted in `SET_GUEST_SYSTEM_INFO` (fn 1), and the
port decodes it: `crates/kayfabe-abi/src/guestsysinfo.rs:93` `decode_guest_driver_version`,
offset `6 * 4`, `char[0x100]` (`ogkm-580:`/`ogkm-610: g_rpc-structures.h:36-47`). `[read]`

Its **only** consumer is `crates/kayfabe-device/src/inittables.rs:1368`, which latches it into
`self.guest_firmware` to be echoed back as `GspGetFeatures`' `firmwareVersion`. It is a
**report-only latch.** `[read]`

The value that actually selects the wire ABI is a compile-time constant:

```
crates/kayfabe-qemu-raw/src/shim.rs:807
    pub const GUEST_DRIVER: kayfabe_abi::DriverVersion = kayfabe_abi::versions::BENCH_DRIVER;
crates/kayfabe-qemu-raw/src/shim.rs:2861
    let abi = kayfabe_device::abi::gsp_abi_for(GUEST_DRIVER).map_err(|_| { … })
```

`:2861` is the only non-test caller of `gsp_abi_for`. `[read]` ⇒ **we read the guest's version off
the wire and do not use it.** The information needed to enforce the band is already in hand and is
discarded — that is the finding, and it is cheaper to fix than it looks, because the decode already
exists and `gsp_abi_for` already accepts any version.

⊘ Note the ordering constraint that makes this non-trivial: `gsp_abi_for` runs at realize, before
the guest has sent anything, because the device must answer register reads first. So the fix is not
"pass the decoded version in" — it is a re-selection after fn 1, or a declared property. The
rustdoc at `shim.rs:797-806` states this honestly and calls itself a bolt-on point.

### 1.2 Do we detect the HOST driver version? ★ **YES, properly.**

`crates/kayfabe-abi/src/host_driver.rs` parses the frontend's `versionString` strictly (three
decimal components, no best-guess), pins the encoders to `[580.65.06, 581.00.00)`
(`:91,:101` `ENCODED_FOR_MAJOR`/`ENCODED_FOR_FLOOR`), names the 610 shift point
(`:108` `CHANNEL_PARAMS_SHIFT_MAJOR = 610`), and refuses by name with three arms
(`Unreadable`, `Unparsable`, `NotEncodedFor`) and **no override**. Enforced per connection at
`crates/kayfabe-isolate-host/src/rm.rs:1683`. `[read]` `[their run]` — the pin was exercised in
both directions on 2026-07-31 per `docs/design/host_driver_version_pin.md:129-175`.

This is the strongest version-axis work in the tree and it is exactly the shape the band needs.

### 1.3 ★ Is host conflated with guest anywhere? **NO — and it was deliberately prevented.**

`DriverVersion` (guest) and `HostDriverVersion` (host) are **distinct types**, and the rustdoc at
`crates/kayfabe-abi/src/host_driver.rs:110-116` says why in as many words: *"the same three numbers
about two independent machines … so the type system is told, and a guest-side triple cannot be
handed to the host-side check by accident."* Cross-check: `crates/kayfabe-isolate-host/src/`
contains zero occurrences of `DriverVersion` / `table_for` / `DriverAbiTable` — the host edge
cannot name the guest key. `[read]`

⇒ **The axis-2 conflation the brief asked me to hunt does not exist. It was already closed.** This
drops the whole "host==guest assumption" line of enquiry from C to A.

### 1.4 ★★★ Would anything proceed on an out-of-band pair rather than refuse? **YES — and it is
the forgiven-status trap, exactly.**

`crates/kayfabe-device/src/guestsysinfo.rs:112-117`:

```rust
RpcFunction::SetGuestSystemInfo => match self.agreed_version(&cmd.payload) {
    Ok(ours) => Some(Reply { rpc_result: NV_OK, body: encode_set_guest_system_info_reply(ours) }),
    Err(_) => refuse(),
},
```

`agreed_version` (`:88-96`) constructs a **typed, fully-informative refusal** —
`VersionMismatch { guest, ours }`, carrying precisely the two numbers needed to name an
out-of-band pair — and its own rustdoc (`:78-80`) says it exists *"so each refusal has a name
rather than being 'some `Reply` with a non-zero status'."* The only production caller then does
exactly that: `Err(_)` **discards the value**, and `refuse()` (`:100-105`) returns
`NV_ERR_NOT_SUPPORTED` with an **empty body**. `[read]`

Three consequences, in increasing severity:

1. **Nothing an operator can read names the pair.** The one place that knows both versions throws
   them away. This is the `PushTooFragmented { len: 0 }` shape one level up — not a wrong name, but
   *no* name where a perfect one was already constructed.
2. **`NV_ERR_NOT_SUPPORTED` is the forgiven status.** It is the status RM's own `StatePreInit`
   sweep reads as *"this engine is absent — delete it"* and carries on
   (`ogkm-580: gpu.c:2170-2214`, cited at `crates/kayfabe-device/src/lib.rs`). Choosing it for a
   *version* disagreement invites exactly the run-on-and-fail-elsewhere failure this project has
   run into before.
3. ★★ **It defeats the protocol's own down-negotiation.** `ogkm-580: rpc.c:8765-8804` (and the same
   block at 610) reads `rpc_result_private`, then re-reads the version pair **back out of the
   message buffer** and retries at the host's pair, or reports *"the host version is too old"*.
   `[read]` We return an empty body, so the guest reads back a **zeroed** pair and is told the host
   is version **0.0** — a specific, actionable, false number, which is the durable-wrong-name
   failure mode again.

★ **This is class B, not C, and that is the good news.** `encode_set_guest_system_info_reply(ours)`
already exists and is already called on the `Ok` path. Answering `VersionMismatch` with **our** pair
in the body is a one-line change at `guestsysinfo.rs:113` that converts a silent forgiven status
into the protocol's designed negotiation. Nothing above the policy seam moves.

### 1.5 The band's reachable width today

| | reachable | why |
|---|---|---|
| guest | **580.159.04 only** | `GUEST_DRIVER` is a build `const` (§1.1) |
| host | `[580.65.06, 581.00.00)` | enforced per connection, refused by name (§1.2) |

⊘ Under rule 3 (n‑1), 580 and 610 are the two branches that must both work. The **guest** table
already carries both — and only those two of its eight rows can answer fn 1, because the other six
have `vgx: None` (`crates/kayfabe-abi/src/versions.rs:338,367,386,405,424,461`). `[read]` So the
guest-side layout work is, by luck of where the tables were built, aimed at exactly the required
band. The **host** side spans one branch and the n‑1 obligation is unmet there — but it is unmet
*loudly*, which is the correct posture for work not yet done.

---

## 2. Axis 3 — GPU architecture, Turing and newer

### 2.1 ★★★ The two non-Ampere `Arch` impls answer their data-plane roles **from a mock**

`crates/kayfabe-chips/src/ad10x.rs:298` and `crates/kayfabe-chips/src/gh100.rs:677` both hold
`inner: MockArch`, and the `Arch` impls delegate to it for **four** roles:

| role | Ad10x | Gh100 |
|---|---|---|
| `classify` | `:324` → mock | `:703` → mock |
| `mmu()` | `:357` → mock | `:736` → mock |
| `userd()` | `:360` → mock | `:739` → mock |
| `is_case2_control` | `:363` → mock | `:742` → mock |
| `pushbuffer()` | `:366` → mock | `:745` → mock |

The crate says so itself: *"An `Arch` that is `MockArch` in every respect except that its GSP is
`Ad10xGspModel`"* (`ad10x.rs:290-291`), and `Cargo.toml` describes GH100 as *"a REFUTATION
FIXTURE"*. `[read]`

★ **This is not new and is not hidden** — `kayfabe-mocks`' own manifest records that it is on a
NORMAL edge under `kayfabe-chips` and therefore in the shipped archive's graph, and
`docs/design/mock_fidelity_audit.md` finding G covers it. I verified the edge:
`qemu-raw → chips → mocks` and `isolate-host → chips → mocks`, both non-dev, unconditional.
`[read]`

★★ **What IS new here: the 2026-07-31 compatibility matrix's headline is now stale.** It states
that `kayfabe-chips`, `kayfabe-core`, `kayfabe-fwd`, `kayfabe-rmrpc` and `kayfabe-mocks` are linked
by nothing, and reasons from that ("a seam in a crate on the last line is a seam no guest can
reach"). Re-running its own deriver, `scripts/compat_matrix.py`, at `f760a4b`:

```
NEITHER : kayfabe-crec, kayfabe-shell, kayfabe-vmm-kvm
```

Only three crates are now outside the shipping closure. `kayfabe-chips` **and `kayfabe-mocks`** are
inside **both** the guest-edge and host-edge closures. `[read]` ⇒ every conclusion in
`compatibility_matrix.md` §1 that leans on non-linkage needs re-reading, and the `MockArch`
delegation above is inside the shipped graph rather than beside it.

**Class B, not C** — the seam is right and the fix is local. But it is the trap that matters for
this brief: the moment a second row is added to `CHIPS`, that generation silently inherits an
**invented** MMU format, USERD model and pushbuffer ABI. The refusal type for exactly this
(`UnbuiltGmmu`, `ga10x.rs:895`, whose `levels()` is `0`) exists and these two arches do not use it.

⊘ **Two `Arch` methods were already rescued from this and prove the shape is fixable**:
`decode_doorbell` and `vchid_from_userd_flags` are explicitly *"NOT `self.inner`"* on both arches
(`ad10x.rs:341-354`, task `#156`; `#174` for the USERD chid), each with a driver citation showing
the generation binds the same HAL. `[read]`

⚠ **Stale doc, correct the record.** `docs/design/doorbell_token_encoding.md:271` still says
*"`Ad10xArch`/`Gh100Arch` delegate to `MockArch`'s invented encoding"* for the doorbell. That was
fixed by `6be9fef`, which is a **descendant** of the doc's own commit `6e4f66f` (both 2026-08-01;
`git merge-base --is-ancestor` confirms the order). `[read]` The row is right about
`mmu`/`userd`/`pushbuffer` and wrong about the doorbell.

### 2.2 ★★★ **The declared floor needs a FOURTH page-table format, and it is the one nobody costed**

The owner's floor is Turing. NVIDIA's own dispatch binds the GMMU level builder **five** ways
(`ogkm-610: src/nvidia/generated/g_kern_gmmu_nvoc.c:705-724`, the whole block read):

| chips | `kgmmuFmtInitLevels_…` |
|---|---|
| `TU102 \| TU104 \| TU106 \| TU116 \| TU117` — **Turing** | **`_GP10X`** |
| `GA100 … AD107` — Ampere + Ada | `_GA10X` |
| `GH100` — Hopper | `_GH10X` |
| else — Blackwell | `_GB10X` |
| `T234D \| T239D \| T264D` — Tegra | `_d44104` |

`[read]` kayfabe has **one** real `GmmuFmt`: `Ga10xGmmu` (`crates/kayfabe-chips/src/ga10x.rs:765`),
i.e. `_GA10X`. Spanning Turing→Blackwell needs **four** families, and the declared floor is in a
different one from the only built row.

★ **But the Turing delta is genuinely small, and the repo already knows it.** The oracle test
`tests/tests/gmmu_fmt_oracle.rs:482-485` states it: *"The whole GA10x generation delta is
`kgmmuFmtInitLevels_GA10X`'s single statement `pLevels[2].bPageTable = NV_TRUE`. Judged against
`_GP10X` (Turing) the 512 MiB leaf simply would not exist."* And the **bit** encoders are already
shared — the same oracle compiles `kgmmuFmtInitPde{,Multi}_GP10X` and `kgmmuFmtInitPte_GP10X`, so
GA106's PDE/PTE encoding *is* Pascal's. `[read]` The single divergence in our code is one match arm,
already flagged: `ga10x.rs:817`, *"★★★ GA10x only: a PD1 slot with the valid bit set is a 512 MiB
PAGE."*

⇒ **Turing's `GmmuFmt` is `Ga10xGmmu` minus one match arm.** Class **B** — a new impl in the
adapter crate, zero logic-crate edits, which is precisely what the seam promises. `[read]`

⊘ Hopper/Blackwell (`_GH10X`/`_GB10X`) are a different matter: VER3 is a real format change
(7 levels, 57-bit VA, unified `ADDRESS 51:12` with no VID/SYS split, a `PCF` field replacing
discrete `VOL`/`READ_ONLY` bits). `GmmuVersion::Ver3` is **declared**
(`crates/kayfabe-arch/src/lib.rs:196-201`) and **constructed nowhere**. `[read]` That is still B —
the enum variant and the trait both exist — but it is a real codec, not one match arm.

### 2.3 ★ The three hardcoded selection pins

The seams are parameterised; **selection** is not. Three sites, each a single greppable line:

1. `crates/kayfabe-qemu-raw/src/shim.rs:3501` — `Gpu::new(Box::new(kayfabe_chips::Ga10xArch::new()), …)`,
   unconditional, *nine lines after* the chip **is** resolved dynamically from the PCI device id.
   `ChipProfile` carries `gsp_model: fn() -> Box<dyn GspModel>` (`crates/kayfabe-device/src/lib.rs:305`)
   but has **no arch analogue**. `[read]`
   ⚠ Constraint on any fix: `crates/kayfabe-qemu-raw/tests/e2_doorbell.rs:477-482` scans `shim.rs`'s
   source and asserts `Gpu::new(` appears exactly once — so the selector must be an *expression*
   feeding the one call, not a second call.
2. `crates/kayfabe-chips/src/host_classes.rs:201-203` — `pinned_host_classes()` returns GA10x
   always, though all three profiles exist. Honestly labelled *"pinned, not default"*.
3. `crates/kayfabe-device/src/inittables.rs:2213` — `GSP_GET_FEATURES` answers
   `GspFeatures::GA106` hardcoded, inside the chip-generic arm; not a `ChipProfile` field.

Class **B** for all three (the trait objects and the profile rows already exist), but they are the
work item, not the seam.

### 2.4 ★ One arch constant inside a chip-generic validator — and I refute the subagent finding on it

`crates/kayfabe-abi/src/grinfo.rs:157,159` define `AMPERE_CORES_PER_SM = 128` and
`AMPERE_TENSOR_CORES_PER_SM = 4`, and `:253,255` consume them inside `validate_against`, a
validator that runs over **any** `GrStaticProfile`. A non-Ampere chip row whose real per-SM
geometry differs is refused with `GrInfoError::DisagreesWithGrStatic`. `[read]`

⊘ **I do NOT assert what Turing's value is.** A subagent reported "Turing is 64 cores/SM and 8
tensor cores/SM" as fact; I could not source it. `GPU_CORE_COUNT` appears in the vendored trees
**only** as an SDK index definition — no open-driver code computes it (it is filled by closed GSP
firmware). `[read]` The constant's own rustdoc is candid that it is *"not an ogkm constant"* but
arithmetic pinned by a GA106 observation. ⇒ the honest statement is: **this is a per-silicon
quantity with no readable source in either tree, sitting un-keyed inside a chip-generic validator.**
A second chip row needs a fresh measurement, not a lookup. Class **B** (add a field to the chip
row); the defect is the missing key, not a known-wrong number.

### 2.5 What is genuinely arch-fixed (safe to hardcode), with the reason

| thing | verdict | evidence |
|---|---|---|
| GPFIFO `GP_ENTRY` 8 B, `GET 31:2`, `GET_HI 7:0`, `LENGTH 30:10` | **fixed** | identical in `clc56f.h` (Ampere), `clc86f.h` (Hopper), `clc96f.h`/`clca6f.h` (Blackwell) |
| USERD `GP_GET`/`GP_PUT` @ `34*4`/`35*4`, size 512 | **fixed** | `maxwell/gm107/dev_ram.h:47,48,50`; GA106 takes Maxwell's HAL arm |
| CE subchannel = 4; UVM binds on 0, fires on 4 | **fixed** | `cla06fsubch.h:30` **byte-identical** 580↔610; `uvm_maxwell_ce.c:29-37`; and `fixed_subch` is a *parameter* at `kayfabe-arch/src/lib.rs:1228-1237`, not a constant |
| doorbell reg offset `0x90` | **fixed Volta→Blackwell** | `turing/tu102`, `ampere/ga100`, `blackwell/gb100`, `blackwell/gb202` `dev_vm.h` all agree; and it is *derived* from the advertised reg base at `doorbell.rs:225-232` |
| `RM_ENGINE_TYPE` / `NV2080_ENGINE_TYPE` | **fixed across the band** | `gpu_engine_type.h` **byte-identical** 580↔610 |
| work-submit token `VECTOR 11:0`, `RUNLIST_ID 22:16` | **fixed TU→GH; refused at GB** | `g_kernel_fifo_nvoc.c:643-662` binds GA100…GH100 to one encoder; Blackwell adds a bit outside our mask so it decodes to `None` — a loud refusal, correct |
| class ids (CE/GR/GPFIFO/USERMODE) | **arch-varying, parameterised** | per-chip lists in `g_gpu_class_list.c` are **identical between 580 and 610** ⇒ class ids vary by *silicon*, not by driver version; `HostClasses` is keyed the right way |

⊘ These are **not** "it works on the bench" — each is a cross-header or cross-tree identity.

---

## 3. Internal NVIDIA semantics — version-varying constants

### 3.1 ★★ A product validator STRICTLY NARROWER than the driver it must accept

This is the brief's named prior shape (*"a test double narrower than the loosest supported driver
passes while the product fails"*) found **in the product, not a mock**.

`crates/kayfabe-gsp/src/element.rs:667-686` validates the 610 MCTP/NVDM transport headers by
comparing **both whole 32-bit words** for exact equality against `0xC000_0001` and `0x2510_DE7E`
(`crates/kayfabe-abi/src/versions.rs:192,195`), raising `TransportHeaderInvalid` on any difference.
`[read]`

The driver validates **two fields**, not two words
(`ogkm-610: src/nvidia/src/kernel/gpu/gsp/message_queue_cpu.c:737-758`):

```c
NvU32 mctpVersion = REF_VAL(MCTP_HEADER_VERSION, …->mctpHeader);
NvU32 vendorId    = REF_VAL(MCTP_MSG_HEADER_VENDOR_ID, …->nvdmHeader);
if (mctpVersion != 0x1)                        { … NV_ERR_INVALID_DATA; continue; }
if (vendorId != MCTP_MSG_HEADER_VENDOR_ID_NV)  { … NV_ERR_INVALID_DATA; continue; }
```

`[read]` The MCTP header word also carries SOM/EOM/sequence/tag bits, none of which the driver
constrains. Our check rejects any element whose transport word differs in those bits. Class **B**
(compare the two fields, which the layout already locates), latent today only because a 610 guest
cannot be selected at all (§1.1).

★ Note the citation at `element.rs:640` is *correct* about ordering and about 580 having no
transport header — the defect is strictness, not provenance.

### 3.2 A control-param size that moved and is not keyed

`crates/kayfabe-abi/src/gpuinfo.rs:109` pins `GPU_INFO_MAX_LIST_SIZE = 0x46`, giving a 564-byte
params struct at `:116`. `ogkm-580: ctrl2080gpu.h:122` is `0x46`; **`ogkm-610: ctrl2080gpu.h:127`
is `0x48`** ⇒ 580 bytes at 610. `[read]` Latent: `answer_gpu_get_info_v2` has no non-test caller
yet. Class **B** — a `GpuInfoWire` field on `DriverAbiTable`, the same shape `MapDmaWire` and
`GspStaticInfoWire` already use.

### 3.3 A wrong citation

`crates/kayfabe-abi/src/versions.rs:598-600` cites `ogkm-610: message_queue_priv.h:112` for
`GSP_MSG_QUEUE_ELEMENT_SIZE_MIN`. **610 deleted that macro**; `:112` is
`gspMsgQueueGetMaxRpcSize`, a different symbol. At 610 the quantity became a runtime field
(`message_queue_cpu.c:88-91`). `[read]` The *value* (4096) is still right, so nothing is broken —
but the justification is false and the 610 mechanism is runtime-published, which is the more
useful fact. Class **B**.

### 3.4 A comment that describes stronger behaviour than the code

`crates/kayfabe-gsp/src/boot.rs:971-980` — the comment says *"use the guest's number instead of the
version key's"*; the code compares and refuses. `[read]` Using the declared value would make this
seam version-free, and 610 hands it to us: `message_queue_cpu.c:82-86` folds the encryption-tag
size into `queueElementHdrSize` when confidential compute is on, so a CC-on 610 guest declares a
larger header and is refused against the table's constant. Class **A** — one line, the field is
already read.

### 3.5 Host-side struct sizes, pinned and guarded

`crates/kayfabe-abi/src/submit.rs:373` `ChannelAllocParams::SIZE = 368` is the 580 layout; 610
inserts `hHandleVASpace` after `hVASpace` (`ogkm-610: alloc_channel.h:312`), shifting everything
from +32 by four bytes, `engineType` included. `[read]` ⊘ **Not an unnoticed defect** — this is
precisely what `host_driver::check` refuses by name (§1.2), and `CHANNEL_PARAMS_SHIFT_MAJOR = 610`
exists to make the refusal explain itself. Class **C** to actually support a second host branch:
the encoders in `submit.rs` and their call sites in `crates/kayfabe-isolate-host/src/rm.rs` take no
version parameter, so the host side needs the twin of `MapDmaWire`. Blast radius: one crate's
encoders plus one crate's call sites, both enumerable.

### 3.6 What I checked and found stable — the negative results

RPC envelope 32 B (`g_rpc-message-header.h` **byte-identical** 580↔610);
alloc/control body headers 32/40 (`g_rpc-structures.h`, compared member-by-member) `[read]`;
`RPC_HEADER_VERSION 0x0300_0000` (`rpc_headers.h:58-59` identical at both — I read both);
msgq library (`msgq_priv.h` **byte-identical**, only the path moved);
LibOS init args (`libos_init_args.h` **byte-identical**);
`gsp_fw_wpr_meta.h` (**byte-identical**; kayfabe models no such struct);
FWSEC/VBIOS parse path (all three driver files **byte-identical** ⇒ the one-variant `VbiosWire` is
justified);
WPR2/FRTS derivation (610's only change is a literal becoming a named constant of equal value);
engine-info ordinals and `GET_DEVICE_INFO_TABLE` bounds (`engine_info.h` byte-identical).
`[read]`

⊘ **Header drift is nonetheless large** — `ctrl2080gpu.h` alone changed 675 lines between the tags,
and §3.2 is unlikely to be the only bad row. The ~40 `*_PARAMS_SIZE` constants in `kayfabe-abi` are
bare `const`s and only three of the four the port *decodes* carry a version selector. A
generator-driven diff of all of them against both trees is the cheap sweep and is not written.

---

## 4. Class D — code or a test asserting an axis's negation

★ **None found on any axis.** Nothing asserts the guest must be Linux, that the chip must be GA106,
or that host and guest versions are equal. The nearest candidates all point the *other* way:
`crates/kayfabe-device/tests/chip_table.rs` declares a whole second synthetic chip and drives the
same register plane through it; `tests/tests/arch_axis_second_generation.rs` boots the unmodified
GSP FSM on AD10x and drives GH100 through its own FSP sequence.

⊘ The closest thing to a D is §2.1 — `Ad10xArch::mmu()` returns an **invented** format where the
driver's own dispatch proves `_GA10X` is correct for Ada. That is a wrong *answer*, not an asserted
negation, so it is filed B. It is worth noting that it is the only place in this audit where the
code states something demonstrably false about NVIDIA rather than merely incomplete.

---

## 5. The axis with genuinely no home — guest kernel version

`GuestKernelVersion`, `LINUX_VERSION_CODE`, `KERNEL_VERSION(`, `vermagic`, `utsname`, `osrelease`
have **zero** hits in tracked Rust. `[read]` It is the only axis with no refusal, where every
neighbour has one (`NoTableForVersion`, `UnknownGuestOsName`, `NotEncodedFor`, `NoChipForDevice`).

★ It is also the axis where that is most defensible: Mode 2 needs **no guest kernel module**, and
the emulated device reads RM/GSP protocol rather than guest kernel structures (§6). Host-kernel
gating, by contrast, is done the right way — by runtime probe, not version number
(`crates/kayfabe-linux-raw/src/kvm_unsafe.rs:58-59` probes `KVM_CAP_NR_MEMSLOTS`;
`spawn_unsafe.rs:126-133` retries without a 6.3+ `memfd_create` flag rather than requiring it).
`[read]`

---

## 6. Axis 5 — why guest OS is cheap, stated as evidence rather than hope

`crates/kayfabe-rmrpc/src/lib.rs:1213-1219` names the whole OS surface: *"the one genuinely
guest-OS-shaped value in the whole bridge … nothing else in this crate is OS-aware."* It is
`NV0000_ALLOC_PARAMETERS.processID`, whose `KERNEL_PID` sentinel is gated on
`RMCFG_FEATURE_PLATFORM_UNIX` (`ogkm-580:`/`ogkm-610: rpc.h:67-77`, byte-identical). `[read]`
Everything else the guest edge parses is the RPC envelope, function codes, and NVIDIA SDK alloc /
control bodies — which a WDDM driver sends identically.

Remaining axis-5 work, all **B/C-small**:
- **C (small):** `GuestOs::from_config_name` has no non-test caller and there is no QEMU property;
  `GuestOs::Linux` is hardcoded at `crates/kayfabe-qemu-raw/src/shim.rs:3531` and `:3557`. `[read]`
  (⚠ two subagents reported `:3448`/`:3474` — pre-shift numbers. Re-grep.)
- **B:** a WDDM *display* miniport would need a scanout path; `modeset`/`EDID` are zero in the tree.
  The PCI identity is already a display-class device (`crates/kayfabe-abi/src/vbios.rs:460-461`,
  class `0x030000`) and a VBIOS is served, so a compute-only bind may not need one.
- **C ×2, and these are the real ones:** MSI-X with no INTx fallback *and no detection* — the
  refusal fires when **we** raise INTx, never when the guest declines to enable MSI-X
  (`crates/kayfabe-vmm-qemu/src/lib.rs:2271`); and guest-physical addresses consumed raw with
  `iommu`/`swiotlb` appearing zero times, so a vIOMMU guest fails as `BadGpa` — a refusal
  attributed to the wrong cause. Both are invisible to `guest_os_axis_gate.rs` because neither is
  spelled in its vocabulary.

---

## 7. Ranked by retrofit cost

| # | finding | axis | class |
|---|---|---|---|
| 1 | Host-side encoders take no version parameter; a second host branch means re-keying `submit.rs` + `rm.rs` call sites (§3.5) | 1 | **C** |
| 2 | MSI-X-only and raw-GPA, both undetected and mis-attributed on failure (§6) | 5 | **C** ×2 |
| 3 | Guest driver version is a build `const`; the decoded wire value is report-only (§1.1) | 2 | **C** (selection mechanism) |
| 4 | `GuestOs` has no front door; `Linux` hardcoded at two composition-root sites (§6) | 5 | **C** (small) |
| 5 | Arch **selection** is three hardcoded pins, though every seam is parameterised (§2.3) | 3 | **B** |
| 6 | Turing needs `_GP10X` levels — `Ga10xGmmu` minus one match arm (§2.2) | 3 | **B** |
| 7 | Hopper/Blackwell need a real VER3 codec; `GmmuVersion::Ver3` declared, never constructed (§2.2) | 3 | **B** |
| 8 | `Ad10x`/`Gh100` answer mmu/userd/pushbuffer/classify from `MockArch`, now inside the shipping closure (§2.1) | 3 | **B** |
| 9 | Out-of-band version pair is refused *anonymously* with the forgiven status, defeating down-negotiation (§1.4) | 2 | **B** (one line) |
| 10 | MCTP/NVDM validated as whole words where the driver checks two fields (§3.1) | 2 | **B** |
| 11 | `GPU_INFO_MAX_LIST_SIZE` 0x46→0x48 at 610, unkeyed (§3.2) | 2 | **B** |
| 12 | `AMPERE_CORES_PER_SM` inside a chip-generic validator, with no readable source for a second arch (§2.4) | 3 | **B** |
| 13 | Six of eight ABI rows carry `vgx: None` and cannot answer fn 1 (§1.5) | 2 | **B** (table edit) |
| 14 | Boot-args header size compared where the comment says use (§3.4) | 2 | **A** (one line) |
| 15 | Host/guest version conflation — **does not exist**, already closed (§1.3) | 2 | **A** |
| 16 | Guest OS seam, refusal, and self-proving gate (§0.1, §6) | 5 | **A** |
| 17 | Host OS Linux-only; one `cfg(target_os)` in the tree (§0.2) | 6 | **A** |
| 18 | RPC function/event numbers, engine-type enums, subchannels, doorbell offset, GPFIFO/USERD layout (§0.4, §2.5) | 1/2/3 | **A** |

**No class D on any axis.**

---

## 8. Corrections to existing docs

1. `docs/design/compatibility_matrix.md` §1 — the shipping-closure list is stale. `kayfabe-chips`,
   `-core`, `-fwd`, `-rmrpc`, `-mocks`, `-rt` are all linked at `f760a4b`; only `-crec`, `-shell`,
   `-vmm-kvm` are not. Every conclusion that leans on non-linkage needs re-reading (§2.1).
2. `docs/design/compatibility_matrix.md` §2.1 cites `shim.rs:602` for `GUEST_DRIVER`; it is
   **`:807`** at `f760a4b`.
3. `docs/design/doorbell_token_encoding.md:271` — the doorbell delegation was fixed by `6be9fef`,
   a descendant of that doc's own commit (§2.1).
4. `crates/kayfabe-abi/src/versions.rs:598-600` — wrong 610 citation (§3.3).
