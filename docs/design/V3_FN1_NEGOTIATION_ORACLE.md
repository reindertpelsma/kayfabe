**STATUS: LIVE, 2026-10-13.** An oracle test (NVIDIA's own guest-side `RmRpcSetGuestSystemInfo`, ogkm
595.84, compiled and driven) for the fn-1 version negotiation; kayfabe compared against it; the one
divergence class fixed. Section 6 is an investigation (no code change) of how the guest version could be
detected instead of declared.

Labels: `MEASURED` = produced by running the oracle or the tests in this change, or read at the cited
file:line. `INFERRED` = my reading of what the measured lines imply. Nothing in this change touched a GPU,
QEMU or a host.

# V3 fn-1 (SET_GUEST_SYSTEM_INFO) negotiation oracle

## 1. What the guest does (MEASURED, ogkm 595.84 `src/nvidia/src/kernel/vgpu/rpc.c`)

| step | source | behaviour |
|---|---|---|
| request | `RmRpcSetGuestSystemInfo` `:8836-8943` | body = `rpc_set_guest_system_info_v` (792 B): the guest's own `VGX_{MAJOR,MINOR}_VERSION_NUMBER` pair, `NV_VERSION_STRING`, `NV_BUILD_BRANCH_VERSION`, title, changelist |
| `rpc_result != OK` | tail of `_issueRpcAndWait` `:2008-2025` | returned verbatim (values below `0xFF000000`) or mapped to `NV_ERR_GENERIC`; **a hard stop**, the body is never read |
| `rpc_result == OK`, `rpc_result_private != OK` | `:8947-8982` | status becomes the private status. If the pair **in the body** differs from the guest's own: when `RPC_VERSION(body pair) >= RPC_VERSION(min supported)` the guest re-issues the same buffer **once** and the retried status is *not* re-checked against `rpc_result_private`; otherwise it logs "host version is too old", status stays the private status |
| `OK / OK` | `:8985-9013` | the guest stores **the reply's** pair (fails `NV_ERR_INVALID_STATE` if it contradicts a stored one), calls `rpcSetIpVersion(pair)` and the fn-64 tail call |
| min supported | `g_rpc-structures.h:12822-12825` | `0x25.0x0E`; generated from `rpc-structures.def`, read by the oracle from the header |

The retry rule has **no order check** (MEASURED by the grid): any host pair at or above the minimum is
followed, whether the guest's pair is older or newer, and a private status whose body pair equals the
guest's own is final (no retry). The log text ("Guest VGX version is newer ...") describes the intent;
the code does not test it.

## 2. The oracle

`crates/kf-abi/tests/oracle/`:

* `fn1_negotiation_oracle.c` - the program.
* `build_fn1_oracle.sh <ogkm-root> <out>` - cuts the slices out of `rpc.c` and compiles. Exit 0 built, **77
  SKIPPED** (tree absent; prints `SKIP: ogkm tree not found at ...`), 1 = tree present but anchors moved or
  compile failed (a rotted oracle is an error, never a skip).
* Pinned tree: `/workspace/ogkm-tars/open-gpu-kernel-modules-595.84`, relocatable with `KAYFABE_OGKM_595`.
  Nothing from the tree is vendored (MIT/GPL-2.0 source is read where it is).

Compiled **byte-for-byte** from the tree: `RmRpcSetGuestSystemInfo` (the whole function) and the
"Now check if RPC really succeeded" tail of `_issueRpcAndWait`. Real headers: `rpc_set_guest_system_info_v`,
`rpc_message_header_v`, `RPC_VERSION_FROM_VGX_VERSION`, the `VGX_*` and `MIN_SUPPORTED_INTERNAL` constants,
the `NV_VGPU_MSG_RESULT_*` codes, `DRF_*`, the internal->external mapping table.

**Shimmed** (listed in the C file header too): the object structs (`OBJGPU`, `OBJRPC`, `OBJSYS`) and the
`IS_*` predicates (one GPU, so the multi-GPU "skip the handshake on the second GPU" branch is not exercised);
the guest identity macros (`VGX_*`, `NV_VERSION_STRING`, branch, title, changelist) become runtime values so
one binary plays every guest; the transport (`_issueRpcAndWait` puts the request on stdout, takes a host reply,
writes it over the shared buffer as a shared-memory plugin would, and runs the real tail);
`rpcWriteCommonHeader` (zero + function + length); `rpcSetIpVersion` records its argument (the 191-range
function table is **not** modelled); `NV_PRINTF`/`NV_RM_RPC_LOG` record; `portStringCopy`; the fn-64 tail call
is recorded.

Modes: `consts` (the tree's constants), `grid` (every cell of the built-in host models), `cell <model> g h`
(one cell, any pair), `drive <pair> <driver> <branch> <cl>` (the host is on stdin: this is how kayfabe's real
policy is driven).

**The grid.** Pair universe = both ends of every row of the tree's `NV_VGPU_GRIDSW_INTERNAL_TO_EXTERNAL_VERSION_MAPPING`,
the tree's own VGX pair, the minimum supported pair and the two neighbours just below it (59 pairs in the C grid, 61 in the kayfabe comparison, which adds every served row's pair; none typed
by hand) x 59 host pairs x 4 host models = 13924 cells: `accept-guest` (OK/OK, body unchanged),
`steer-to-host` (the protocol's down-negotiation: private status + host pair when the guest's differs),
`steer-always-private` (private status even for an equal pair), `hard-fail-mismatch` (`rpc_result =
NV_ERR_NOT_SUPPORTED` on a mismatch, what kayfabe did).

`crates/kf-abi/tests/fn1_negotiation_oracle.rs` pins what the oracle says (7 tests): equal pair accepted in one
exchange; steer followed once in either direction when the host pair >= min, `too_old` below it with the private
status verbatim; `rpc_result != 0` never negotiates; a private status naming the guest's own pair is final;
`accept-guest` leaves the guest on its own pair even below the minimum; the agreed pair is what reaches
`rpcSetIpVersion`; and the constants kayfabe sends equal the tree's (`0x6a`, `0x56`).

## 3. kayfabe against it

`crates/kf-rm/tests/fn1_negotiation_oracle.rs`: for every served `guest-driver=` row (29, from
`kf_abi::generated::matrix::MEASURED`) and every pair in the universe, a guest that says it IS that driver is
driven against the real `GuestSystemInfoPolicy` (reply converted to the wire as `GspFsm` does, one policy per
exchange) and its decision is compared with the oracle's `steer-to-host` cell for (guest pair, row pair): status,
exchanges, agreed pair, `ip_version`, fn-64 calls.

**Divergences found (MEASURED, before the fix: 1740 of 1769 cells (29 rows x 61 pairs), i.e. every cell with guest pair
!= row pair; the 29 exact cells passed):**

1. **Class A, 1620 cells, row pair >= the guest minimum.** The oracle's guest follows a steer: OK/private/host pair,
   one retry, ends accepted on the *row's* pair with the fn-64 tail call. kayfabe answered `rpc_result =
   NV_ERR_NOT_SUPPORTED`: a hard stop, one exchange, nothing agreed. Cause: `Reply` held one value and wrote it
   to both `rpc_result` and `rpc_result_private`, so the two could not differ.
2. **Class B, 120 cells, row pair below the guest minimum (the 535/545-era rows).** Both fail; the oracle's guest
   ends with the protocol's `0x6a` (`NV_VGPU_MSG_RESULT_GUEST_HOST_DRIVER_MISMATCH`) and the "too old" log,
   kayfabe with `0x56`. The distinction matters to a guest-side reader of the log.
3. **Found while designing the fix, not by a cell:** the retry is the buffer the reply was written over. A steer
   reply authored like the success reply (zeros plus the pair) would make the retry arrive with an empty
   `guestDriverVersion`, which the driver cross-check refuses as a different release. The oracle's `drive` mode
   carries the buffer exactly, so the grid cell goes red until the body keeps the guest's identity.
4. The existing test `a_guest_speaking_another_version_is_refused_rather_than_agreed_with` asserted the refusal
   (`NV_ERR_NOT_SUPPORTED`, empty body). Its load-bearing half (never agree to the guest's pair) is kept; the
   assertion is rewritten (`a_guest_speaking_another_pair_is_steered_to_ours_and_never_agreed_with`), and a new test
   keeps a *different driver release* a hard refusal.

## 4. The fix (small, derived from the oracle)

* `kf_gsp::Reply` gains `rpc_result_private: Option<u32>` (`None` = same as `rpc_result`, exactly the old wire).
  55 literal sites mechanically gain `rpc_result_private: None`. `Reply::on_wire` is the one conversion (three
  `GspFsm` sites use it). Every reply but one is byte-identical to before.
* `GuestSystemInfoPolicy`: on `VersionMismatch` for an identity the driver cross-check accepted, reply
  `rpc_result = OK`, `rpc_result_private = NV_VGPU_MSG_RESULT_GUEST_HOST_DRIVER_MISMATCH` (a named constant in
  `kf_abi::guestsysinfo`, test-pinned to the tree's value), body = the **row's** pair (from `DriverAbiTable`, as
  before) plus the guest's changelist and its printable-ASCII, NUL-terminated identity strings re-authored from the
  decoded request (a string with a control byte, or without a NUL, is left empty; the three lengths stay zero).
  The guest's pair selects nothing: the guest ends on the row's pair or fails by name.
* Unchanged: exact match (reply byte-identical, the authored-not-reflected test still passes); a different driver
  release, a truncated payload, a row with no VGX citation: `rpc_result = NV_ERR_NOT_SUPPORTED`.
* No version number is typed: the row's pair is the table's, the min and the result code come from the tree.

Why steer rather than "accept the older guest" (what a plugin might do for a guest older than the host): kayfabe's
layouts are measured for the row's pair only; agreeing to the guest's pair would select that guest's RPC tables
against layouts measured for another. Steering is safe in both directions precisely because the guest code has no
order check (section 1).

**Test results (this branch):** `cargo test -p kf-abi -p kf-rm -p kf-gsp` all green, including the 7 oracle-pin
tests, `kayfabe_and_the_real_guest_agree_on_every_pair_of_every_served_row` (red at `892676ecf`, green at the
fix), the Windows-twin test (guest `580.88`/`r580_78-7`/cl 0 with every pair in the universe against the
`580.65.06` row, the retry still identified as the twin) and the hostile-string test. `cargo clippy` on the three
crates: no new warnings in touched code (existing warnings elsewhere untouched).

## 5. NOT covered

* **The closed host plugin.** What NVIDIA's real vGPU plugin answers to an older or newer guest (silently accept an
  older one? steer both ways?) is not in ogkm. The oracle models three host behaviours and proves kayfabe's reply
  is *followed* by the real guest code; it says nothing about what a real plugin does.
* **The closed Windows driver.** Whether it runs this code, its VGX pair, its minimum, whether it retries once: not
  measured. The Windows-twin test drives the *Linux* guest code with the Windows identity strings. A Windows guest
  that treats a private status differently would still be refused by name or hang the handshake; this needs a
  Windows run (not done, no GPU).
* **Other ogkm versions.** The oracle is 595.84. Earlier trees' versions of the function (580.x is cited as
  `rpc.c:8760-8828` in `kf-abi/src/guestsysinfo.rs`) were not read in this change; the retry rule is INFERRED to be
  the same there. The minimum (`0x25.0x0E`) is 595.84's: an older guest has its own, which is why class B is
  reported as "too old" for the oracle's guest only.
* The multi-GPU skip rule (one GPU is shimmed), the `rpcSetIpVersion` function-table selection (only the argument is
  checked), fn 64's body, and real hardware (nothing ran on a GPU or in QEMU).
* The chain-level path: tests drive `GuestSystemInfoPolicy` and the FSM conversion helper, not a booted guest.
  INFERRED from reading that the other fn-1 observers (`HostStubPolicy`, `SwRunlistHostOwnedPolicy`, the
  `GspGetFeatures` latch, `osevent`) take "the most recent handshake wins", so seeing fn 1 twice is idempotent.

## 6. Detecting the guest version (owner addition, investigation only, no code change)

Requirement: the guest version should be DETECTABLE from the guest's declared state but never trusted; any
decision on it must be safe for a false declaration; a mismatch or too-large skew is refused by name.

### 6.1 Where `guest-driver=` is consumed (MEASURED, read)

Path: `qemu/hw/misc/kf3/kf3.c:1906` (`DEFINE_PROP_STRING("guest-driver")`) -> `kf3_realize(..., s->guest_driver, ...)`
(`:1606`) -> `kf-qemu/src/ffi_unsafe.rs:144,176` -> `DeviceConfig.guest_driver` -> `kf-qemu/src/device.rs:573-583`:
unset defaults to the **host's** version; the string is parsed (`DriverVersion::parse`, strict) and must have a
`table_for` row.

| consumer (device.rs unless stated) | what it takes from the row | when |
|---|---|---|
| `kf_rm::abi::gsp_abi_for(version)` -> `console.fsm(abi)` (`:582`, `:970`) | `GSP_MSG_QUEUE_ELEMENT` framing, `queueElementSizeMax`, `MESSAGE_QUEUE_INIT_ARGUMENTS` offsets, RPC header | **before fn 1** (realize; fixed for the device's life) |
| `kf_abi::vbios::build(.., table.vbios_wire())` (`:616`) | the synthetic VBIOS parse path | **before fn 1** (the guest reads the ROM at probe) |
| `GspFsm::publish` reading RMARGS (`kf-gsp/src/boot.rs:~1507`) | init-args layout from the row (`element_hdr_size_off` exists only where the guest declares it) | **before fn 1** (boot handshake) |
| `kf_abi::timer::layout(version)` (`:663`) | timer register layout | realize, only with `KF3_TIMER_MAP=1` |
| `kf_rm::ReselectAtFn1::new(*table, source, build)` (`:951-957`) -> `build(table)` | **every** chain link: `GuestSystemInfoPolicy` (VGX pair, driver cross-check), `StaticInfoPolicy` (fn 65 `sizeof(GspStaticConfigInfo)` and encoding, `kf-rm/src/staticinfo.rs:465-520`), `InitTablePolicy` (control answers by version, W349 refusals), the display link, `GspGetFeatures` | after fn 1 (the chain is **re-built at fn 1** when the version was *defaulted*, `Reselection::Reselected`) |
| `RPC_UNLOADING_GUEST_DRIVER_V.at(version)` (`:726`) | fn-47 body layout | at unload |
| `DisplayPlane::build(row, table, ..)` (`:863`) | display wire | after fn 1 |
| `GspGetFeatures.firmwareVersion` | **not** the row: the guest's own fn-1 string, latched (`kf-rm/src/inittables.rs:1813`, `kf-abi/src/gspfeatures.rs`) | after fn 1 |

`GuestDriverSource::Declared` (a `guest-driver=` was given): fn 1 refuses a different release by name
(`RefusedDeclared`). `Defaulted`: fn 1 re-selects the **post-fn-1** chain to the guest's reported release when
`kf_abi::versions::pre_fn1_surface_differs(provisional, reported)` is `None` (element wire, init-args wire, element
max, VBIOS wire, the numbers of the pre-fn-1 RPCs, the two struct layouts), else refuses by name. So detection at
fn 1 **already exists for the second half**; what is frozen at realize is the first half (the three **before fn 1**
rows), keyed on the declaration or the host's version.

### 6.2 What the guest places in memory before fn 1 (MEASURED from ogkm, one gap INFERRED)

* **Order** (MEASURED): `kgspInitRm` -> `kgspBootstrap_HAL` -> inside it `kgspSendInitRpcs`
  (`kernel_gsp_tu102.c:533`, `kernel_gsp_gh100.c:1040`) sends `GSP_SET_SYSTEM_INFO` and `SET_REGISTRY`
  (`kernel_gsp.c:4607,4614`); only after boot comes fn 1 (`:5060`), then `GET_GSP_STATIC_INFO` (`:5067`). So the
  bodies and **lengths** of two version-structured RPCs also precede fn 1.
* **The `.fwversion` check** (MEASURED, `kernel_gsp.c:5571-5624`): `_kgspFwContainerVerifyVersion` finds the ELF
  section `.fwversion` in the firmware **container** (`pGspFw->pBuf`, the file the OS gave the driver), requires
  `size == strlen(NV_VERSION_STRING)+1` and a byte-equal string, else `NV_ERR_INVALID_DATA` (the container is
  the file `request_firmware` returned, `kernel-open/nvidia/nv.c`). The ELF walker is `_kgspFwContainerGetSection`
  (`:5914`, 64-bit LE ELF, section header table + `.shstrtab`).
* **Is that container visible to the device?** MEASURED: no. After the check the driver extracts only three
  sections: `.fwimage` -> `pGspFw->pImageData` (`_kgspPrepareGspRmBinaryImage`, `kernel_gsp.c:~5686`), `.fwsignature_<chip>` -> a signature memdesc
  (`_kgspCreateSignatureMemdesc`), and the build-id section (`GSP_BUILD_ID_SECTION_NAME`) into a CPU-side copy. The
  radix-3 table handed to the booter covers **`.fwimage` only** (`kgspCreateRadix3(... pGspFw->pImageData,
  pGspFw->imageSize)`, `:5720`; `pWprMeta->sizeOfRadix3Elf = pGspFw->imageSize`, `kernel_gsp_tu102.c:769`). The
  container with `.fwversion` stays in guest kernel memory at an address nobody tells the device. Reading it
  would need a scan of guest RAM: unbounded, and forbidden by the hostile-guest rule. **kayfabe cannot read
  `.fwversion` at boot** without a new guest-facing mechanism.
* **What the device is told, bounded and addressable** (MEASURED): the SEC2 mailbox pair on Booter Load carries the
  physical address of `GspFwWprMeta` (`kernel_gsp_booter_tu102.c:100-103`, `sysmemAddrOfData`). The meta
  (`gsp_fw_wpr_meta.h`) holds `magic`/`revision` (handoff interface revision, bumped when GSP-RM ucode is revoked),
  `sysmemAddrOfRadix3Elf`/`sizeOfRadix3Elf`, `sysmemAddrOfBootloader`/`sizeOfBootloader`, the signature address and
  size, and the FB layout. The FSM today latches only the booter *argument* (`seq.rs:239`), it does not read the
  meta. The LibOS boot args (MAILBOX0/1 -> region array -> `RMARGS` -> `GSP_ARGUMENTS_CACHED`) **are** read at
  boot (`boot.rs publish`), and their *shape* is version dependent: 580 is the 4-field `MESSAGE_QUEUE_INIT_ARGUMENTS`
  (no element-header size), 595/610 declare `queueElementHdrSize` and 5 more fields (`boot.rs:70-100`).
* **What those carry about the version** (INFERRED): `sizeOfRadix3Elf`, `sizeOfBootloader` and the signature size
  are per-firmware-build numbers, i.e. a fingerprint, but a fingerprint table is a captured table
  ("derive, never capture"). The `.fwimage` is itself an ELF of the closed GSP-RM; whether it embeds the driver
  version string in a section kayfabe could walk with bounded reads (ELF header, section table, one string) is
  **unmeasured**: there is no GSP firmware binary on this box (`/lib/firmware/nvidia` absent). A measurement on a
  box with the open driver installed (`gsp_*.bin` under `/lib/firmware/nvidia/<ver>/`) would settle it: list the
  container's and the `.fwimage`'s sections for two tags; the falsifier is "no version-bearing section in
  `.fwimage`".
* **It adds no trust** (INFERRED): on Linux the fn-1 string and `.fwversion` are the same `NV_VERSION_STRING` by
  the check above; a guest that forges one forges both. On Windows the repo records the firmware `.fwversion` as
  `580.65.05` against a driver string `580.88` (`kf-abi/src/windows_twin.rs:27-29`), so even the equality does not
  hold there. A firmware string is a second *declaration*, useful for consistency, not for trust.

### 6.3 Proposal (not implemented)

A pure function, no I/O:

```
detect(fn1: Fn1Declared { driver: &[u8;256], branch: &[u8;256], cl: u32, vgx: (u32,u32) },
       fw: Option<&str>) -> Result<Detected { row: DriverVersion, via: Native | WindowsTwin }, Refusal>
```

* Strict, canonical match: the driver bytes up to the first NUL must be printable ASCII, <= 31 bytes, parse with
  `DriverVersion::parse`, **round-trip byte-exactly** (`format!("{v}") == s`: today `parse` also trims and accepts
  `"580.65.6"`, `"0580.65.06"`, `" 580.65.06"`, `"595.84.00"` as aliases of one row, harmless but wider than
  needed - MEASURED by reading `lib.rs:307-330`), and have a `table_for` row. Else the Windows-twin rule
  (`windows_twin::linux_twin`: name + branch + changelist). Else a named refusal: `Malformed`, `Unmeasured`
  (never "nearest lower row"), `TwinMismatch`, `SkewTooLarge`.
* The VGX pair is a **witness, not an input**: it must equal the row's pair, or the exchange goes through the
  down-negotiation of this change (so a Windows guest with its own unknown pair still ends on the row's pair). A
  pair that differs and a host pair below the guest's minimum is refused by the guest itself with `0x6a`, by name.
* `fw: Option<&str>` (if a bounded reader is ever built from section 6.2) only **vetoes**: `Some(s)` that is not
  the row's `.fwversion` string (Linux) or the twin's recorded firmware string (Windows) refuses; `None` never
  changes the row.
* `guest-driver=` stays only as an optional **pin**: `Some(pin)` and `detect(..) != pin` refuses the boot by name
  (today's `RefusedDeclared`). Unset: the provisional row for the three before-fn-1 consumers is the host's
  version (today's default), and `detect` at fn 1 either equals it (the common case), or must pass
  `pre_fn1_surface_differs(provisional, detected) == None` to re-select (today's `RefusedSurface` otherwise).
* **The honest limit:** the three before-fn-1 consumers (FSM framing, VBIOS, init args) cannot be chosen from fn 1.
  Either keep them provisional and verify at fn 1 (above), or add a **pre-fn-1 witness** - the init-args shape
  (`queueElementHdrSize` present or not distinguishes 580 from 595/610 today) and the `GSP_SET_SYSTEM_INFO` /
  `SET_REGISTRY` payload lengths - and refuse by name on a disagreement with the provisional row, instead of
  failing later in a parse. A false declaration then changes nothing the guest could not already do to itself: the
  only data it can get answered is layouts of a measured row, every decoder is bounded by the declared payload, and
  the cross-checks above refuse the rest.

### 6.4 Tests that would pin it

1. **Declared-version fuzz** (`detect` quantified over inputs): for random byte arrays, every mutation of every
   served tag's canonical string (extra/missing digit, leading zeros, trailing space, `-x` suffix, embedded NUL
   then digits, non-UTF-8, 255-byte unterminated, all zeros, `0xFF` fill), and cross-product with random branch /
   changelist / pair: the result is `Ok(row)` **only if** the bytes before the first NUL equal
   `row.to_string()` byte for byte (or the twin rule's exact strings); otherwise a named `Refusal`. Never a different
   branch's row, never a panic, never a "nearest" row. The failing-first negative control: the current
   `ReportedDriver::decode` + `parse` fails the "byte-exact" property on `"580.65.6"`.
2. **Row-equivalence**: for every served tag T, `detect(fn1_of(T)) == T`, and for every pair of distinct served
   tags `detect(fn1_of(T)) != U`.
3. **Pin**: `Some(pin)` with `detect != pin` refuses the boot; `None` never changes the row; the pin equal to the
   detection is a no-op.
4. **Pre-fn-1 witness**: a guest whose init-args shape (580 four-field vs 595 nine-field) disagrees with the
   provisional row is refused by name at publish, not later; the oracle (`drive`) is extended with a guest whose
   identity strings and pair disagree to prove the whole handshake still ends in a named failure rather than a
   mis-selected row.
5. **A firmware-section measurement** (only if 6.2's gap is closed): the section walker is bounded (header +
   `e_shnum` capped + one string), fuzzed against malformed ELF, and a `.fwversion` that disagrees only vetoes.
