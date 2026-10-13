# Native deferred FIFO relocation: recovered positive reference

**STATUS: RESEARCH, 2026-10-13.** A native capability is measured; no product
caller, Windows repair or cross-VM containment claim follows.

## Measured result

The previous Codex session completed this test but stopped before publishing its
results. Source `1a666d78301d444c4b35b021483db12841ff8980`, RTX 4070,
stock host driver 595.91.07, 2026-10-12 23:18:02–23:18:19 UTC:

| Registration caller | NOP control | Deferred trigger |
| --- | --- | --- |
| UID 65534, effective capabilities zero | Replacement marker and GET stay zero | Replacement GPU marker = 2, replacement USERD GET = 2 |
| UID 0, effective capabilities recorded in raw log | Replacement marker and GET stay zero | Replacement GPU marker = 2, replacement USERD GET = 2 |

All four processes return success, free their own channel/view/mapping/memory,
and have zero Xids in separately attributed microsecond journal windows with
two seconds of observation after process exit. Both modes receive the same 128
post-tail doorbells and retain resources for two seconds. The replacement
marker is referenced only by the replacement pushbuffer. The CPU initializes
markers but never writes completion values. The channel birth reports USER.

The pre-run hypothesis was that a fully prepared owned replacement GP/USERD
could execute through public V2 registration. Its falsifier was missing native
replacement marker/GET, registration refusal, cleanup failure or host Xid.
The falsifier did not occur. Raw logs, source, binary SHA256, GPU identity,
journal windows and supervisor verdicts are in
[prepared-native-migration](../../traces/windows_matrix_20261011/gravity-vulkan/prepared-native-migration/README.md).

## What this corrects

The earlier same-configuration update faults do not establish that unprivileged
native relocation is unavailable. The prepared relocation is a positive
counterexample on this host/driver. Public direct-control privilege annotations
cannot override that observation about the deferred V2 path. The reason the
same-configuration case faults and the nested RM status remain unknown.

Two identical replacement descriptors intentionally accommodated either initial
GET 0 or inherited GET 1. Therefore GET 2 and marker 2 do **not** identify the
initial cursor rule or prove the requested 64-entry ring capacity. The fixture
uses one live, owned, device-local 64 KiB allocation. It does not establish
sysmem USERD relocation, a guest relay transition, Windows compatibility or
support on another driver/family.

## Authority assessment and implementation gates

This is an owned-resource operation by an unprivileged native client, not
evidence of arbitrary resource access or a security vulnerability. No foreign
client, host physical address, kernel channel or whole-guest mapping is used.
The root case is a separate reference; the UID 65534 case establishes capability
without relying on root's registration privileges.

The bench helper authors host client/channel/object/memory handles and exact
versioned fields. It checks ancestry and encoding, but is **not** a production
admission authority: production must additionally prove the entire FIFO span
belongs to the target refreshed VA space, the USERD span belongs to retained
memory, API-handle namespace/lifetime is exclusive, and old/new mappings remain
live through the actual transition. Guest control bytes must never be replayed.
No raw privileged control or changed channel privilege is justified.

Before choosing a native product path:

1. Discriminate cursor reset/preservation with distinct per-slot GPU markers and
   matched NOP/trigger stimulus. Test capacity/wrap separately if needed.
2. Establish private host USERD continuity and the exact guest relay endpoint
   switch. Registration or a preceding GPU tail alone is not action completion.
3. Review ownership, mapping/free races, handle reuse and ordering; gate the
   capability by evidence for the exact host driver rather than version guessing.
4. Run exact Linux gates/fast/desktop and original Windows workload oracles after
   any runtime change. M1–M3 cross-VM availability experiments remain open.

The existing worker/shadow diagnostics stay default off. This finding warrants
evaluating the authored native alternative before more parser wall-clearing;
it does not select that alternative or retire the mediated design.
