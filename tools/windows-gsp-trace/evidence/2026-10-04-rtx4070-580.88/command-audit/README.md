# What the Windows capture establishes

**STATUS: RESEARCH, 2026-10-04.** All 4,535 retained records were checked against
the actual Windows branch (`v3-windows`, `c50fad9a`), including command IDs
inside the documented deferred-API wrapper. This is an offline source audit;
Windows has not completed initialization through Kayfabe.

[Full inventory](audit.md) · [Machine-readable audit](audit.json) ·
[Reproduction and methodology](../../../audit/README.md)

## Coverage, with no generic passthrough counted

All five observed RPC function IDs are known: FREE, RM_CONTROL, RM_ALLOC,
CONTINUATION_RECORD and POST_EVENT. Their contents expose the gaps:

| Direct control IDs | Count |
| --- | ---: |
| Display, channel or init-table handler | 22 |
| Conditional authored host-fact query | 3 |
| Empirical identity / capture-derived constant reply | 2 |
| No specific handler | 102 |
| Total | 129 |

Of the 102 without a handler, **92 have at least one successful native reply**.
This is a backlog to classify, not evidence of 92 independently fatal blockers.
It includes physical display/audio management, telemetry, optional capability
queries and graphics state. Twelve IDs have at least one native error reply
(`NV_ERR_NOT_SUPPORTED`, 0x56, or `NV_ERR_OBJECT_NOT_FOUND`, 0x57); two of those
also succeed for other observations. A blanket refusal would not reproduce the
latter behavior either.

The local OGKM 580.159.04 SDK resolves **64 of 129** control IDs by name. Of the
remaining 65, 55 enter the GSS-legacy rule, two enter the BinAPI rule, and eight
have neither a name in these SDK headers nor an allowlist entry. This is not a
claim that all 65 are undocumented everywhere. All 20 allocation classes have
published names; only 17 have allocation decoders in this branch.

## Concrete compatibility work found

1. **Deferred graphics context operations.** The driver successfully allocates
   `NV50_DEFERRED_API_CLASS` (0x5080) twelve times and submits eight
   `NV5080_CTRL_CMD_DEFERRED_API` controls (0x50800101). Four contain
   `GPU_INITIALIZE_CTX` (0x2080012d), four `GPU_PROMOTE_CTX` (0x2080012b).
   Kayfabe has neither the class nor wrapper implemented. Its direct promotion
   handler does not implement scheduling/executing the deferred operation.
   The successful wrapper replies establish acceptance, not later execution.
2. **Preemption context buffers.** Eight successful
   `GR_CTXSW_PREEMPTION_BIND` (0x20801211) requests have no handler. The public
   SDK describes the mode and preemption-buffer addresses, including context
   pool/control buffers. This is concrete work beyond answering the pool-size
   query; the addresses alone do not establish that query's sizing formula.
3. **Channel setup.** `FIFO_GET_LATENCY_BUFFER_SIZE` (0x0080170e, twelve requests)
   and `FIFO_SET_CHANNEL_PROPERTIES` (0x0080170f, two requests) succeed natively
   and have no handler. These are named controls with public source to study.
4. **Known ID, unsupported request variant.** All ten Windows requests to clock
   query 0x2080a028 fail the existing authored-input gate: Windows supplies ten
   domains where the recorded Linux rows describe one. Its request selectors
   also vary. The single 0x2080a026 request uses input 0x800 at byte 0 and
   0x10000000 at byte 8, while the current row requires 0x400 and 0. The sizes
   match, but none of those eleven requests can use the existing cached answer.
   The 0x2080a084 request does match its row, still conditional on host facts.
5. **Other allocation gaps.** `GF100_SUBDEVICE_INFOROM` (0x90e7) is allowlisted
   but lacks a decoder. `NV40_I2C` (0x402c) is deliberately refused because a
   guest does not own the physical board bus; observing it is not authorization
   to forward physical I2C operations. Virtual display behavior and optional
   queries need separate treatment.

The original failed Windows-through-Kayfabe run also already established
`FB_GET_INFO_V2` index 1 and `BUS_GET_INFO_V2` index 24 refusals, plus missing
`GPU_GET_CLASSLIST_V2`, compression-store info and other init queries. Those
findings are in `v3-windows:docs/design/V3_WINDOWS_DISCOVERY.md` at the audited
revision. Existing command IDs therefore cannot be counted as complete
implementations simply because their outer structs are known.

## The original startup blocker remains unresolved

**GR_GFX_POOL_QUERY_SIZE (0x2080121f) is absent.** The first retained queue
sequences are request 2707 and reply 2710 with unknown prefixes. One later
reply record is also missing. Zero recorder FIFO drops does not recover the
missing prefix. Seven request heads and six reply heads are fragmented; their
IDs are audited, but this inventory does not reassemble the parameter bodies.

The original failure occurred immediately after refusal of that query. It is
the leading blocker hypothesis, not yet proven causal by implementing it.
Its public layout is known, but we still lack a successful native reply and a
source-backed rule that works across dies. Linux RM compiles that control out,
so forwarding to the Linux host is not a solution.

Next evidence needed: retain earlier initialization or deliberately trigger a
fresh initialization while the recorder is already ready, then capture the
query inputs/outputs. Repeat on the RTX 3060 and compare with the RTX 4070
before deriving a family-independent rule. The prepared D3D11 probe has not
been executed. WDDM paging, submission/doorbells and TDR behavior through
Kayfabe remain unvalidated; this VFIO reference capture proves none of those.

## Verification

The existing strict decoder validates the complete export and its source
hash. All RPC/control/class counts were independently recomputed from the
text export and matched the audit. The Rust registry helper builds against
the clean pinned Windows branch. Eleven existing decoder tests pass.
Only source and text evidence are saved; no product behavior changed.
