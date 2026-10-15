# Win11 VM 2990: Blender / Cycles scripted tests (2026-10-15)

STATUS: LIVE, 2026-10-15, INTERIM (written when the VM was taken down after a TDR; tests c, d, e and the failure end of b are not yet run).

Scripts: `scripts/bench/blender/` (own code): `cycles_bench.py` (procedural scene), `cycles_soak.py`, `gui_redraw.py`, `vram_series.sh`,
`soak.sh`, `guirun.sh`, `blrun.sh` / `blwait.sh` / `runwait.sh`, `bl_run.ps1` + `smi_sampler.ps1` (guest side), `filesrv.py` (loopback file
server, guest reaches the host as 10.0.2.2:8790), `monitor.sh` (read-only host monitor). Raw data: this directory.
VM: kf3 eb9f2aa3, production profile, 16 GiB RAM, 8 GiB framebuffer, host RTX 4070 (12 GiB, 8884 MiB of it held by the VM's qemu from boot,
so growth of the guest's VRAM use does not change host VRAM use: measured, host used stayed 9069-9096 MiB during every test). Owner was using the VM
(SolidWorks benchmark process and Blender open): timings carry that noise. All runs were headless `blender -b` started by us as SYSTEM (hidden, no screen takeover).

## Step 1: what Blender sees (measured)
Blender 5.2.2 LTS (hash d13f752e3b9c) at `C:\Program Files\Blender Foundation\Blender 5.2\blender.exe`.
`get_devices_for_type`: CUDA -> `NVIDIA GeForce RTX 4070` (CUDA) + `AMD Ryzen 9 7900` (CPU); OPTIX -> `NVIDIA GeForce RTX 4070` (OPTIX) + CPU; HIP, oneAPI: none.
Falsifier for "the GPU is usable by Cycles" (no CUDA/OPTIX device, or the first GPU render fails): NOT met. First OPTIX render (5760 tris, 640x360, 16 spp) FINISHED. 
`gpu.platform.*` is not available in background mode; the GUI query is part of test (c), not yet run.

## Test (a): Cycles render, 3.93 M triangles (3 displaced ico spheres), 8 distinct 4096^2 PNG textures, 1920x1080, 128 spp, fixed seed (measured)
Scene: `--sub 9 --objs 3 --ntex 8 --texsize 4096 --samples 128`; 3 934 720 triangles. render_s = wall clock of `bpy.ops.render.render` after a 64x64 warm-up render.
| run | device | render_s | warm-up_s | GPU mem peak (guest nvidia-smi, 1 s samples) | base | image mean / std |
|---|---|---|---|---|---|---|
| a_optix1 | OPTIX | 5.87 | 2.35 | 2941 MiB | 1051 | 0.63984 / 0.10051 |
| a_optix2 | OPTIX | 5.32 | 2.00 | 2941 MiB | 1051 | same |
| a_cuda | CUDA | 7.61 | 3.34 | 2959 MiB | 1051 | 0.63984 / 0.10051 |
| a_cuda2 | CUDA | 7.58 | 2.54 | 2959 MiB | 1051 | same |
| a_cpu | CPU, 4 threads, BelowNormal | 148.56 | 1.05 | 1051 (no GPU use) | 1051 | 0.63984 / 0.10051 |
GPU/CPU ratio: OPTIX 148.56 / 5.6 = about 26x (CPU capped at 4 of 8 vCPUs so as not to starve the owner; measured, the ratio on 8 threads is not measured).
GPU and CPU images agree to 5 digits in mean and std (the render is correct, not just finished). All: 0 host Xid, 0 GSP resets, 0 new guest events (monitor).
Peak GPU memory use: +1.89 GiB (OPTIX) over the base.

## Test (b): VRAM pressure (measured)
Hypothesis: Cycles fills the 8 GiB guest framebuffer with textures and then either spills or reports an out-of-memory error; film size does not matter (Cycles X allocates per tile).
Falsifier for the film part: peak grows with film size. Result: FALSIFIED the other way, see (3).
(1) Texture series, OPTIX, 4 objects x 1 310 720 tris (5.2 M tris), 64 spp, 1080p, N distinct 4096^2 textures (64 MiB each on the GPU):
| N | render_s | peak MiB (delta over base) | min guest free RAM MiB | result |
|---|---|---|---|---|
| 8 | 3.88 | 3013 (+1962) | 8202 | ok |
| 24 | 13.87 | 4037 (+2986) | 7237 | ok |
| 40 | 16.90 | 5061 (+4010) | 6226 | ok |
| 56 | 21.91 | 6098 (+5047) | 5188 | ok |
| 72 | 30.39 | 7112 (+6054) | 4198 | ok |
| 88 | 35.48 | 7748 (+6698) | 3119 | ok |
| 104 | 85.05 | 7685 (+6963; base had dropped to 722) | 1712 | ok (render time jumped 2.4x; peak did not grow: INFERRED host-memory spill, not confirmed, no Cycles log) |
Series stopped by MY guest free RAM floor (3000 MiB), not by a failure: the largest completed size is N=104 (6.5 GiB of textures + 5.2 M tris), peak 7748 MiB of 8192. 0 Xid, 0 resets, 0 guest events.
(2) Film series, N=88, 1 object, 16 spp, resolutions 1080p, 4K, 5760x3240, 7680x4320: all ok, peak 7605 / 7653 / 7653 / 7653 MiB (render_s 18 / 21 / 24 / 29). Film size does not change VRAM use: Cycles X allocates per 2048^2 tile. Stopped at the RAM floor (min free 2515 MiB at 7680x4320).
(3) Tile series (tile 8192, OptiX denoise on, N=72, 1 object): step 1 (1920x1080) ok, render_s 6.08, peak 6675 MiB (+5947), min free RAM 5329. Step 2 (5760x3240) was running when the TDR hit (below).
The failure mode of Blender at the VRAM limit (clean out-of-memory message vs crash) was NOT observed yet: the RAM of the guest, not the 8 GiB framebuffer, ended the series first.

## FINDING: guest TDR during test (b)(3) step 2 (measured)
- Host 02:17:52 CEST = guest 00:17:52 (reported by the coordinator: guest nvlddmkm event 153, black broker window, no host Xid).
- Running then: `b_optix_n72_5760x3240`: Blender OPTIX, N=72 textures, 1 object (1.33 M tris), 5760x3240, 16 spp, `--tile 8192 --denoise 1` (OptiX denoiser), started at host ~02:17:20 (the previous step finished at 02:17:18), i.e. 32 s into the process (about when the first full-size 8192 tile render or the denoise would run). Its last result: none (no result.json). Last finished result: step 1 above.
- Monitor (`monitor_until_tdr.csv`): 02:17:27 guest_events 0; 02:17:58 guest_events 1; host Xid 0, GSP resets 0, ring_refused 0, life_ended 0 up to 02:21:02; the 02:21:32 line (resets 1, life_ended 2) is the coordinator's restart of the VM.
- Evidence saved by the coordinator: `/data/claude-diag/black-20261015-0220-qemu.log` (`chan token 0x80c DEAD: split walk ... did not apply`, display STALL chn 33/37).
- Candidates, ranked (INFERRED, not tested): (1) the OptiX denoise / large-tile kernel on a 8192^2 tile allocating or touching a large VRAM range (about 6.7+ GiB then more for film + denoise buffers) hits the kf3 split-walk page-table update that did not apply (the DEAD token line), so the channel stalled and the guest TDRed; (2) a long single kernel launch at 8192 tile size exceeding the 2 s TDR timeout through a slow path in kf3 (VM exits); (3) the display STALL on chn 1/33 is a consequence of the compute channel death, not the cause. Distinguishing runs (after restart, once): same step without `--denoise`; same step with tile 2048 and denoise.
- Not retried yet (instruction: wait until the coordinator reports the VM is back).

Earlier runs (a and b(1), b(2)) with the default 2048 tile and no denoise were clean: 0 host Xid, 0 GSP resets, 0 guest events.
