# Windows J: the post-registration StartDevice failure

**STATUS: RESEARCH, 2026-10-05.** Reviewed private journal plus bounded offline
analysis of the trusted Windows 580.88 retail driver. No product modification,
feature admission, GPU execution, or success stub is made by this investigation.

At product revision `fdc991c9`, Windows J accepted both previously failing fn4
SYSRAM registrations and advanced from 213 to 363 traced RPC records (215 to 365 serviced messages). It still
reported Code 43 and no GPU channel births. Its fresh live dump recorded
`StartDevice` failure `STATUS_INSUFFICIENT_RESOURCES` (`0xc000009a`). This status
is an explicit generic error assignment in the driver, not proof of exhausted RAM.

The journal's final failing chain is a **display buffer precondition failure**.
The static path strongly identifies a zero-sized TMO buffer because Kayfabe
advertises TMO absent. The earlier refused audio-codec allocation is explicitly
optional. Scheduling control `0x20801111` is not the root identified by this chain.

## Bounded L follow-up: the earlier helper now passes

**2026-10-05:** Experiment L, product`b431aeaf`, advertises the diagnostic TMO
capability while refusing **all display-channel methods before execution**. It
still reports Code 43, no channels and zero display methods. Its 365 RPC records
differ from J's 363 only by two additional`0x50700117` cleanup calls. This is
construction progress, not TMO operation or rendering support.

The new journal assertion 35 begins`16e97f4 → 16b4f2a → 1615220 → 1614aed →
1959907 → 1965547`. At`16e9675`, the driver called constructor`16f03c0` for the
current display-window descriptor. At`16e9689` it tests the constructor's success
byte at input+`0xa8`; zero branches to`16e97ef`, whose assertion return address
is`16e97f4`, then returns false through the already documented fatal chain.

This block is reachable only after **all three earlier calls to`169d3f0`
succeeded**, including J's failing second/TMO descriptor at`owner+6238`.
The changed leaf therefore confirms that the previous allocation helper passed
under this diagnostic. It does not prove that the advertised capability is safe
for normal use.

Constructor`16f03c0` clears the success byte at`16f0484`. Its initial checks
require nonzero DWORDs at input+0 and+4, input+`0x10` at least
`input[0]*8+0x28`, and input+`0x14` between 1 and 8. It also validates the input
buffer-count/flag/pointer combinations, allocates CPU storage and initializes it.
Only its success path at`16f05e9` sets input+`0xa8` to1. The caller's new assertion
alone does **not** reveal which of these earlier checks or allocation failed.
No live local values were recovered, and no further condition is inferred in
this bounded follow-up. The next investigation starts at this constructor.

## Pinned input and reproduction

The inspected `nvlddmkm.sys` SHA256 is
`31c79cce80b573e21647ace8d1a2b65697f992e7f86196f89e30af477b1bd47c`.
All addresses below are RVAs in this exact file. They are research coordinates,
never product layouts, class definitions, or captured per-GPU data. Raw dumps,
loaded addresses, private journal bytes and guest memory remain private.

[The inspection script](../inspect-startdevice-j.py) verifies that exact hash,
reads the PE image base, and optionally asks trusted local `objdump` to disassemble
20 fixed ranges of at most 256 bytes each. It never executes the NVIDIA driver:

```sh
python3 tools/windows-debug-capture/inspect-startdevice-j.py \
  --driver /path/to/trusted/580.88/Display.Driver/nvlddmkm.sys --disassemble
```

The private journal was decoded with the existing compiler-derived NVCD schema.
Assertions35–47 supplied the final chain below; the decoder's record numbers
include common metadata records between assertions. This record numbering is
local to this dump, not a protocol constant.

## Direct propagation to the fatal status

| Journal/call RVA | Checked behavior in the pinned driver |
|---|---|
| `169d836` | Cold failure arm of helper`169d3f0` returns false. It has not reached its allocation/map operations. |
| `16e93b2` | Tests the helper result for descriptor`owner+6238`; false jumps to`16e9530`, which returns false. |
| `16b4f2a`, assertion`16b4f33` | Tests the display initializer result and returns false. |
| `1615220`, assertions`1615268`/`1615285` | Tests the child initializer, logs failure, performs cleanup and returns false. |
| `1614aed`, assertion`1614af6` | Propagates failure through display setup. |
| `1959907`, assertion`1959910` | Propagates failure from that display setup. |
| `1965547` | Tests that result and branches to`1965bc6`. |
| `1965bc6`, assertion`1965bd1` | Assigns`r15d=0xc000009a`, then follows the error cleanup/return path. |

The innermost helper has three guards before making any resource call:

- descriptor pointer must be non-null;
- descriptor DWORD at+4 (allocation byte size) must be nonzero;
- `(descriptor handle at+50 == 0)` must equal bit2 of owner byte+112.

Any failure reaches the same cold arm. The caller passes a statically non-null
`owner+6238` and has just written its nonzero handle at`owner+6288`. Immediately
before it, the same helper succeeded on`owner+6190`, whose handle is also nonzero,
using the same owner/flag. The helper contains no direct write to owner byte+112.
That strongly distinguishes the zero-size guard from a handle/flag mismatch.
The journal itself has no snapshot of that live local DWORD, so the size value is
an inference from the code and advertised capabilities, not a recovered local.

## Source-defined capability responsible for that size

At`16e8efa`–`16e8f0c`, the driver calls vtable+e00 and skips the second buffer's
size contribution when false. The successful arm accumulates a rounded entry size
into`owner+623c` at`16e8fc5`. Helper`16f0690` calculates that entry size as
`align_up((entry_count+5)*8,256)` in this exact driver. This formula is not an
implementation requirement or an accepted guest size limit.

All five static vtables found with initializer`16e7d90` at+a80 use helper`208c0`
at+e00. It passes byte offset`(0x3c+window)*32 = 0x780+window*0x20` to the
capability getter and returns bit20. Concrete getters`1ed30` and`1ed70` treat the
argument as a byte offset: `(offset>>2)*4` indexes a DWORD in the capability array.
It is **not a bit index**, and the index is a **display window**, not a display head.

Public OGKM 580.65.06 commit`307159f2623d3bf45feb9177bd2da52ffbc5ddf9`,
[`clc773.h:503–530`](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/sdk/nvidia/inc/class/clc773.h#L503),
names that exact word `NVC773_PRECOMP_WIN_PIPE_HDR_CAPA(i)` and bit20
`TMO_PRESENT`. Existing generated display tables already contain that field.
At product`fdc991c9`, [`caps.rs`](../../../crates/kf-disp/src/caps.rs) explicitly
advertises no TMO and omits this bit while authoring CSC/LUT capability bits.
The same static source correspondence exists in C573/C673/C773/CA73 headers;
this observation does not prove every driver handles absent TMO identically.

The resulting incompatibility is that this Windows initializer unconditionally
tries to allocate the second descriptor after accumulating size only for
TMO-present windows. In Kayfabe's advertised configuration every contribution is
skipped. This is a capability/initialization-contract problem; accepting another
unrelated RPC would not satisfy the failing guard.

Advertising TMO is **not authorized by this finding**. The source-defined TMO
methods and their actual processing/storage/completion obligations must be
implemented or a supported driver path that avoids the feature found. A captured
bit or fabricated nonzero allocation size is not a valid v3 solution.

## Earlier assertions are not interchangeable with this cause

Assertion 21's allocation-wrapper stack leads through`16158e2`. Its callsite
explicitly requests class`0x90ec`; public`cl90ec.h` names it`GF100_HDACODEC`.
If allocation fails, the branch skips recording the handle and continues at
`16158f0`. Its caller at`1959373` ignores that child return, executes another
helper and returns true. It is therefore not this dump's fatal startup cause.

Assertions at`19fe082`, `19460d2`, `1947d1c`, `1769a29`, `17884de` and`304308`
occur earlier than the final chain. Their individual contracts have not been
fully audited here; their presence alone does not identify a fatal RPC. The final
journal plus the explicit status-assignment branch provides the narrower causal
result above. No claim of complete compatibility follows from this single run.
