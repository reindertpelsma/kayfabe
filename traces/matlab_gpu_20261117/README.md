STATUS: LIVE, 2026-11-17 (one run, one box, one die; text evidence only).

# First kf3 GPU test of the Linux proprietary image C (MATLAB R2026b), 2026-11-17

Run: Vast lease 61058718 (label `kf-matlabgpu-`), RTX 3080 (GA102, 10 GiB, Ampere), AMD Ryzen 7 3700X host, nested KVM, 12 GiB RAM, 15 vCPU.
Host driver: swapped by `provision_host_driver.sh` to 580.159.04 open module (box_provision.log). kf3 QEMU 10.2.4 built by `build_kf3.sh`
from branch `claude/matlab-gpu-test-20261117` at `42e3a62b` (that is the PR #457 head `8167abd1f` plus two script-only commits; no kf3 code changed).
Image: R2 `linux-proprietary/kfimg-matlab-20261117/image.zip`, 8 128 391 922 bytes, sha256 verified on the box (image_fetch.log: download 335 s, unpack 63 s).
Guest: offline clone (restrict=on, recorded clock), 2 vCPU, 4 GiB RAM, `fb-mb=4096`, kf3 device `guest-driver=<run's version>`. Identity checks: `boot*_clone_identity_check.txt` (values stripped).
Harness: `scripts/bench/linux/matlab_gpu_box.py` (fetch, driver, boot, install, guest), `matlab_gpu_prov.sh`, `matlab_gpu_test.m`; `propimage_linux.py run --gpu kf3` was added for this run.
The NVIDIA `.run` came from NVIDIA's public CDN on the box and was checked against NVIDIA's own `<file>.run.sha256sum`.

Falsifier, stated before the run: "MATLAB sees the kf3 GPU, gpuDevice returns the die name, and the gpuArray matmul/fft return correct values (cross-check vs CPU result)".

## Result

| Attempt | Guest driver | MATLAB | Evidence |
|---|---|---|---|
| 1 | 580.159.04 (= host; the task's rule) | `gpuDevice` error: "GPU computing in MATLAB requires a newer graphics driver" | attempt1_* |
| 2 | 590.48.01 (public .run, CUDA 13.1) | requested command prints `NVIDIA GeForce RTX 3080` and `1`; full test all PASS | attempt2_* |

MEASURED (attempt 2, same sealed image, fresh clone, same host and kf3 device, only the guest driver differs): the requested
`matlab -batch "g=gpuDevice; disp(g.Name); a=gpuArray.rand(4000); b=a*a; disp(gather(sum(b(:)))>0)"` returned `NVIDIA GeForce RTX 3080` and `1`
(26.8 s wall incl. MATLAB start). `matlab_gpu_test.m`: compute capability 8.6, 2000x2000 double matmul GPU vs CPU max relative error 4.1e-16 (cpu 0.262 s, gpu 0.078 s, not a benchmark),
fft2 1024x1024 complex vs CPU 1.6e-16 and ifft2 round trip 1.3e-15, single-precision sin^2 sum error 0, Deep Learning Toolbox `dlconv` and `fullyconnect` on `dlarray(gpuArray)` vs CPU 1.9e-7 and 9.2e-8. All PASS.
MEASURED: boot to ssh 41 s (attempt 1) and 36 s (attempt 2); driver build in the offline guest about 3.5 minutes each (`install-driver` verified modinfo and libcuda equal the requested version);
guest `nvidia-smi` lists the RTX 3080; host `dmesg` showed 0 Xid lines after both attempts; host GPU, `nvidia-smi` and the box stayed healthy.

Attempt 1 cause: MEASURED, MATLAB R2026b bundles CUDA runtime 13.1.80 (`libcudart.so.13.1.80`), and the guest's `nvidia-smi` of 580.159.04 reports CUDA 13.0 (attempt1_580_guest_nvidia_smi.txt),
590.48.01 reports 13.1. INFERRED (not separately proven, but attempt 2 fixes it with nothing else changed): MATLAB refuses a driver older than its CUDA runtime.
Bucket: our own config error (driver choice), not a kayfabe limitation. The rule "guest driver = host driver" cannot be applied to this image while the host is 580.x. The host 580.159.04 with a 590.48.01 guest is a pair the driver matrix already lists (ladder 4/4, docs/design/V3_DRIVER_MATRIX.md).

Not proven: the guest `nvidia-smi` UUID equal to the seed UUID was not checked (the boot probe runs before the driver exists); GPU use was not sampled from the host (matlab runs lasted seconds);
only RTX 3080 was tested (the owner's order 3060, 3090, 4090 was not followed: the only AMD 3060 offer does not exist, the Intel 3060 is outside the image's AMD CPU profile, and two boxes on the one AMD 3090 host never answered ssh in 20 and 10 minutes, ids 61055276 and 61057554, destroyed);
no long-running or multi-process GPU load; the guest dmesg carries NVRM assertion lines at driver load (`NV_ERR_NOT_SUPPORTED` from `kgraphicsCreateGoldenImageChannel` and others, in both attempts, counts in attempt*_guest_dmesg_nvrm.txt);
whether they are expected under kf3 is not established here (no native control run).

Cost (`vast_bill.py`): 61058718 0.12 USD, 61057554 0.04, 61055276 0.09, total about 0.25 USD (bandwidth lines not yet billed at the time). All three destroyed by id and absent from `show instances-v1`.
Presigned URL: one, TTL 4 h, held in a 0600 file on the box and deleted after the sha256 check; never logged.
