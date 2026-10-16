STATUS: RESEARCH, 2026-10-16. GPU-free prototype first; production integration and hardware validation are pending.

# One store view for all channel readers

The scope is one native store allocation, shared by all USER and Translated channels in its
QEMU process. A separate device/store must own a separate instance: physical offsets alone
cannot identify allocations across devices. The firmware carve-out is the exclusive upper bound.

The interface accepts physical store offsets, with checked addition and a maximum of 256 bytes
per access. The command-source adapter retains its independent whole-command and placement checks.
It partitions a larger admitted snapshot into small accesses; it never uses this seam for page tables.

FULL is an explicit operator property, default off. Realize must map the entire permitted store
or refuse `STORE_VIEW_FULL_MAP`. There is no BAR1/free-space detection or silent WINDOWED fallback.
The read path validates bounds and accesses the owned mapping directly, with no table or lock.

WINDOWED uses a dense table indexed by physical offset / 64 KiB. A slot has EMPTY, ARMING, and
ARMED states plus a reader count in the same atomic word. A reader makes one bounded CAS attempt
to pin ARMED; contention or ARMING returns Retry. A writer can take ARMED only at reader count zero.
The resulting RAII pin is a per-slot hazard reference: reclamation is impossible until its last
reader exits. A stalled reader pins only its span. Thread unwind drops its reference. This avoids
an epoch domain whose stalled participant could defer reclamation of unrelated spans.

The exclusive arming winner owns only its slot; native open/RM_MAP/mmap and release execute on
worker/maintenance threads outside placement, topology, channel and device metadata guards.
No reader waits for that winner. Native calls themselves can block in the driver, as existing
arming does; claiming that an ioctl is nonblocking would be false. The table and pin protocol
have no wait, OS call, Mutex, RwLock, or unbounded CAS retry. A separate backend owns native calls.

A global live-window budget bounds BAR1 consumption, independent of per-channel reads. A clock
hand supplies bounded eviction work. Each mapping charges the owner that armed it. An owner can
reclaim its own mappings or an owner's surplus; it cannot evict a less represented owner to grow
its own share. Second-chance bits avoid repeatedly evicting hot spans. The policy is not an
isolation boundary: all offsets still require the store bound and source-placement admission.

On RM refusal, retire an eligible mapping and retry once; then use an ephemeral owned mapping for
that access, released after the access outside other guards. Count each outcome by name. Cleanup
failure retains mapping ownership for explicit maintenance retry/session teardown, never pretends
it released a resource. Integration must preserve snapshot preparation outside row/topology guards:
a read miss under those guards returns Retry, never maps there.

Prior art in this repository: kf-trap relay_dispatch packs ownership into one atomic word and
uses Loom; the existing userstore backend retains partial arms and cleanup failures. These are
hypothesis sources, not proof of the new reclamation protocol. GPU-free tests must exercise the
actual slot implementation under Loom plus real memfd mappings under concurrent eviction.

Acceptance: exact byte patterns, balanced arms/releases and fd/VMA counts, stalled-reader and
failure storms, fair-share adversary, 8/16/32-thread ten-minute runs, final sixty-minute soak,
and 1..32-thread read scaling. Hardware: exact-revision Linux broker/interactive lanes, then
Heaven comparisons. Existing FULLMAP experiment must be read before any FULL comparison.

CPU write discipline: WINDOWED writes acquire exclusive idle-slot ownership, returning Retry
if any reader exists; they never wait. FULL writes require an exclusive Rust borrow. This keeps
the FULL read path free of atomics while preventing a plain data copy racing a host CPU writer.
The runtime paths currently being replaced only read views; no new GPU work is CPU-executed.

Integration draft, 02:25 CEST: both channel readers use one instance created at device realize.
QOM ABI 31 adds `store-view-full` (default false) and `store-view-windows` (0 = 256 global windows).
`user-view-keep` is retained as a deprecated property with a diagnostic, not reinterpreted as a quota.
The old cache exists only in its baseline tests. Inactive channel owners no longer reserve their
last window; active owners keep a minimum share under concurrent adversarial eviction.

Translated read contention is a typed worker Retry: the ring restores its cursor and does not
retire the entry; the runner fences any earlier queued work and schedules another worker turn.
The new GPU-free test injects contention at the GP descriptor and at the pushbuffer separately,
then verifies one fetch and one retirement. No completion is supplied for the refused read.

New perimeter API: `VolatileRegion::write_data_small(&mut self, offset, bytes)` checks write
protection, the 256-byte cap, overflow and the real mapping bounds before any byte is stored.
Byte stores permit unaligned data writes. Its dedicated test covers arbitrary small lengths,
boundary refusal, oversize refusal and a read-only mapping. It is not an engine executor.
