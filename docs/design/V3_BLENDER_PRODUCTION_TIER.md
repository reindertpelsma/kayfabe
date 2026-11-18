STATUS: DESIGN-ONLY, 2026-11-18. GPU-free fixtures exercise the adapter. **INFERRED** marks behavior that requires a Blender run.

# Blender production workloads

**DESIGN.** Use Blender production projects to find guest-specific failures after the existing app gate.
The current Blender smoke paths explicitly disable denoising
([app matrix script](../../scripts/apps/src/blender_render.py),
[graphics suite script](../../scripts/bench/gfxset/src/blender_scene.py)).
Production rows exercise the render, denoiser, interactive and error paths on the same project
on bare metal and in the kayfabe guest. This lane does not change the open guest image or its CI stage
selection. It uses the asset provisioning boundary in the [open image recipe](V3_APPS_IMAGE_RECIPE.md).

## Assets and provenance

**SOURCE REVIEW, 2026-11-18.** These catalog records were read without downloading project payloads.

| Official source | File-level licence and access | Use |
| --- | --- | --- |
| [Blender demo download index](https://download.blender.org/demo/) and [Cycles demo index](https://download.blender.org/demo/cycles/) | These indexes list payloads without file-level licence or checksum. No CC0 grant is established by the directory listing. | Candidates only. Resolve the exact asset's licence and credit before selecting it. |
| [Charge production catalog](https://studio.blender.org/projects/charge/) and [Einar facial demo asset record](https://studio.blender.org/projects/api/assets/6279/?site_context=featured_artwork) | `einar_facial_demo.blend`, CC BY 4.0, attribution to Blender Studio. The asset record links the source download and supplies no checksum. | Start with this published project for EEVEE, viewport and character-material rows. Confirm dependencies and active camera on the selected build. |
| [Charge 050_0130 shot asset record](https://studio.blender.org/projects/api/assets/7332/?site_context=featured_artwork) | Shot archive, CC BY 4.0, attribution to Blender Studio. The catalog marks access as subscriber-only. The asset record supplies no checksum. | Use an operator-staged local copy with linked dependencies. Never pass access credentials to a guest or rented box. |
| [Charge Material Asset Library Freebie asset record](https://studio.blender.org/projects/api/assets/7249/) | Source archive, CC BY 4.0, attribution to Blender Studio. The asset record supplies no checksum. | Stage locally for material and texture cases. Apply the materials to a production scene with a recorded operation. |

**DESIGN.** Keep an asset manifest with the exact source URL, project-relative file, revision if given,
file-level CC0 or CC BY grant as applicable, required attribution and digest. Additional files in an
archive need their own licence review when the release has exceptions. Do not infer a grant from
the film's licence or from download access. No CC0 source is currently selected. Keep all project
assets out of git and R2. Fetch approved assets at runtime. Supply the publisher's SHA-256 if
available. For the listed unpinned sources, the operator supplies either an official Blender
HTTPS URL for a direct project or a local path. A locally calculated SHA-256 records which bytes ran.
The runner accepts a SHA-256 pin in either mode and records the asset digest for comparison.
Stage archives and their relative dependencies outside the runner and record their digest manifest
with the run. Stage both sides from the same asset copy and dependency tree. Launch with Blender
auto-execution disabled. Operation scripts are local, reviewed test code.

## Enumeration and coverage

**DESIGN.** Generate the inventory on each selected Blender build and retain both JSON lists.
Use the union as the checklist, preserving host-only identifiers for guest attempts. An identifier
the guest build cannot select is a support difference only after matching the Blender build and
configuration. A case without a runnable scene operation stays visible as `not_run`.
The standard asset and per-case variants are data in the caller's case file.

| Area | In-app inventory | Required production action |
| --- | --- | --- |
| Engines and devices | `RenderSettings.engine`, registered render engines, Cycles device preferences, GPU backend RNA | Render one frame on Cycles CPU, CUDA and OptiX, plus every supported device class. Keep HIP as an explicit unsupported control where no HIP device is available. Render EEVEE and Workbench. Force Vulkan at process startup when available, then verify the active backend and render with a raster engine. |
| Denoisers | Cycles render-denoiser enum, explicit OpenImageDenoise and OptiX controls | Render at a low fixed sample count with each denoiser and with it disabled. OptiX requires a denoise-specific GPU observation. |
| Modifiers and physics | Modifier type, force-field, rigid-body and constraint types, fluid domain and flow enums | Exercise each modifier on a compatible shot object. Include cloth, soft body, liquid and gas/smoke. Step or bake the simulation over a fixed window and compare rendered frames and stable summary values. |
| Geometry Nodes, shader and compositor | Registered node classes polled for each node-tree type, including simulation nodes and nodes shared between tree types | Wire each node with valid inputs into an evaluated output. Reset and step simulation zones, connect a material to a visible object, and render compositor outputs with the operation enabled and disabled. |
| Texture bake and sequencer | Cycles bake enum and effect-strip operator enum | Bake each type to an active image with recorded UV and target settings. Build strips over production footage for each effect and compare selected frames. Track which effects have GPU evidence separately from enumeration. |
| Viewport and object types | Viewport shading and object enums | Exercise wireframe, solid, material and rendered views through Blender context. Draw/edit Grease Pencil and exercise volume, hair/curves and other object types in a visible shot. |

**INFERRED.** Blender 4.5 API behavior, RNA enum completeness in background mode, node polling,
renderer selection, exact frame filenames and high-resolution TGA output need live confirmation
on the chosen build. The device preferences and denoiser setup were read in the
[Cycles 4.5 source](https://raw.githubusercontent.com/blender/blender/blender-v4.5-release/intern/cycles/blender/addon/properties.py).
The [sequencer API](https://docs.blender.org/api/4.5/bpy.ops.sequencer.html) and
[CLI documentation](https://docs.blender.org/manual/en/4.5/advanced/command_line/arguments.html)
describe the effect operator and GPU backend option.

## Pressure and lifecycle variants

**DESIGN.** Pair each variant with the same production scene at ordinary settings. Keep settings,
budget and proposed falsifier in the run record. Record the host and guest build, driver, device
class, cache state, effective settings, VRAM limit and elapsed time.

| Variant | Case data and steps | Falsifier |
| --- | --- | --- |
| Resolution and samples | Render 7680×4320 and 15360×8640 at 100%, then each at a high declared Cycles sample count. Turn off adaptive stopping for tests that require a fixed sample workload. Use the same wall-clock budget on each side. | Guest times out or diverges after a stable host result. |
| Near VRAM and OOM | Raise subdivision and object instance count on a named production mesh. Use unique mesh data when shared geometry produces too little pressure. Calibrate against peak VRAM, then bracket the host allocation failure with a nearby successful control. | Guest fails below the host success point, survives where the matched host reproducibly fails without an equivalent capacity explanation, or reports a different OOM class. |
| Many materials/textures | Duplicate or replace production materials and image data with independently evaluated variants. Ensure viewport and camera actually see the variants. Record decoded texture sizes and residency. | Guest drops textures, produces a divergent image, or fails where the host succeeds. |
| Animation | Render a configurable inclusive range with fixed sample/seed settings. Compare every requested frame. Use a shorter range from the same shot as control. | Missing, corrupt or divergent guest frame, crash, or timeout. |
| Concurrent render and viewport | Run a bounded background render while a GUI operation orbits a camera and changes shading modes. Prefer a real GPU display session without Xvfb. Record both process windows and redraw/render progress. | Guest loses viewport progress, hangs or corrupts output while host records progress on both paths. |
| Cancel and restart | Use an invoked render and a Blender timer/operator in a GUI session, capture the cancellation callback and partial-output state, then restart and verify completion in the same session. | Guest misses the cancellation or fails the restart that completes on host. |
| Device switch | Render, synchronize completion, change device preferences in the same Blender process, then render again. Changes to the GPU frontend that require a restart use a separate row. | Guest keeps the old device, fails the next render, or produces a divergent image. |

**CODE READ.** The adapter includes settings for resolution, samples, mesh subdivision and
instances, and animation ranges. Contextual operations in the feature table and the concurrent,
cancel/restart, device-switch and material-pressure workflows need asset-specific operation scripts.
The script consumes the case dictionary as selector data and can use Blender's timers and
operators. GUI operation scripts must exit Blender after emitting their final result. The wrapper
applies the stated deadline to its Blender child. A concurrent operation that starts other
processes owns their cleanup and must bound their lifetimes. **INFERRED.** Actual memory residency,
display availability, Blender's cancel/restart sequencing, and guest timing need live checks.

## Running and deciding

**CODE READ.** Generate and retain inventory, then copy selected rows into a run case list and set
any case-specific settings. Use absolute runtime paths. Each run needs a fresh artifact directory.

```sh
blender -b --factory-startup --disable-autoexec --python-exit-code 17 \
  --python scripts/apps/blender_prod/blender_case.py -- inventory /tmp/blender-cases.json
python3 scripts/apps/blender_prod/runner.py run /tmp/blender-cases.json denoiser/OPTIX \
  /tmp/blender-config.json /tmp/blender-host1
python3 scripts/apps/blender_prod/runner.py compare /tmp/blender-cases.json denoiser/OPTIX \
  0.00392156862745098 /tmp/blender-guest /tmp/blender-host1 /tmp/blender-host2
```

The run config contains `blender`, `seconds` and `asset`, for example
`{"blender":"/opt/blender/blender","seconds":900,"asset":{"path":"/run/assets/shot.blend"}}`.
Case fields include `engine`, `device`, `backend`, `denoiser`, `samples`, `adaptive_sampling`, `resolution` as
`[width,height]`, and either `frame` or `frames` as `[start,end]`. For pressure set `mesh`
with `subdivision`, `instances`, `spacing` and optionally `unique_meshes`. Set `gui: true`
to inherit the configured GPU display or `operation` to a reviewed script. A script receives
`bpy`, `case`, `out` and `emit(status, reason, **kv)` and emits one terminal result. It writes
`render.tga` or the requested animation frames, or sets `image: false` in its case when it
returns only structured values. Mark a claimed GPU path with `gpu_proof` using the backend
or stage name. Generated device, OptiX denoiser and raster engine rows carry that field.

**DESIGN.** Use two or more independent bare-metal runs with identical inputs and settings.
Repeat equivalent runs with VFIO when isolating kayfabe from guest OS or driver configuration.
Keep host and guest CPU/GPU mode, device class, engine, Blender version, assets, cache policy
and resource budgets comparable. The code checks the recorded primary asset digest, operation
digest, case fields and budget; the run record supplies build, dependency, driver and device
checks. Do not compare CPU to CUDA, or CUDA to OptiX, as the primary oracle.

**CODE READ.** The still/sequence oracle compares RGB8 uncompressed TGA, with raster origin
normalized. Per frame, distance is maximum absolute channel difference divided by 255.
The host noise floor is the largest distance across all pairs of host repeats. The guest must
fall within the floor plus a declared margin against every host repeat. Report the floor, guest
distance and limit. A margin of zero uses exactly the host floor.
For a deterministic control start with zero, then justify any margin and cap the acceptable
host spread before interpreting results. An 8-bit display-space comparison is suitable
for this first lane. Add a linear float oracle before making HDR or subtle precision claims.
**INFERRED.** Fixed-seed Cycles should be near-identical for the same device class; the first
host repeats can falsify that expectation.

**DESIGN.** Collect timestamped driver or API telemetry attributable to the active workload.
For GPU compute, keep both the selected device configuration and process-correlated observed
GPU work. For OptiX denoising, tie the observed OptiX denoise activity to the denoise stage;
OptiX render or GPU memory allocation alone does not establish that stage. For Vulkan and
EEVEE, record renderer/backend and submitted GPU work. Include CPU and denoise-disabled
negative controls. The comparison reads an operator-produced `gpu-proof.json` with
`backend`, `t0`, `t1` and nonempty `evidence` pointing to the retained observation.
It checks the label and window. Telemetry collection and evidence interpretation are external.
A successful run without required proof is incomplete.

**CODE READ.** The run emits a diffrun-shaped row (`case,status,area,reason,secs,t0,t1,kv`)
plus the raw log and artifacts. Feed rows keyed by case to the coordinator's
`diffrun.compare(host, guest, limits, xids)` for status and Xid triage. Retain the Blender
comparison verdict alongside that report. The Blender comparator checks host repeat stability,
native error class and image tolerance. It catches differences such as an OOM class mismatch
under two identical `fail` statuses. Equal failures are error-parity evidence only. Two
unsupported results or two timeouts do not prove successful feature execution.
**INFERRED.** Native CUDA, HIP, OptiX and Vulkan OOM patterns in the wrapper were tested with
synthetic messages. Retain real logs to confirm spelling, recoverable warnings, process exit and
CPU fallback behavior before treating the matcher as hardware evidence.

## REDUCTION

**DESIGN.** Capture the run revision, asset and settings digests, process markers and their
monotonic and wall-clock correlation, both stdout/exit statuses, kernel logs and Xids,
and render output or its metadata. Bound the kf3 census window to start/end plus channel
retirement. Collect the refused RM control IDs, the channel/class and method counts,
and ordered memory operations (allocation size/flags, mapping and placement, bind, submit,
completion, unmap/free, relevant return codes). Preserve the first divergent operation and the
minimum preceding state needed to replay it.

| Failure class | Reduction and owning test |
| --- | --- |
| Guest loses support, fails a backend or silently falls back | Reduce to the first distinct capability query, RM control refusal or channel allocation. Fixture the control/class and status in `kf-rm`, or the channel allocation and enabled engine decision in `kf-chan`. |
| Image/artifact mismatch or operation result differs | Reduce to one changed pixel region, divergent frame or result from a single feature. Bisect compositor/engine stages, then isolate memory binding or method translation and replay through `kf-qemu` or `kf-chan`. Keep hardware image parity as the control for effects beyond replay. |
| Wrong allocation failure or capacity threshold | Reduce to the memory-op sequence around the first refused allocation/map and preserve the logged slot/heap geometry. Replay placement in `kf-cuda`, mapping in `kf-qemu`, or an RM refusal in `kf-rm`, as indicated by the first divergence. |
| Crash, timeout, hang or cancellation/restart defect | Isolate the first missed state transition: submission, completion, channel retire or release/rebind. Replay deterministic transition and ordering checks in `kf-chan` or `kf-qemu`. Retain the smallest timed hardware control for watchdog, scheduling or timing dependencies. |
| Missing proof, protocol or staging failure | Reduce the log, artifact set or configuration to a Python fixture first. If captured evidence instead reveals a deterministic guest state divergence, replay it in the owning crate. |

Use the [replay test pattern](V3_REPLAY_TESTS.md). Put logged fixture lines and labelled derived
inputs under the owning crate's `tests/fixtures/`, with the Rust reproduction in its `tests/`.
Recreate the logged pre-state, assert the refusal or wrong decision and pin the acceptance
invariant. A fake backend records requests and bounds, without simulating completion of GPU work.
