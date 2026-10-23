# VF1 physical-GPU SPECviewperf15 reference — 2026-10-22

> Release: SPECviewperf 15.x (patch level not recorded)
> Configuration: Windows guest (owner image) in a QEMU/KVM VM, 12 GiB RAM, 8 vCPUs, Full HD; VFIO arm = physical GeForce RTX 4070 passed through; kayfabe arm = kf3 virtual GPU (details in README.md of this directory)
> (nc) = non-compliant: run in a virtualised configuration, not an official SPEC result and not submission-compliant; SPEC and SPECviewperf are trademarks of the Standard Performance Evaluation Corporation, https://www.spec.org

MEASURED: **PARTIAL overall.** Both full native iterations completed all nine
workloads:150subtest rows,18Completed statuses. The selected follow-up and Steel
Nomad were not run after the command channel failed and the unchanged15-minute
watchdog stopped QEMU. NVIDIA/audio and an active Ubuntu Wayland desktop were
independently verified restored by04:55CEST, before the06:30deadline.

MEASURED: first iteration01:51:37–03:15:28CEST; second03:15:28–04:39:02CEST.
Both use one native boot. Run-directory timestamps are from a guest clock two
hours behind host local time; neither clock nor benchmark timer was changed.
These are vendor composites, not a fabricated overall suite score. Spread below
is(max-min)/mean with n=2. Official CSV values agree with selected result.json.

| Workload | VFIO1 | VFIO2 | Range/mean |
|---|---:|---:|---:|
| blender-01 | 61.61 (nc) | 60.61 (nc) | 1.64% (nc) |
| catia-07 | 63.10 (nc) | 63.13 (nc) | 0.05% (nc) |
| creo-04 | 104.94 (nc) | 103.97 (nc) | 0.93% (nc) |
| energy-04 | 54.57 (nc) | 54.67 (nc) | 0.18% (nc) |
| enscape-01 | 37.26 (nc) | 37.39 (nc) | 0.35% (nc) |
| maya-07 | 162.14 (nc) | 160.37 (nc) | 1.10% (nc) |
| medical-04 | 50.54 (nc) | 50.58 (nc) | 0.08% (nc) |
| solidworks-08 | 29.51 (nc) | 29.19 (nc) | 1.09% (nc) |
| unreal_engine-01 | 63.58 (nc) | 63.70 (nc) | 0.19% (nc) |

MEASURED: the preserved kayfabe run marked Blender,CATIA,Creo Failed; Energy
Completed(45.60); Enscape remained incomplete; the remaining workloads had no
output. This corrects the handoff's broader failure description.27partial numeric
subtests are preserved separately from run status. The same interval contains
39nvlddmkm153records; this is not asserted to be39TDRs or proof of their cause.

MEASURED: paired-subtests.csv contains all27baseline rows and both native samples.
Largest native-mean/baseline ratios: BlenderClassroomEevee07=4.383x;
EnscapeBistroInteriorUltraHQ04=4.157x; BlenderColorXray04=3.417x.
Enscape04's native values11.64/13.05FPS have11.42%range/mean. Completed Energy
composite mean54.62 versus45.60 is+19.8%. Four Energy subtests remain within
about1% of baseline. Ratios involving failed/incomplete baseline workloads are
subtest comparisons, not successful workload composites.

MEASURED/CODE READ comparison conditions: same owner disk through a new CoW,
copied UEFI variables, same QEMU binary/source89fc8a222, CPU flags,12GiB RAM,
8vCPUs,cache=none/aio=threads/iothread,UUID/SMBIOS/MAC retained. Physical GPU
and audio replace kf3-gpu. Executed argv is vfio-command.json. Backing disk was
open O_RDONLY, overlay O_RDWR. Native physical memory.total=12282MiB; kayfabe's
configured framebuffer store=8192MiB. Capacity is a confound, not a proven cause.

MEASURED: primary desktop3840x2160 at nominal60Hz. Native render FPS exceeds60
(up to666.80), so there is no uniform60FPS render cap. CODE READ(owner handoff):
kayfabe broker output is59.996Hz; this is different from vendor render FPS.
No profile/vsync override was applied; no matched uncapped kayfabe rerun was made.
MEASURED: saved Window definitions match between four shared baseline/native
workloads; native Blender client1920x1080 and SolidWorks1920x998 were observed.
SolidWorks' smaller dimensions match its vendor FullHD definition. Original
runtime rectangles and exact fractional display refresh remain unverified.

CODE READ: SPEC documents15GBavailable RAM; the required12GiB VM is below that
requirement. Treat these as diagnostic references, not submission-compliant
scores. [SPEC user guide](https://gwpg.spec.org/wp-content/uploads/2025/12/SPECviewperf15-User-Guide.pdf).

MEASURED: System/Application samples through about04:38CEST show unchanged boot
record14976,zero new Display/nvlddmkm events and zero ApplicationError1000 events.
The post-suite WER audit found83logged records:81saved EventTimes before native
boot,zero verified after boot,and CitySample/AncientGame RADAR_PRE_LEAK_64 records with
unavailable original fault time. Old records are not counted as new native TDRs.
Guest event logs after command-channel loss were not collected.

MEASURED: full CLI output was archived before any follow-up. Its task was Ready
and no benchmark process remained. Its exitCode field is **null**, so no zero CLI
exit code is claimed. This is a real automation defect despite18Completed
workload statuses. A benign process-exit probe then returned no result; agent
requests timed out. INFERRED: its exit1terminated the shared PowerShell command
host. A virtual-keyboard attempt to restart the existing agent task had no
verified recovery before the unchanged watchdog fired. No follow-up benchmark
was started, no safety gate was bypassed, and no timer was extended.

CODE READ: PowerShell documents this process-handle/ExitCode failure pattern.
A retained handle or direct Process API needs isolated Windows tests for exit0
and exit7 before this wrapper can be trusted for CLI status. That fix and its
runtime validation were **not completed**; an unverified fix was not committed.
[PowerShell issue20400](https://github.com/PowerShell/PowerShell/issues/20400).
The table emitter was syntax-checked and exercised on partial live output; the
selected-run end-to-end check was not run. Recommend the supported vendor CLI
as the automation entry point, with these wrapper gaps fixed first.

MEASURED: watchdog reasonNO_PROGRESS_15_MIN; QMPquit ended the owned QEMU with
exit0 at04:54:41CEST. Clean Windows shutdown was not verified. The command
channel was unavailable after a probe performed after the full benchmark;
a physical guest/GPU hang is not proven. New host Xids=0 by anchored journal.

MEASURED: automatic restoration recorded restored:false after a transient
nvidia module-load error. This record is preserved unchanged. Independent
verification found GPU=nvidia,audio=snd_hda_intel,nvidia-smi successful(driver
595.91.07),GDMactive,UbuntuWaylandActive=yes,GNOMEShell present,noQEMU,and lock
acquirable. nvidia-persistenced stayed inactive, matching its initial state.
The independent verification establishes the restored desktop; the automated
restorer's error is not hidden or treated as a successful command.

MEASURED tests:63Python pass;6Windows export checks pass; installer and embedded
runner parsed with zero errors.22distinct recorded mutants each fail at least
2tests(2–7). Before fixes: initial1loadererror; latest behavior regression62tests/
2failures in EXPECTED RED a6ada19eb; standalone-switch EXPECTED RED4898f89d9.
The exit-code defect is a gap in that coverage. No product Rust/C/CUDA changed;
no cargo build/target was created. validation-summary.json holds the kill table.

CODE READ: scripts/bench/specviewperf/ contains the interactive-user CLI runner,
text exporter, table parser and host supervisor. Full native output text remains
on the trusted host under the authorized lane directory; no vendor binaries or
raw system-info objects are published. Earlier safety-refusal and restoration
files are historical, superseded by the explicit owner desktop override and
this measurement. The host supervisor also had an earlier SSH-scope launch
failure; the measured two full iterations used a dedicated system service.

MEASURED at05:14CEST: overlay and copied UEFI variables removed after the full
results push was verified. No VF1 QEMU/helper service remains; no target directory
was created. The first cleanup lock wait expired after120s under an unknown
holder, which was left untouched. After read-only observation showed its release,
one new bounded cleanup acquired/released the lock. A later05:17nonblocking
probe found the shared lock busy again; its holder was left untouched. Owner disk mtime still matches pre-native23:22:47.682478UTC.76native
text files and21baseline text files remain on the trusted host.11unconsumed own
agent requests were cancelled. The temporary host agent server is stopped; start
the existing broker again when guest-agent use is needed. VF1 did not restart
the owner VM. See final-cleanup.json and queue-cleanup.json.
