# GfxP pool: Linux source counterpart and stub boundary

**STATUS: RESEARCH, 2026-10-04. Source supports a bounded emulation experiment;
the complete sizing algorithm and Windows acceptance remain unvalidated.**

There is a Linux source counterpart, including this exact API. A missing CPU
implementation is not evidence that Linux cannot route it to GSP. The
[existing routing correction](README.md#correction-gr_gfx_pool_query_size-is-not-proved-linux-inaccessible)
and kernel-caller requirement apply. The ordinary-user probe's permission error
does not test an authorized kernel caller.

OGKM 580.65.06, revision `307159f2623d3bf45feb9177bd2da52ffbc5ddf9`, provides more
than just the 40-byte query structure:

| Source | What is actually public |
| --- | --- |
| [ctrl2080gr.h:1298](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080gr.h#L1298) | Query input and output meanings; initialize/add/remove controls, memory handle/offset/size, slot lists, and quiescence requirements. `MAX_SLOTS=64` bounds the add/remove array; it is not by itself proof of a global query maximum. |
| [kernel_graphics_object.c:600](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/nvidia/src/kernel/gpu/gr/kernel_graphics_object.c#L600) | The firmware-client 3D promotion list includes both `GFXP_POOL` and `GFXP_CTRL_BLK`. The list is used when client-RM context-buffer allocation is enabled. |
| [kernel_graphics_context.c:329](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/nvidia/src/kernel/gpu/gr/kernel_graphics_context.c#L329) | Global pool/control-block IDs map to engine context-property IDs. The same file manages per-channel preemption/spill/beta/pagepool/RTV buffers and treats pool initialization separately from control-block initialization. |
| [Pascal dev_ctxsw_prog.h:30](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/inc/swref/published/pascal/gp100/dev_ctxsw_prog.h#L30) | Control-block offsets for preemption, spill, CB, pagepool, stride, sizes, slice arrays and maximum slices, through offset 0x54. These are actual published layout fields, not inferred from a capture. |
| [Turing dev_ctxsw_prog.h:26](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/inc/swref/published/turing/tu102/dev_ctxsw_prog.h#L26) | Additional RTV size/offset fields at 0x58, 0x5c and 0x60. |
| [Blackwell dev_ctxsw_prog.h:31](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/307159f2623d3bf45feb9177bd2da52ffbc5ddf9/src/common/inc/swref/published/blackwell/gb100/dev_ctxsw_prog.h#L31) | Repeats the base pool-control offsets; this is not a complete cross-family inheritance proof or a sizing formula. |

Therefore “the pool has no Linux equivalent” or “none of its internal layout
is public” would both be too strong. The known common purpose is storage and
bookkeeping for graphics preemption. Normal preemption buffers and the pooled
variant are related, but not interchangeable layouts.

The owner's existing twin-satisfied context-promotion policy makes a local
virtual pool a reasonable experiment: the guest's pool need not automatically
become the physical host twin's preemption storage. Query success is a resource
description, not a GPU completion. Waiting for another native capture is not a
prerequisite to testing that hypothesis.

However, the published offsets alone do not establish the exact size/alignment
formula, and one must not turn the last known field offset into a claimed full
structure size. Nor does the promotion ruling prove that Windows never reads
the control block itself. A query stub needs explicit, bounded, consistent
virtual storage requirements, followed by validated guest-local initialize,
add/remove and bind behavior if those calls arrive. Guest handles/addresses
must never become VMM pointers or raw host RPC payloads. Actual host preemption
and completion still need real execution-plane support; a local bookkeeping
success must not pretend the host performed them.

The experimental acceptance criterion is a recorded Windows run past the old
query/teardown point, with the next request bodies and failures retained. This
would establish progress for that run, not cross-die correctness or complete
Windows support. No query stub has been implemented or tested by this source
note, and no per-die constants were copied from a capture.
