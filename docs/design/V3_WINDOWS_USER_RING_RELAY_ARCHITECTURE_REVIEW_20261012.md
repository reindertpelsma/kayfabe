# Windows user-ring relay: review against v3

**STATUS: RESEARCH, 2026-10-12.** Default-OFF worker diagnostic is now connected
and source-reviewed; ordinary Linux Passthrough unchanged. Exact4ec9f9a24 native
and Linux gates/fast/desktop passed. Windows1580 reached sign-in but refused a
prototype source-buffer cap before the benchmark. No Windows migration PASS.
The atomic three-mode dispatch remains a separate unconnected prototype;
inline CopyRing, no-API fast switching, vidmem source USERD and latency proof
remain open. The runtime scans on workers from birth, copies only intercepted
segments, and retains real HOST witness/GPfetch lifetimes. See the latest
[resume](../RESUME_CODEX_WINDOWS_20261011.md) and user-worker evidence.

Owner proposes initial inspection followed by fast forwarding while no deferred
API is installed. This is distinct from object allocation: five campaigns show
3015080 allocations but only3 successful bundle-registration channels. Metadata
may identify a sole software object without historical SET_OBJECT, so that binding
is not an absolute late-switch blocker. Opaque later hardware packet continuation
still needs a justified recovery/admission boundary; CPU exclusion alone does not
supply it. No measured compliant Windows cutover violation is asserted. A late
5080 allocation and a later API registration on an existing object are separate
admission events. Any fast path must account for both before returning success.

## Architectural judgment

The three policies follow v3's rationale if dispatch policy stays separate from
channel privilege and completion kind:

| Policy | Callback work | Why it fits |
| --- | --- | --- |
| Direct | Forward opaque native token through mapped MMIO | Preserve ordinary Passthrough and Linux's existing restricted space. |
| CopyRing | Bounded pre-created USERD/GP views, at most four descriptors, publish private PUT and native token | Stable private ring removes privileged native migration while small submissions avoid worker latency. Larger deltas and contention wake off-vCPU service. |
| Worker | Durable pending bit and conditional nonblocking wake | Parse immutable pushbuffers, author software actions and perform mapping outside the vCPU. |

The four-descriptor limit is a trusted scheduling budget, not a GPU fact or a
substitute for correctness. Exhausted private credits cause backpressure; they
never retire GPU work. Where native work exists, its completions remain real. Translated and
Emulated completion rules remain in authoritative channel metadata, not in a
three-mode bitfield. USER work remains USER even when a worker inspects it.

Copying ordinary user GP pointers does not intrinsically require parsing their
pushbuffers. Their native USER channel, physical-operand prohibition and own
restricted GPU address space enforce isolation. CPU inspection becomes necessary
when kayfabe interprets/replaces software actions; at that point mutable guest
pushbuffers cannot be trusted as the executed snapshot. Windows private command
mappings remain GPU read-only; private USERD is not GPU-VA mapped. Linux receives
no host-owned GPU mapping. Declared Windows identity and sysmem USERD are routing
facts, never proofs of authority.

Pre-created source views need complete-span and lifetime admission plus coherent
VA-manager publication. A callback cannot call a mapping API, native RM ioctl,
blocking lock or lease helper that takes a mutex. It may perform bounded memory
and MMIO accesses and conditionally request a nonblocking worker wake. MMIO and
ioeventfd must converge on the same dispatch implementation. Those runtime
requirements are not implemented by the atomic prototype.

## CPU cutover versus native cutover

The companion u64 prototype preserves the existing full token/stamp and completion
kind. Its own word holds policy (2 bits), exclusion/lifecycle/history/pending flags
(5), opaque host token (32) and allocation generation (25). Exhaustion refuses
reuse instead of wrapping. Metadata lives outside the word; retirement excludes
claimants and native cleanup must succeed before a replacement is published.
Callbacks use a fixed number of atomic operations, no retry loop. A failed claim
publishes durable pending work; the caller must use the existing wake protocol.
Direct contention has a separate off-vCPU token drain, never inspection conversion.
A delayed callback can cross Free/reuse: resolve and validate metadata only AFTER
claiming, against the returned ticket generation; never retain a pre-claim pointer.

A switch excludes new inline claimants and waits off-vCPU for the existing claim
to exit. Native publication history is marked BEFORE PUT/MMIO. Only a channel
that has never published native work can then complete the fresh handoff. The
prototype deliberately has no operation that clears a previously published
opaque-stream handoff. Failed cleanup, missing events and elapsed time never
authorize reuse or advancement.

MEASURED, public NVIDIA FIFO manual at open-gpu-doc commit
`9fdf5c4062007929d9f4e6cbad9c9771fe61b880`,
[dev_pbdma.ref.txt](https://github.com/NVIDIA/open-gpu-doc/blob/9fdf5c4062007929d9f4e6cbad9c9771fe61b880/manuals/ampere/ga100/dev_pbdma.ref.txt):

- Lines153–164 explicitly permit hardware compressed-method headers and arguments
  across GP segments; SYNC_WAIT can see the prior header as processed before its
  arguments arrive. Software-method splitting has additional restrictions.
- Lines839–847 describe remaining packet data count; a following word can be an
  argument, not a method header.
- Lines3570–3590 say SetObject is not required by engines, Host holds no subchannel
  object map, and engine class state can be shared across a TSG.

INFERRED: a GP boundary, idle channel or empty GET/PUT is not sufficient parser
state for starting inspection at that point. A nonempty witness blindly appended
to an opaque partial packet could become an argument. Existing consumption tests
use complete host-authored packets and do not prove this opaque-prefix case.
Packet, binding, persistent HOST-method and pending MEM_OP state would all need
continuity before a late conversion could be enabled.

MEASURED, [run1361 selected lines](../../traces/windows_matrix_20261011/gravity-vulkan/h1-channel-audit/run1361-selected-lines.txt): deferred-API object
allocation on native channel4e is log8315; scheduling8365; first recorded USERD
forward8437. Registration comes much later. INFERRED: choose worker inspection at
object-allocation admission if the exact channel has never published native work,
with old-claim exclusion before acknowledging the object. This could let ordinary
Windows channels use CopyRing while this Vulkan channel starts inspection before
its first native submission. A hostile late object allocation still needs a clean
refusal/gated path; this trace is not a universal ordering guarantee.

## Deferred notification candidate

MEASURED, public ogkm595.84: deferred_api.c247–250 enables bNotifyTrigger at object
construction; method_notification.c51–74 clears it when notifying. A source search
finds no ordinary rearm. Unix os.c1515–1524 posts an ordinary OS_EVENT without the
method/data/status payload; the extended callback retaining it is kernel-only
(event_api.c75–89). The public source does not expose the GSP caller/timing proof.

INFERRED: authoring an allowed own-space deferred invalidate might provide useful
measurement, but no repeatable per-trigger CPU acknowledgment gate is established.
It does not solve native parser handoff. This is separate from privileged native
FIFO invalidation/own-PDB queries, which were rejected from public source. No
privileged-host workaround or runtime notification probe was performed.

## Verification and remaining work

The same dispatch implementation is compiled with native atomics and Loom atomics.
Finite models exercise publication versus fresh switch, simultaneous publishers,
Free/reuse and metadata lifetime, delayed callback admission across replacement,
pending wake preservation, later PUT delivery and contended Direct last-ring service. They use no preemption/permutation/time cap. They do not prove
all possible program lengths, GPU/MMIO ordering, pointer-view validity, native
parser handoff, the surrounding eventfd protocol or an integrated VM.

Two deliberate broken variants falsify the cutover model: omit native-publication
history, or allow a fresh handoff while an inline publisher is still active. Both
fail; guards restored. Independent review found Direct contention could strand
pending work because inspection-worker admission rejects Direct; a separate
opaque-token drain and final-ring model address that defect. Review also removed
an unconditional idle Worker wake; drained workers now sleep unless new pending
work or retirement requests service. Independent re-review found no remaining
blocker in this unconnected CPU scope, without clearing runtime activation.

Runtime birth is still disabled. Notifier and guest USERD initialization, guarded
source views, actual GP copying, 5080 admission/migration, MMIO/ioeventfd integration
and native ordering must be completed and reviewed. Then exact-revision Linux
regression precedes a one-variable GravityMark Vulkan run against the same-GPU
VFIO179/183s PASS. Current Kayfabe GravityMark result remains FAIL/TIMEOUT.
