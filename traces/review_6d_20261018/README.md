# Candidate 6d review fixes

STATUS: LIVE, 2026-10-23. Base 01a171e63; code head 48431b8e0.

INFERRED hypotheses, registered before runs: extracting the existing host-object
wiring should preserve the birth tier; seed telemetry should distinguish an
unbound launch from an explicit bind; Cargo-built harness tests should replace
the six source probes. FALSIFIERS: a guest class changes the tier, explicit or
refused launches count as seeded, or a native configuration mutant survives.

MEASURED tests first, 2026-10-23: EXPECTED RED acb89830e added the tests alone.
Its telemetry target had 15 passes and 5 assertion failures. A refusal control
initially used misalignment, which belongs to the later bind perimeter; test-only
EXPECTED RED db6ea1737 tightens that input to invalid launch flags and again
has 15 passes and 5 assertion failures, all for absent seed telemetry. No missing
target counts as red. The birth-facts/native tests introduce the next API and
are exercised after extraction. Their original host-only behavior was correct;
no synthetic product defect was introduced to manufacture an assertion red.

CODE READ M1: ChanPlane now calls translated_decoder_config with its actual
HostRing through HostRingFacts. The only production implementation reads that
ring's allocated CE and video objects. Guest EngineObject statements are not an
input to the function. Tests send C8B5/C9B5 guest object allocations through the
real ChannelPolicy, then exercise this same extracted wiring with a fake C7B5
host. Host allocation and physical ChanPlane execution remain outside these
GPU-free tests. The guest-class mutant substitutes the guest C8B5 choice in the
selector; it is a hand mutation, not a hardware guest replay.

CODE READ M2: every BORN Translated line includes ce_seed, including zero.
TState counts a successful CE Launch decoded from the seed before an explicit
CE bind; it carries the count across segments and never clears it at the bind.
A refused launch does not increment. The RETIRED line reports seeded_launches.
These are decoded launches, not a claim of submission or GPU completion; the
hardware acceptance below checks both completion and data.

CODE READ L1: six source-scraping probes invoking rustc from PATH were removed.
Six Cargo-built unit cases in the actual kf-gr-tier binary replace their
configuration assertions; two more check the actual missing-bind input packet.
The binary uses birth_decoder_config for both native rings. Stable CI already
runs cargo build --workspace, and kf-harness is a default-feature workspace
member. The public-type guard is cargo check --workspace --all-targets.
No test assertion was relaxed and no ignore was added. The typed telemetry
assertion strengthens the existing diagnostic assertion used for EXPECTED RED.

CODE READ L2: keep the allocated-object pin. DerivedHostClasses::choose selects
the newest CE in generated family classes intersected with the host reply.
HostRm::alloc_ce_object allocates one object of that selected class; HostRing
caches the same class. GET_CLASSLIST describes allocatable classes, not aliases
of an already allocated object. Dedicated-copy push has no GR-class remapping.
Accepting every advertised class would require proof of compatible method and
object routing, beyond the current pin. The earlier CE requirement explicitly
refuses another class. The tests cover all five families and all seven derived
CE choices, including host profiles advertising only the older class. Separate
Ampere and Blackwell cases retain refusal of a lower class when the allocated
object is the newer one. These synthetic host replies do not establish the
class selected by every Linux/Windows driver stack. Recommendation: retain the
pin until the owner approves and validates aliasing; older-guest/newer-object
hardware compatibility remains unverified.

CODE READ L2 source path: kf-chip/src/host_classes.rs (selection),
kf-host/src/channel.rs (alloc_ce_object), kf-chan/src/host.rs (allocation,
ce_class and push_inner), and kf-chan/src/tmode.rs (the allocated-class pin).
No captured per-die class list or new compatibility table was added.

CODE READ L4: the experiment comment and startup text now describe negative
inputs using an inferred odd extension. The hue/vibrance test comment names
the two driver-version tables it actually loops over. Rendering is unchanged.

CODE READ hardware input: kf-gr-tier --ce-seed-only uses the existing owned
store/T-space setup and a host-derived non-GRCE COPY engine, then a Translated
kernel-work decoder. It allocates no GR context and does not run the older GR
checks first. The six-word guest entry is subchannel 4 SEM_A/B/PAYLOAD at 0x240,
then LAUNCH_DMA at 0x300 with 0x4c. It contains no guest SET_OBJECT. This is a
synthetic version of the Energy missing-bind segment. Host-authored binding is
still required and retained. Native host channels remain unprivileged USER;
KERNEL here describes the translated guest workload, not host RM privilege.
This harness does not test the guest's ChannelPolicy or interrupt-vector relay.

Hardware recipe for Q4 (not executed by F1): build the revision-pinned native
binary using the authorized build farm or the local build contract:

```sh
flock -w 3600 /tmp/claude-0/kf-local-cargo.lock \
  env CARGO_TARGET_DIR="$TARGET" nice -n 19 \
  cargo build -p kf-harness --bin kf-gr-tier --release --locked -j2
KF_GATE_GPU=0 flock -o /tmp/kayfabe-fastguest.lock \
  "$REV_BIN/kf-gr-tier" --ce-seed-only
```

Use the binary built from the reported code revision, retaining its source
identity with the result. Require CE_SEED_INPUT with nonzero ce_seed and
"guest_set_object=absent", a PASS for
"dedicated_ce_without_guest_set_object_completed_on_engine" with
seeded_launches=1 and the expected engine-written semaphore, then resource
release PASS and GR_TIER_VERDICT=PASS. A zero counter or an early refusal is
not coverage. NoCeObject/ForeignClass, an unchanged semaphore, or a failed
verdict falsifies the fix on that host. In a VM run correlate the BORN line's
ce_seed, engine and token with RETIRED seeded_launches, forwarded entries and
the workload's result; absence of refusals alone is not a positive witness.

Open issues, recorded without fixes: L3 keeps guest-selected CE tiers on a GR
ring's subchannel 4. L5 requires hardware to distinguish the inferred odd OLUT
extension from alternatives. L6 source-text kernel pins remain drift alarms,
not behavioral mutation killers. M3 belongs to the controller's separate
lockstep investigation. None of those paths was changed here.

MEASURED final verification, 2026-10-23, build-farm source:
BF_HEAD 48431b8e0e0f0c8accac31efc7108e6dd53436d2.
Workspace all-target check passes. Full suites for kf-chan, kf-qemu, kf-harness
and kf-disp pass: 1358 passed, 0 failed, 7 existing ignores, 66 result summaries.
KAYFABE_NO_KVM=1, KAYFABE_SLOW unset, release/locked, farm jobs8/nice15.
No standalone probe main was executed. This includes one passing lockstep
suite; it does not resolve M3's reported intermittent failure.
[validation.json](validation.json) carries exact commands, BF_HEAD and log hashes.
The focused suites pass 23 seed, 10 policy, and 8 native tests on the dev box,
using the global Cargo flock, jobs2, nice19, release/locked. The actual native
executable also builds. Whole-workspace format check and all 14 fast gates pass.
Full workspace Clippy --all-targets is 532 unchanged warning fingerprints
versus master e9c6d39e5, added 0; its debt gate reports new=0. See
[clippy-delta.json](clippy-delta.json). Claims reports new=0; ledgers unchanged.

MEASURED mutation campaign, 2026-10-23, code head 48431b8e0:

| Mutant | Distinct test killers |
|---|---:|
| Substitute the guest C8B5 tier for the host class | 4 |
| Disable the seeded counter | 5 |
| Ignore an explicit CE bind | 2 |
| Hide the birth seed | 3 |
| Guess C7B5 in the native configuration | 3 |
| Prepend SET_OBJECT to the hardware repro input | 2 |
| Remove the allocated-object class pin | 2 |
| Count refused launches | 2 |

[mutants.json](mutants.json) names the tests. Every mutant compiles and fails by
assertion; each source is restored byte-for-byte. The farm runs use only the
pushed code, independently of local temporary mutants. Hardware acceptance is
NOT DONE (n=0); Q4 owns it. L2 cross-class compatibility needs an owner decision.
