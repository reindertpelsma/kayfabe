# Control 0x20801111: bounded scheduling review

**STATUS: RESEARCH, 2026-10-05.** Static review of authenticated proprietary
Linux 535.309.01 and 610.43.02. No control is implemented or permitted by this
note. No GPU, driver binary or extracted object was executed for this review.

**Zero entries do not establish a no-op contract.** Both inspected dispatchers
call scheduler preparation and submission methods even when the index count is
zero. The concrete 535 software-scheduler implementation can clear an existing
software list, mark a runlist dirty, select a different runlist buffer, and enter
submission/preemption paths with no new channel entries. Whether any particular
empty request can be emulated requires its complete payload and established
prior state; absence of guest GPU-channel births alone is insufficient proof.

This is also not established as the next Windows initialization root cause:
the H experiment's `ALLOC_MEMORY` refusal precedes its `0x20801111` refusal.
The latter may be an error cleanup request. Exact payload capture and memory
allocation analysis take priority over making the scheduling call succeed.
[H evidence at c24933ed](https://github.com/reindertpelsma/kayfabe/blob/c24933ed/traces/windows_pool_20261005/probe-h/qemu.log).

## Provenance and scope

The [parent note](README.md) pins the installers to NVIDIA's HTTPS checksums,
records the extracted objects' hashes, and compiles the matching public metadata
layouts. [scheduling-symbols.json](scheduling-symbols.json) identifies the reviewed
functions by symbol, section offset, size, code hash and relocation edges.
The function-byte hashes cover unrelocated ELF text, not runtime addresses.

| Layer | Linux 535.309.01 | Linux 610.43.02 | Review extent |
|---|---|---|---|
| Subdevice control | `_nv046624rm`, `.text+0x43ce10` | `_nv055811rm`, `.text+0x4b1cf0` | Full argument dispatch and ownership checks |
| Scheduling dispatcher | `_nv044643rm`, `.text+0x443720` | `_nv053921rm`, `.text+0x4967c0` | Argument flow; zero-count branches; both virtual calls |
| Concrete software scheduler preparation | `_nv044666rm`, `.text+0x447680` | Not resolved in this follow-up | State changes and flag-controlled list reset |
| Concrete software scheduler submission | `_nv044667rm`, `.text+0x449e00` | Not resolved in this follow-up | Buffer build, selection, submission/preemption branches |
| Hardware-format buffer build | `_nv013280rm`, `.text+0x4496b0` | Not resolved in this follow-up | Offset-4 argument use and empty-build state changes |
| Conditional asynchronous helper | `_nv023008rm`, `.text+0x458e30` | Not resolved in this follow-up | Offset-32 value stored in a per-engine pending record |

The concrete scheduler is one native policy implementation, not every possible
GPU/policy combination. In 535, `_nv044631rm` constructs policy-dependent
classes. NVOC initialization `_nv003173rm` installs the inherited preparation
and submission wrappers `_nv007675rm` and `_nv007676rm`; their pointer adjustment
and method slots resolve to `_nv044666rm` and `_nv044667rm`. Another policy class
has not-supported methods in the corresponding slots. Native policy support
must not be inferred solely from the presence of the control registration.

## Parameter fields

The following names describe observed behavior; they are **not invented public
ABI names**. Byte offsets are in the 40-byte control parameter structure.

| Field | Bounded meaning / observation | Confidence and limit |
|---|---|---|
| +0, u32 | Software-runlist resource handle; own-client lookup, internal class check and matching Subdevice parent | Both control handlers |
| +4, u32 | Group ordinal used while building the hardware-format runlist. When equal to the zero-based group counter, the current hardware-entry position is saved | 535 builder at `0x44999f..0x4499b6`; not a generic operation selector. Exact public name and sentinel rules unknown |
| +8, u32 | Memory handle resolved in the calling resource hierarchy; the Memory object's CPU mapping supplies the records and index array | Both handlers; validation still occurs when count is zero |
| +12, u32 | Number/bound of 12-byte records; selected 16-bit indices are checked against it | Both dispatchers; not proof that all nested record fields are audited |
| +16, u16 | Byte offset from that owned Memory mapping to the index sequence | Both handlers |
| +20, u32 | Number of selected indices | Zero skips the per-index iteration, not scheduler preparation/submission |
| +24, byte | Requests list rebuilding: preparation clears the software list, marks it dirty and selects the list to build; dispatcher visits the selected records only when this flag is nonzero | 535 concrete implementation; no validated public flag name |
| +25, byte | Selects the staged-list path; changes which saved runlist ID is used, and nonzero skips the immediate submission/preemption block | 535 concrete implementation; does not prove the whole call is inert |
| +26, byte | Forces dirty-state handling when the requested list equals the currently selected list; also affects the submission/preemption branch | 535 concrete implementation; exact intended API guarantee unknown |
| +32, u64 | Nonzero conditionally selects an asynchronous branch and is stored in a per-engine pending record | Potential event/pointer-bearing field. Complete type, lifetime and notification contract unproven; must never be forwarded as an opaque host value |

Padding/reserved bytes and accepted noncanonical boolean values have not been
specified by a public control declaration. This table is research evidence,
not a complete parser specification or forwarding audit.

## Why an empty list can still do work

In 610 the dispatcher calls virtual methods at offsets `+0xb8` and `+0xd0`
before/after its optional record loop. The branch at `0x4968cc..0x4968cf` skips
the loop at zero count, returning to the second virtual call. The analogous
535 branch is `0x443820..0x44382a`, returning to the second virtual call at
`0x4437a3`. This is independently visible in both inspected versions.

In the resolved 535 implementation:

- Preparation at `0x4476cc..0x44771b` uses flag +24 to clear the selected list
  with `_nv034640rm`, mark it dirty and record its selection. The clear helper
  empties its linked-list head/tail/count and releases nodes.
- With flag +24 set, submission invokes the buffer builder even with no newly
  selected records. The builder resets entry counters, selects the other buffer,
  acquires its mapping, and commits the selected buffer and counts. An empty
  list reaches `0x449a90..0x449ad6`; it does not mean "leave all state unchanged."
- With flag +25 clear, submission can enter scheduler virtual methods and a
  conditional asynchronous helper. Flag +26 and whether the list is already
  current affect those branches. An empty list may be used to remove/preempt
  previous GPU work; acknowledging it without honoring that effect could forge
  a scheduling/completion guarantee.
- +32 reaches `_nv023008rm` only on a conditional branch, but when nonzero that
  helper saves it in a pending per-engine record at `0x458eb6..0x458ec4` and calls
  further hardware-related methods. Its absence removes that particular token
  path; it does not suppress the rest of submission.

There is a relevant **public analogue**, not a proved identification:
`NV2080_CTRL_FIFO_DISABLE_CHANNELS_PARAMS.pRunlistPreemptEvent` is documented as
a KEVENT handle for asynchronous hardware-runlist preemption; its implementation
restricts non-null values to kernel clients. This reinforces the need to audit
the +32 field as potentially pointer-bearing, but does not establish that
`0x20801111` uses that exact declared member or the same notification mechanism.
[Public declaration](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080fifo.h#L330),
[public privilege check](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/nvidia/src/kernel/gpu/fifo/kernel_fifo_ctrl.c#L725).

No public `0x20801111` declaration was found in the earlier 11-tag header sweep
or in `git log --all -S'0x20801111'` for `ctrl2080fifo.h` in the local full OGKM
repository. Those are bounded negative searches, not proof that every public
source or an alternate equivalent is absent.

## Implications for Kayfabe

Keep `0x20801111` refused. There is presently no justified generic empty-list
success case. A future narrow emulation would need to establish the request's
meaning, memory ownership/bounds, supported flags, prior list state, and absence
of hardware/notification obligations. It must preserve guest-driver, host-driver
and GPU-family compatibility axes; these binary offsets are research locators,
never product tables. The call remains kernel-client-only by inspected metadata;
there is no unprivileged forwarding authorization.

Reproduce the provenance manifest after the data-only extraction described in
the parent note:

```bash
python3 tools/windows-debug-capture/inspect-runlist-scheduling.py \
  --object /path/535/nv-kernel.o_binary \
  --object /path/610/nv-kernel.o_binary > scheduling-symbols.json
objdump -dr --disassemble=_nv044666rm /path/535/nv-kernel.o_binary
objdump -dr --disassemble=_nv044667rm /path/535/nv-kernel.o_binary
objdump -dr --disassemble=_nv013280rm /path/535/nv-kernel.o_binary
objdump -dr --disassemble=_nv053921rm /path/610/nv-kernel.o_binary
```

The manifest validates input hashes and records symbol identities. It does not
automatically prove the semantic observations above. No raw proprietary bytes,
private Windows payloads or full disassembly are committed.
