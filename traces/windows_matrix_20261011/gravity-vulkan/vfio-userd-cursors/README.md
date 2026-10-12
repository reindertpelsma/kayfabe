# Native USERD cursor observations around ring-growth registration

**STATUS: RESEARCH, 2026-10-12.** Reviewed native app PASS179s; complete bounded sample export. Exact cursor reset remains unmeasured.

H-CURSOR-OBS: read-only sampling of native SYSTEM memory-list USERD pages captures
usable old/new cursor transitions during the same passing Gravity Vulkan workload.
Falsifier: no usable slot transitions, incompatible public memory-list/RAM layout,
collector error, or native workload failure. A PASS of the app alone is not a
cursor result. Even useful passive samples are not atomic GPU trigger execution
or a general old-backing retirement proof. Compare against previous183s reference;
only passive sampling is added, same QEMU source5a407dcd8/SHA1325fefb009c... .

A controller-side process opens only the owned native QEMU's guest-RAM memfd,
read-only. Verify actual QOM8GiB ram0 size and public PC RAM aliases from QMP.
Decode only public single-page SYSTEM-list requests, with page-count/length/PFN
bounds, then read that page from the RAM file. It cannot read GPU BAR MMIO or
arbitrary process virtual memory. Retain only GP_GET/GP_PUT/Reference for the
page's eight512-byte slots; other bytes are discarded. Two samples flag changed
fields; neither is claimed atomic. Alias names/offsets are matched, source fd
size/name is checked; mismatches fail rather than guessing physical translation.

Page candidates and free barriers carry allocation serials/generations; a passive
capture can omit earlier frees or aliases, so these are not product ownership pins.
Tail live GSP output has writer delay; registration observation need not precede
the GPU trigger. Full strict GSP footer/export-hash validation remains mandatory
after shutdown. Sample output has its own hash/footer,100000-record/330s bounds.
The1ms interval/observation deadline never releases or changes GPU work. Sampling
stops when the matrix finishes, before normal native VM shutdown.

Fresh disposable baseline child, personal1380 untouched; serial GPU lock and
post-run NVIDIA/DMA-FQ/disk checks. No privileged driver call, pause, private
driver analysis, disk/RAM export, or secret is added. GPU-free boundary/public
memory-list/field-selection checks pass. Linux/Windows product runtime unchanged.

## Initial096d1fac6 result and review corrections

Native app PASS183s/rc0/image and timings, zero153/4101/WER/reboot. Native source
and SHA unchanged5a407dcd8/1325fefb009c..., collector source096d1fac6. Sampler
exited1 at100000-record cap without a footer. The wrapper's EXIT0 means native
lane/cleanup success; **H-CURSOR-OBS collector failure occurred**, not full proof.
Original compressed incomplete samples and strict complete GSP export are retained
in initial-result/. Partial analyzer requires explicit --allow-incomplete and
reports its independently computed hash; no collector footer is invented.

Limited sampled facts: first registered slot0x600 observed GET/PUT0/0 about961us
after registration observation, then225/948 at2094us. Second slot0x800 observed
0/0 at996us, then250/1428 at2152us with a changed second CPU sample. These are
native RAM field observations, not atomic trigger/cursor-transfer proof. Initial
sampler's request-only candidate attribution was independently reviewed and is
not promoted as valid allocation/lifetime authority. Preserve its limitations.

Independent review found request-only success attribution, missing parent-free
invalidation, oversized partial-line retry, gap correlation and offset-only
summary joins. Corrections require unique successful observed fn4 reply, carry
hDevice/request index, invalidate client/device/object frees and fail on gaps.
This fresh cold-boot profile accepts only each queue's initial prefix at sequence0;
nonzero retained-history prefixes or later discontinuities stop collection. No
resumed VM snapshot is accepted. Successful empty replies still supply only a
bounded passive candidate, never a product ownership pin. Summary joins exact
client/handle/generation/allocation serial. Six GPU-free boundary/lifetime cases
PASS. Independent reviewer found no reproducible parser/ledger model defect;
all native-adapter gates remain open.

Reviewed retry keeps1kHz reads but ordinary advancing exports at100Hz, preserving
initial/identity/zero transitions immediately; planned record cap now writes a
truthful limited footer. Original failure stays retained. New immutable runner
uses a separate root and same native binary/workload; no change to a running
script or product code. No usable reviewed hardware result yet.


## Reviewed c28d1bf61 result, 2026-10-12 01:44UTC

Native QEMU source/SHA unchanged5a407dcd8/1325fefb009c..., reviewed runner and
sampler sourcec28d1bf61. **Native app PASS179s/rc0**, image862178bytes,
129885 timing lines, NVIDIA3D98.63%/copy40.00%, peak2247MiB. Zero guest
153/4101/WER/live-kernel/reboot and bounded hostXid0. The previous183s result
is a separate passing reference; this run is179s. Wrapper and sampler EXIT0.
Normal shutdown, NVIDIA restored, DMA-FQ/noQEMU/lockfree/stop-file absent;
115960332288bytesfree. Personal Windows chain untouched; no Vast instance exists.

Strict GSP decoder validates18724 records/source/export metadata hashes,
no declared sequence gaps/drops. Capture still limited=true at300s,
160 unstable snapshots/6661392 invalid-element attempts; those are observer
counters, not GPU faults. The separate sample export validates14973 records
and its SHA735a5e7cddf595372e5fc4427dedd49fd73815731c8e026dbd5dffd49819d023.
292126 page reads, two successful-reply-paired SYSTEM memory candidates,
two registration observations and two observed free barriers. Footer reason
owner-collection-stop/limited=false means a complete bounded file, not complete
GPU/RM history. Collection stopped normally at the app's end.

Measured first registered slot0x600: first associated CPU sample GET35/PUT949,
1794us after registration observation, both reads stable. Second slot0x800:
first associated sample GET58526/PUT58549,183653us after registration observation,
both reads stable. No associated sample precedes either registration. Successful
reply pairing delayed the second candidate's installation. Both requests resolve
to the same SYSTEM page in this run; handles and allocation serials differ.
Samples show real cursor progress on requested slots, **not exact initial values,
old/new transfer, trigger execution or retirement semantics**. The initial
request-only collector's zero observations remain separately limited evidence;
they are not upgraded by this reviewed run. H-CURSOR-OBS supplies useful new-slot
observations but does not close the old/new transition measurement gate.

Compressed raw exports, app/oracle, summaries, original logs and hashes are in
reviewed-result/. Reproduce with strict summarize_samples.py and the public GSP
decoder after decompression. No password, input keys, guest RAM or disk exported.
No Kayfabe application behavior or compatibility claim changed.

## Same-page cross-slot reanalysis, 2026-10-12 03:56UTC

cross_slot_window.py validates both original exports before reporting the narrow
second-registration window; reviewed-result/cross-slot-window.json preserves
its output. Original candidateff00a00b and laterff00a00c resolve to the same
SYSTEM page. While ONLY the original candidate is associated, raw slot0x800
reads GET/PUT0/0 at-2329.676us relative to second registration observation,
with old slot0x600 at15693/15693. At+8616.813us, old slot is15705/15705 and
raw new slot1429/1429; at+19389.284us new3953/4309. Two CPU reads agree for
these samples. This is a raw field observation on the original candidate's
page, NOT retrospective new-handle ownership or an atomic trigger sample.

INFERRED: the zero-to-lower-progress transition supports a new-ring reset
hypothesis over copying the old cursor numerically. It does not prove an exact
initial GPU cursor, universally exclude a wrap, or authorize old-backing
retirement. No product reset/preserve rule or constant is added. Existing
summarize_samples.py strict new-handle association and its limits remain intact.
