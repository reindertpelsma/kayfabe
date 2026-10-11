# Passthrough FIFO-update registration refusal

STATUS: RESEARCH, 2026-10-11 22:52UTC. Prepared default-off diagnostic; Windows result pending.

## Hypothesis and falsifier, before hardware

H2: Windows can recover from NOT_SUPPORTED at registration of the deferred
FIFO_UPDATE_CHANNEL_INFO on a live Passthrough Windows user-work channel,
retaining its old ring or choosing another valid path.
Falsifier: the original GravityMark1.89 Vulkan workload fails, loses its device,
submits a refused handle's trigger, or faults/recovers. Absence of method samples
is not proof that no trigger was submitted. A timeout is not a pass.

Required success evidence: same original600s workload as VFIO PASS183s, normal
exit, benchmark image/timings, measured NVIDIA activity; no hostXid32 or guest
153/4101/reboot/recovery. Record the actual FIFO-refusal counter. If complete
later-trigger coverage cannot be established, mark that part UNMEASURED and do
not promote the experiment to default behavior solely from missing logs.

Only experiment switch: `KF3_WIN_REFUSE_PT_FIFO_UPDATE=1`, default OFF; not a
boot requirement. Other production flags/media/oracle remain matched. Deliberate
divergence: native Windows privileged registration returns OK; this diagnostic
returns NV_ERR_NOT_SUPPORTED, leaving no registered entry. The 5080 allocation,
other deferred operations, and all other channel kinds remain unchanged.
Parent classification comes from the validated live graph plus Kayfabe's
Passthrough twin map, never the bundle's hClient/hChannel. Existing handle and
capacity checks precede the deliberate refusal. Single first-refusal log and
VM-lifetime `fifo_update_registration_refused=N` heartbeat counter; no unbounded
per-refusal log/store. No host verb, migration, trigger consumption, completion,
new privilege, timeout workaround, or address-space change.

## Offline checks

VFIO observer-summary.json has no plain20801116 control, two successful
INTERNAL50800104 registrations, reported_missing0 in each direction and
capture_complete_header=false. A deferred trigger executes inside GSP; it need
not issue another CPU-to-GSP RmControl. Therefore actual native execution stays
UNMEASURED; absence of a plain control does not establish a different trigger
condition in Kayfabe.

Published h1-channel-audit/audit.json: native updates request65536 then131072
entries; USERDmemoryff00a00b thenff00a00c, offsets0x600 then0x800; FIFOoffsets
0xa1c2c0000 then0x2dfe90000. Original native alloc params are absent, so its initial
ring/USERD and memory-object aperture/size cannot be reconstructed. No invented
placement comparison. Run1361 logs old ring32768/FIFO0x6b20000 and RAM-relayed
USERD, but not the complete update bundle/memory-object allocation. Probe1510
captures old32768 to requested65536 and newFIFO0x42be000/USERDoffset0x600 with
memoryhandleff00a00b; backing aperture/size and old handle/offset are unmeasured.
Handles/addresses are measurement data, not product constants.

Manual1380 bounded observations: no new hostXid since readiness and no printed
FIFO-update command. No measured Passthrough migration trigger. Its owner-reported
Minecraft/GPU freeze is unresolved separately; no shared cause claimed.

## Validation

Failing-first registry test initially failed compilation because the refusal
API/counter did not exist. Targeted test now passes; kf-rm131 lib tests and5
RPC integration tests pass, including all three registration forms, spoofed
bundle targets, invalid objects/sizes/handles, preserved other APIs and
out-of-scope admission. kf-qemu161 lib tests pass including final counter wiring; full kf-rm tests
pass. Original logs retained here. Exact-revision Linux/hardware results follow.

## Decision guard

If refusal fails, do not build a migration workaround. Publish the refusal's
failure and a design decision: restricted mediated user channels chosen at
birth (stable host FIFO, parse/control the stream, own user VAS only, no physical
operands); optional validated host-patch tier; or partial clean-refusal scope.
Registration already occurs after scheduling and4096+ submissions, so it is too
late to convert this channel. No cost estimate exists yet. Preserve personal
manual1380 image chain; use disposable baseline-backed test overlays.
