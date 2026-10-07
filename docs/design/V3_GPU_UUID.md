# Per-VM GPU UUID (`gpu-uuid=`)

**STATUS: LIVE, 2026-10-08 (code and GPU-free tests; NOT yet run on hardware or in a guest).**
Branch `claude/gpu-uuid-per-vm-20261008`. Owner decisions that are still open are in §7.

Source of the design: the owner, 2026-10-08 (the problem) and 2026-08-08 (the shape, recovered
from their messages; the quotes were not independently re-verified). Code: `kf_rm::gpuuid`
(`crates/kf-rm/src/gpuuid.rs`), `kf_abi::gspstaticinfo::GpuGid`, `crates/kf-qemu/src/gpuuid.rs`,
`qemu/hw/misc/kf3/kf3.c` (ABI 22). Tests: `crates/kf-rm/tests/gpu_uuid.rs`,
`crates/kf-qemu/tests/gpu_uuid_property.rs`.

## 1. The problem, and what it is not

Orchestrators and cluster tools that key on the GPU id crash or misbehave when two VMs report the
same one. Before this change the guest UUID was a function of the chip row alone
(`StaticInfoPolicy::gid_for_board`: PCI device, subsystem vendor, subsystem, revision), so every VM
on a chip row reported the same UUID. This is a **collision** problem. It is not about hiding the
host GPU: the model and architecture the guest sees stay truthful.

## 2. Where the guest-visible UUID is produced (sites)

**MEASURED, 2026-10-08, `rg -i 'uuid|gid' crates/kf-*`:** there is exactly one producer of the
UUID on the way to the guest.

| # | Site | What it does |
|---|---|---|
| 1 | `GspStaticInfo::gid` → `encode_gsp_static_info` | the bench-layout `GET_GSP_STATIC_INFO` body (`StaticInfoPolicy::body`) |
| 2 | `encode_gsp_static_info_at` | the same field at the layout of the guest's driver version (`body_measured`, `body_measured_with`) |
| 3 | `StaticInfoPolicy::respond` | the fn-65 reply, the only way the bytes reach the guest |
| 4 | `served_policy` → `served_chain` | the chain the device installs; builds the policy from `BoardFacts` on every rebuild (`ReselectAtFn1` included) |

All four read **one** value, `BoardFacts::gpu_gid`, resolved once at realize and shared by the
`Arc<BoardFacts>` of every chain rebuild. `StaticInfoPolicy::gid()` exposes it. The test
`every_site_carries_the_one_declared_uuid_at_every_measured_version` drives sites 1-4 at every
driver version the matrix covers and asserts the same 16 bytes at that version's own
`gidInfo.data` offset, and that neither the chip-row value nor the host GPU's UUID appears
anywhere in the served body.

**INFERRED (not measured end to end in this session):** what the guest then shows
(`NV2080_CTRL_CMD_GPU_GET_GID_INFO`, NVML's `GPU-...`, `nvidia-smi -L`, UVM's GPU UUID,
`NV0000_CTRL_CMD_GPU_GET_UUID_FROM_GPU_ID`) is computed by the guest's own RM from site 3. The basis
is the header comment of `GpuGid` (`gpuGenGidData_FWCLIENT` is the only producer on a GSP client,
`ogkm-580 gpu_gspclient.c:152-159`) and the boots of 2026-08-08 in which a zero field failed
`RmInitAdapter`. Neither `0x2080014a` nor `0x00000275` is answered by any v3 link
(`rg -i '2080014a|GET_GID|UUID_FROM' crates/kf-*` finds only the capability table). The check that
would settle it is two guests on one chip row printing different `nvidia-smi -L` (§8).

**Not a site, but worth saying:** no v3 path carries a guest-supplied UUID back to the host (UVM's
`UvmGpuMappingAttributes` and `GET_UUID_FROM_GPU_ID` on a host client are not translated
anywhere). The day one does, it must map the guest UUID to the host's.

## 3. The property

`gpu-uuid=` on the `kf3-gpu` device (QOM string, `qemu/hw/misc/kf3/kf3.c`; Rust parse in
`kf_rm::gpuuid::GpuUuidMode::parse`):

| value | meaning |
|---|---|
| unset, `auto` | **default.** Stable across boots of the same VM, distinct across VMs (§4) |
| `random` | 16 fresh bytes from `/dev/urandom` per boot |
| `host` | the host GPU's own UUID, with a logged warning (two VMs on one GPU collide again) |
| `GPU-xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`, the same without `GPU-`, or 32 hex digits | explicit |

Refused at `Device::realize` before anything is opened (`Config::check`), by name: any other word
(`Auto`, `host `, `random\n`, ...), any malformed UUID text (wrong length, a dash in the wrong
place, a non-hex or non-ASCII byte, `0x`, a sign, trailing text, a NUL, input over 40 bytes), and
the all-zero value in every spelling (it is what the guest reads as "GSP static info not
initialized" and fails `RmInitAdapter` on). The refusal message gives the length of the offending
value, not the value.

`vm-id=` (QOM string): the VM identity `auto` hashes. Absent, QEMU's `-uuid` is used when given.
The all-zero UUID (QEMU's value when `-uuid` is not given) is no identity. A malformed `vm-id=` is
refused.

## 4. The `auto` construction (exact)

```text
digest = SHA-256( "kayfabe/gpu-uuid/auto/v1\0"   // 25 bytes, domain-separation tag
               || vm_id                           // 16 bytes
               || host_gid                        // 16 bytes
               || slot )                          //  1 byte
gid    = digest[0 .. 16]                          // refused if all-zero (2^-128)
```

* `vm_id`: `vm-id=`, else QEMU `-uuid` (§7, decision A).
* `host_gid`: the host GPU's UUID, asked of the host RM at realize with
  `NV2080_CTRL_CMD_GPU_GET_GID_INFO` (`0x2080014a`; index 0, flags `FORMAT_BINARY | TYPE_SHA1` =
  2; the reply must say `length = 16` and be non-zero). It is `HostFacts::host_gid`, with a
  `PROVENANCE` row, and is **derived from the host at every boot, never captured**. A host that
  does not answer yields `None`: `random` and an explicit value still work, `auto` and `host` refuse
  by name.
* `slot`: the device's guest PCI `devfn` byte (§7, decision B).
* Every field is fixed-width, so no input can be re-read as another and no length prefix is
  needed. A change of construction takes a new tag (`v2`); the tag is never edited in place. The
  value for one input set is pinned in `auto_is_the_documented_sha256_construction`, computed
  with Python's `hashlib`, outside this repository.
* SHA-256 is `kf_util::sha256` (dependency-free, FIPS 180-4 vectors in its tests); the workspace
  takes no third-party crates.

Nothing here is secret: anyone who knows the VM id and the host UUID can recompute the value. That
is fine for a collision-avoidance id. (It is not the TPM's rule in `OWNER_RULINGS.md`, which is
about secrets.)

## 5. What is implemented

* `kf_util::sha256`; `GpuGid::{parse, Display}` (`GPU-` text round trip, hostile-input refusals),
  `parse_uuid_text`; `kf_rm::gpuuid` (modes, `auto_gid`, `resolve`, `parse_vm_id`, `os_entropy`).
* `BoardFacts::gpu_gid` and `StaticInfoPolicy::new` using it (`None` falls back to the per-chip-row
  value, which is what every GPU-free test that does not declare one still gets).
* `HostFacts::host_gid` and its query/derivation/provenance row.
* `kf-qemu`: `Config::{gpu_uuid, vm_id, pci_devfn}`, `Config::check`, `gpuuid::resolve_for`, the
  realize log lines (`kf3: guest GPU UUID GPU-... (gpu-uuid=..., basis ..., devfn ...)`, plus a
  named warning for `host` and for `auto` without an identity), FFI ABI 22.
* The C device: `gpu-uuid` and `vm-id` properties; `-uuid` passed through; the device's `devfn`.
  **MEASURED 2026-10-08:** `kf3.c` passes `cc -fsyntax-only -Wall ...` with the build dir's own flags against
  QEMU 10.2.4 (no warnings). It was not linked or run.

## 6. What is not done

* **Hardware and guest verification** (§8): no QEMU or guest was started (the GPU belongs to
  another run).
* **The optional HMAC with an overridable key** (owner's design): `auto` is a plain domain-separated
  hash. Adding a key is a new tag and a property.
* **The user-settable GPU NAME** (owner's design; architecture/model stay truthful): untouched.
  `HostFacts::gpu_name` is still the host's own string. Remaining work: a `gpu-name` property,
  length/printable validation (`GpuName::new` exists), an `explicit`/`host` mode, and the same
  single-site rule (it is already one site: `StaticInfoPolicy::with_name`).
* UUID translation guest to host (§2): nothing needs it yet.
* `gpu-uuid=` is not exposed through any launcher script (`win_vm.sh`, `run_fast_guest.sh` pass
  nothing); `KF3_DEV_EXTRA="gpu-uuid=..."` works for the scripts that already take it.

## 7. Decisions for the owner

A. **Which VM identity.** Options: (1) QEMU `-uuid` (what is implemented, with `vm-id=` as an
   override) - standard, set by libvirt automatically, stable across restarts of the same VM;
   (2) only a new `vm-id=` property - explicit but every launcher must pass it; (3) the VM name
   (`-name`) - not unique, rename changes it; (4) the Windows launcher's SMBIOS UUID, which is
   the same thing as (1) if `-uuid` is passed to both.
   **No launcher in `scripts/` passes `-uuid` today** (MEASURED 2026-10-08: `rg -e '-uuid' scripts`
   finds no QEMU invocation), so with the default, every current lane takes the fallback below.

B. **No VM identity (`auto` with no `-uuid`/`vm-id=`).** Implemented: a RANDOM UUID for that boot
   and a named warning. It avoids the collision and never refuses an existing launch line, at the
   price of not being stable. Alternatives: refuse to realize (stable-or-nothing, breaks every
   current launch line); keep the old chip-row value with a warning (stable, but it is the
   collision this feature removes).

C. **The `slot` byte.** The owner's formula was `hash(vm id + host GPU UUID)`. Two kf3 devices in
   one VM can sit on the same host GPU (the per-card budget code allows it), and they would then
   collide inside the VM. `slot` (PCI `devfn`) separates them; the price is that moving the device
   to another slot changes its UUID (`gpu-uuid=<explicit>` pins it). Dropping it is one line in
   `auto_gid`.

D. **`auto` needs the host UUID.** If the host RM does not answer `GET_GID_INFO` (it is in the
   nvproxy allow-list, so it should), `auto` refuses to realize rather than silently hash without
   it. Alternative: hash `vm_id` and `slot` only. The control is an INFERRED non-privileged call:
   it was not issued to a real host in this session.

## 8. To verify on hardware (not run)

1. `scripts/bench/v3_gates.sh` and `build_kf3.sh` at this revision.
2. Two guests on one chip row, each with its own `-uuid` and default `gpu-uuid`: `nvidia-smi -L`
   prints two different `GPU-...`; each is the same after a reboot of that VM. The realize log
   shows `basis Auto` and the guest UUID, and never the host's.
3. `gpu-uuid=GPU-...` explicit: `nvidia-smi -L` prints exactly that value.
4. `gpu-uuid=host`: the guest prints the host's value, the log carries the warning.
5. A malformed value: QEMU refuses `kf3: realize refused: gpu-uuid= (N bytes) must be ...`.
