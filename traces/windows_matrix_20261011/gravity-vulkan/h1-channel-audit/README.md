# H1: deferred FIFO migration and channel classification

**STATUS: RESEARCH, 2026-10-11 20:59UTC.** Evidence audit before implementation;
no hardware run or owner-session interaction.

H1: only kernel-submitted Translated channels trigger FIFO_UPDATE_CHANNEL_INFO;
direct-doorbell Passthrough user channels do not migrate their FIFO/USERD.
Falsifier, stated before the audit: a channel classified Passthrough, with
measured direct guest doorbells, actually triggers a registration whose inner
command is FIFO_UPDATE_CHANNEL_INFO. Registration alone is insufficient to
establish this falsifier; absent trigger coverage is inconclusive.

Audit run1361 and the same-GPU VFIO observer: channel allocation,5080 allocation,
registration, token handout, scheduling, first doorbell and first0x200 trigger.
Preserve unavailable events as UNMEASURED. VFIO has no kayfabe channel-kind
classification; correlate public alloc flags/controls rather than inventing one.

Decision guard: implement software migration only if the relevant channel is
actually Translated and its public semantics are established. If a direct
Passthrough trigger falsifies H1, record evidence and owner options, then stop
implementation. Otherwise state the remaining measurement needed. No host
privilege escalation, opaque control forwarding or speculative channel conversion.

## Result: H1 is inconclusive; Step 1's precondition is not met

**MEASURED**, from [audit.json](audit.json) and the reproducible
[audit.py](audit.py), with [selected original log lines](run1361-selected-lines.txt):

| run1361 registering client / channel | Kind | 5080 registrations | Inner FIFO update | First logged trigger |
|---|---|---:|---:|---|
| c1d00015 / ff040001 | Translated | 43 | 0 | log2774, initialize context |
| c1d0007a / ff040025, host4e | Passthrough, Windows user-work | 1 | 1, API4000002d | UNMEASURED |
| c1d0009d / ff040001, after recovery | Translated | 23 | 0 | log21908, evict context; trigger logging is bounded |

Fault channel4e: channel birth8311, 5080 allocation8315, schedule-enable8365,
first forwarded relay submission8437, forwarded submission4096 at9047,
registration9052. Its release reports4495 forwarded submissions, zero relay
refusals, guest/host GP_GET=GP_PUT=0x2f0c. Registration is thus **after scheduling
and substantial real submission**, not an initial signal suitable for choosing
its mode. Token *assignment* is in the birth record; the exact token-handout
reply was not logged. Successful submissions establish that it was already usable.

The DOORBELL-LEDGER for token27 reports zero trap/fast rings, but this is **not
proof that the guest did not submit**. The USERD-relay worker independently
reports4495 forwards/rings. `serve()` routes relayed Passthrough twins through
`relay_serve()` before the other channel pump; the final trap/fast ledger does
not include those relay counters. Preserve these separate measurements.
The relay forwards the guest ring without translation and has a private host
USERD; it is still a Passthrough channel. Its guest token27 differs from native
host token4e, so the generic claim that every Passthrough guest token is the
native host token does not describe this relay implementation.

**MEASURED**, same-GPU VFIO observer: two registering channel parents:
clientc1d00015/channelff040001 has53 registrations, none FIFO update;
clientc1d00076/channelff040026 has two FIFO updates, both registration replies
status0. Channel allocation is observer line10918, 5080 allocation10920,
schedule10946 atQPC180090362705481, first registration11811
at180099575548355 (**9.213s after schedule**), second13999 at180157459631053
(67.097s after schedule). Both updates target their registering channel.
Ring count changes to65536 then131072, with different ring offsets and USERD
handles/offsets. These are measurement data only, never implementation constants.

**UNMEASURED:** actual GPU0x200 trigger on the fault channel; direct guest BAR0
doorbell events; exact token handout; atomic cursor transition/old-ring retirement.
VFIO observes RPCs, not GPU methods or doorbells. Its channel-allocation records
include the header and declared parameter size but **no allocation parameter
bytes**; flags/ProcessID/USERD-at-birth cannot be reconstructed from that capture.
No kayfabe classification is assigned to a native VFIO channel. The earlier
remaining-apps GravityMark capture also has one FIFO registration and no
RELAY-PEEK/PT-SNAP records, so it does not close the trigger gap.

**INFERRED:** an unregistered host5080 trigger remains a plausible cause of
Xid32. Registration plus Xid32 does not identify the actual method. H1 is neither
proved nor formally falsified under its stated direct-doorbell/actual-trigger
criterion. The fault channel is demonstrably not Translated, so implementing a
Translated-only migration now would not establish a GravityMark fix.

## Public-source correction and constraints

Rechecked public OGKM tags595.84 and595.91.07. In both,
[`deferred_api.c`](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/595.91.07/src/nvidia/src/kernel/gpu/deferred_api.c#L694)
lines710–714 explicitly **clear the PBDMA interrupt before executing this
software method** (`return NV_FALSE` from IsSwMethodStalling). The plan's
“skips clearing” statement is reversed. Registration saves privilege at100,
and trigger execution restores it at575. File SHA256 at both tags:
`7f1691356d5d39afb64decd1c730b664ea7d979c93adc3a96856d6c96f2c43b0`.

[`g_subdevice_nvoc.c`](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/595.91.07/src/nvidia/generated/g_subdevice_nvoc.c#L5085)
flags0x4 at5085 remain privileged. Update implementation begins at519 for595.84,
550 for595.91.07; requires client-allocated USERD at556/587 and a GSP client.
The existing native USER probe faults while NOP is clean; its inner error is
not decoded, so permissions remain a source-supported hypothesis.

Ownership, bounds and stream ordering remain mandatory even for a purely
software Translated migration. Public CPU-side code alone does not specify the
complete GPU cursor transition. No migration or privileged host path was added.

## Next discriminator and options

The owner's manual1380 is protected. No GPU job was started for this audit.
Next measurement must capture the faulting channel's submitted software method
**after registration**, not spend its bounded peek budget during initialization.
Record0x200 payload, object/subchannel binding, ring index, GET/PUT and host fault
time. A successful bounded read is an observation of guest bytes, not proof the
GPU fetched identical bytes; retain that distinction and correlate the fault.

If a real Passthrough update trigger is established, stop product implementation
and present the owner's choices. Registration-time conversion before schedule
and token use does **not** satisfy the observed sequence above; it would need a
different earlier public signal or a separately designed architecture, not a
late conversion. Clean NOT_SUPPORTED registration refusal is an experiment only
if Windows tolerates it. An optional patched-host tier belongs in
V3_HOST_PATCH_LIST and requires an owner choice. None of these options is built.

Reproduce offline: compile layout.c using OGKM580.65.06's public
`src/common/sdk/nvidia/inc` and `src/common/inc` include paths, write its output
to layout.json, then `python3 audit.py`. All decoded layouts come from those
headers; all inputs and their SHA256 values are recorded in audit.json.

## Exact89e8ddf8 Linux result, 2026-10-11 21:43UTC

Gates9/9+gate10 and corrected fast30/30 PASS, zero failures/crashes/notrun.
Initial fast NOTRUN remains documented below. Logs/binary SHA in
[late-fifo regression](../late-fifo-89e8ddf8/README.md). Windows smoke/control/
probe active; candidate sample and GravityMark result remain pending.

## Hardware campaign update, 2026-10-11 21:39UTC

Owner released manual1380; it stopped uncleanly and the installed disk is saved
separately. Prior allocator queue cancelled before running; checks combined into
exact89e8ddf8. Gates9/9+gate10 PASS, build/client/fastguest succeeded. Initial
fast suite NOTRUN because this runner omitted BENCH_DIR mkdir; original exit1
and fast log retained. Archived resume_campaign.sh fixes only that precondition
and starts fast30, then Windows smoke1490/control1495/probe1510 under sharedlock.
Only probe changes KF3_RELAY_PB_PEEK value; no migration or API registration.
No GravityMark fix/actual GPU trigger claimed. Historical prepared/protected
paragraphs below describe the earlier stage, superseded by this update.

## Prepared diagnostic, 2026-10-11 21:16UTC — NOT HARDWARE-VALIDATED

`KF3_RELAY_PB_PEEK=after-fifo-update` is a new value of the existing measurement
switch. Unset remains off; `=1` retains sampling from birth. The late mode
requires the existing authored twin5080 object and watches its guest registry
on a relay worker. On the first observed FIFO-update entry it starts at the
previous forwarded PUT, retains512 entries/64 logged software methods/16K words
per segment, emits a bounded64-byte bundle prefix and reports budget exhaustion.
It does not reset its budget on additional registrations, forward bundles,
register anything natively, migrate the ring or execute software methods.
All sampling follows the real host doorbell, as in the original peek.

The association carries an allocation-lifetime flag shared with the channel.
It publishes at admission before the action can queue or host allocation can
wait; later allocations cannot overwrite that pending lifetime. Guest object/subtree
free invalidates it without taking the relay lock. The sampler checks liveness
around registry lookup and also after arming, preventing ordinary handle reuse
or a concurrent free during allocation from reviving a stale association.

Independent review caught an existing diagnostic decoder omission: only the
first method/datum of a packet was logged. Three failing-first tests demonstrate
an incrementing packet crossing0x200, multiple nonincrementing API data words,
and IncrementOnce reaching0x200. Diagnostic decoding now expands these public
packet forms within the existing input bound. Final `cargo test -p kf-qemu --lib`:
**161 passed**, zero failures/ignored. See [validation.txt](validation.txt) and
[independent review](review.md). This is not a Linux hardware regression result.

Pre-run question: does a submitted late packet on the faulting Passthrough twin
contain0x200 with the registered update API handle? A matching sample supports
the candidate; a different positively identified faulting method contradicts
it. **No arm/no hit is inconclusive**, not the falsifier. Binding before the
sampling window is not recorded; sampling can race with guest rewrites/GPU
fetch, and registration can become visible after a batch was forwarded. Only
the first host-authored5080 is associated; later guest-only5080 objects are not
covered. Legacy formats remain sized-only. The probe cannot independently prove
GPU execution or formally falsify H1. Correlate submitted method/object binding,
GET/PUT, registration lifetime and fault time before an owner architecture choice.

Do not run this in manual1380. After the owner ends that session, respect the
already queued allocator campaign and GPU lock. Before a Windows claim, validate
the exact diagnostic source revision with Linux gates9/9+gate10 and fast30/30.
Use a fresh same-revision/same-flags GravityMark control and then change only
the peek value for the measurement run. Preserve unclean stops and failures.
No hardware run has been made with this diagnostic and no Vulkan fix is claimed.
