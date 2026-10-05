# Windows E: class 0xb297 is on the recorded StartDevice failure path

**STATUS: RESEARCH, 2026-10-05.** The private live dump's NVIDIA journal restores an
internal assertion chain that the ordinary Microsoft stack does not preserve.
Together with the pinned retail driver's branch instructions and Kayfabe's RPC
trace, it identifies the refused `RmAlloc` of class `0xb297` as a direct failure
on this Windows E initialization path. The class's meaning and safe implementation
are still unknown. This is not permission to return success for an unknown class.

This corrects the preliminary suspicion that the earlier golden-context
`GPU_PROMOTE_CTX` refusal was the direct StartDevice blocker. That operation does
fail and leaves saved assertions, but execution continues into the later Windows
initialization chain described below. No golden acceptance or product policy was
changed by this investigation.

## Inputs and bounds

Windows E used Kayfabe `e71e4a8b`, NVIDIA Windows 580.88, and the pool/native-timer
experiments. Its GSP evidence is in
[`probe-e-initial/qemu.log`](../../../../traces/windows_pool_20261005/probe-e-initial/qemu.log).
The trusted official driver examined offline was `nvlddmkm.sys`, SHA256
`31c79cce80b573e21647ace8d1a2b65697f992e7f86196f89e30af477b1bd47c`.
All offsets below are **RVAs in that exact file**, not per-family product constants.

The successful signed KD `.enumtag` operation yielded one 80842-byte tag with GUID
`{270A33FD-3DA6-460D-BA893C1BAE21E39B}`. Its raw SHA256 is
`738e4640ca509db99f6023664385263c9134ea56f0f2fec375f864305aa3ff22`.
The raw dump, tag bytes, absolute kernel addresses, full decoded journal and any
unreviewed debugger output remain private. The tag's outer record table is not
decoded or assigned invented meanings.

At tag offset `0x1d68`, `NVCD_HEADER` has the public `NVCD_SIGNATURE` and
`GUID_NVCD_DUMP_V1`. Its declared length is 73315 bytes, while only 73314 bytes
remain. The final record header contains three of its required four bytes. The
decoder reports this truncation and does not invent a missing byte, validate the
whole-dump checksum, or claim a complete outer dump.

The first record is independently complete: group `RmGroup`, type `RmProtoBuf_V2`,
eight-byte `RmProtoBuf_RECORD`, followed by all 68779 declared protobuf bytes.
The compiler-derived descriptor closure decodes one `NvDebug.NvDump`, including
44 DCL messages, 21 assertion records and one bugcheck record. One field,
`Dcl.DclMsg.324`, is unknown to the selected public schema and retained only as
length/hash. There is no claim to understand every Windows-private field.

Schema source: OGKM tag `580.65.06`, source commit
`307159f2623d3bf45feb9177bd2da52ffbc5ddf9` (annotated tag object
`8032cb60785ff04a49698fbe1fac8781bd37b0d6`). `nvcd-schema.c` compiles the actual
`NVCD_HEADER`, `NVCD_RECORD`, `RmProtoBuf_RECORD`, `PRB_*` constants and generated
`g_*_pb.c` descriptor objects. No regex or captured byte layout constructs the
schema. Resulting `schema.json` SHA256:
`47aae92da4e4872693b399f75ceff2c66ff01b0a519d8532c07eee61f609fc42`.
This is a research decoder for that schema, not a new product compatibility claim.

## Recorded causal chain

The saved journal's caller addresses were rebased using KD's loaded module range;
only the resulting file-relative offsets appear here. The ordinary KD stack
already establishes `dxgkrnl!DpiFdoStartAdapter` and live dump `0x1b0`, arguments
`2`, `0xc000009a`. NVIDIA's journal supplies the missing internal branch sequence.

| Exact-driver RVA | Recorded branch/call fact | Implication |
|---|---|---|
| `0x19c2d77`, `0x19c2d7f` | Fifth allocation-wrapper argument is class `0xb297`; call targets `0x1a12530` | Identifies the resource requested by the failing caller |
| `0x1a1266c`–`0x1a12682` | Wrapper invokes RM, tests returned status, journals at return address `0x1a1267d`, and returns whether status was zero | This saved assertion is an allocation failure, not an inferred RAM shortage |
| `0x19c2d84`–`0x19c2de7` | False result follows the error branch and returns false | The first `0xb297` allocation is required by this object's initialization |
| `0x19c2f77`, `0x19c4f72`, `0x19c5093` | Saved callers follow the false-return chain; `0x19c5093` is the next saved assertion | Failure propagates rather than being ignored as an optional feature |
| `0x1a6b277`, `0x1a6b160`, `0x19eb82a` | Remaining saved callers lead to the outer initialization boolean | Connects the inner allocation failure to the StartDevice setup routine |
| `0x19eb82a`–`0x19eb83b` | The false branch explicitly assigns `0xc000009a` | This explains the live dump's `STATUS_INSUFFICIENT_RESOURCES` return |
| `0x1965e6f` | Later saved assertion on the negative-NTSTATUS cleanup path | Consistent with the caller unwinding initialization |

The independently captured GSP request is `RmAlloc class=0xb297`,
`client=0xc1d00002`, `parent=0xff030000`, `handle=0xff008050`, `result=0x56`.
The parent is the guest's subdevice object, not a GPU channel. The binary then
continues through cleanup. No claim about a hardware engine follows from the
class number's suffix. A future implementation needs source/behavior evidence
for allocation parameters, subsequent controls/methods, lifetime and isolation.

## Earlier assertions and the golden-context ruling

OGKM `journal.c:1792–1855` defines the assertion fields. At `:2460–2461`, generic
`rcdbRmAssert` stores level zero, while `rcdbRmAssertStatus` stores the actual
NV_STATUS. `nvstatuscodes.h:115` defines `0x56` as `NV_ERR_NOT_SUPPORTED`.
Windows-layer level-one assertions use a different wrapper; they must not be
mislabelled as NV_STATUS `1` from the journal field alone.

The earlier saved status assertion at `0x1abb0a` (Windows source line2582)
follows `AllocWithHandle(hObject=0xbaba0046,parent=0xbaba0045)`. Its constants and
call sequence match public `kgraphicsCreateGoldenImageChannel_IMPL`,
`kernel_graphics.c:2098–2500`, especially `:2482`. The corresponding GSP
`GPU_PROMOTE_CTX` is refused before the engine object's allocation reaches GSP.
The next assertion at `0x39d558` matches a failing scheduling callback. These are
real failures, but not the final branch that returns `0xc000009a` above.

Public `gpu.c:3437–3438` normalizes `NV_ERR_NOT_SUPPORTED` during post-load; Linux
guest workloads have already survived the corresponding golden refusal. The
[owner's promote ruling](../../../../docs/OWNER_RULINGS.md) permits bookkeeping
emulation because a host twin owns the real context. The current
[P5 implementation boundary](../../../../docs/design/V3_P5_PORT_MAP.md) explicitly
keeps the unbirthed golden kernel-GR channel refused. Any extension must retain
typed ownership, exact ABI/entry validation, no guest-address dereference, and no
forged completion of GPU work. This evidence does not justify widening it.

Other matched assertions: `0x302c74`/`0x302d2f` follow shared-data controls
`0x20800afe`/`0x20800aff`; late cleanup assertion `0x303f09` follows
`GR_SET_FECS_TRACE_HW_ENABLE` (`0x20800a37`). Their names come from public
`ctrl2080internal.h`. Merely counting unsupported commands would not distinguish
these paths from the required `0xb297` allocation.

## Reproduction and validation

Build and decode on the trusted controller; never execute a schema emitter built
from an unreviewed rental checkout:

```sh
bash tools/windows-debug-capture/build-nvcd-schema.sh /path/to/ogkm-580.65.06 /tmp/new-nvcd-schema
python3 tools/windows-debug-capture/decode-nvcd.py \
  /tmp/new-nvcd-schema/schema.json /private/analysis.stdout /private/new-decoded.json \
  --offset 0x1d68 --enumtag-guid 270A33FD-3DA6-460D-BA893C1BAE21E39B
python3 tools/windows-debug-capture/test_nvcd.py
```

The decoded JSON is created exclusively with mode0600 and never overwritten.
Raw byte fields become length/hash pairs, but strings and integer fields can
still contain private information; **the JSON remains private**. File/depth/field/
record/varint/length bounds are explicit. Unknown wire kinds and incompatible
known-field wire shapes fail closed. The opaque outer tag offset is supplied
explicitly, not guessed as an ABI fact.

The compiled schema build passed; the reusable decoder reproduced the earlier
independent prototype's full decoded protobuf and unknown-field census exactly.
Eleven adversarial tests passed, covering changed synthetic C offsets, truncation,
overflow, wrong wire shapes, packed budgets, recursion, unknown fields, checksum,
nonadvancing records and duplicate/short KD tags. No GPU was used by this decoder.
