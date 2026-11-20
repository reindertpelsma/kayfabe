# What you can test

You can check kayfabe on your own machine. This part of the manual says what runs out of the box from a plain clone of the repository, how to choose what to run, and how to bring your own images. None of it is needed to use kayfabe.

Everything runs from the repository alone: no private repository, account or licence is needed for the open checks.

## What runs out of the box

| Check | Needs | How |
|---|---|---|
| GPU-free unit tests | the Rust toolchain ([Building from source](building.md)) | `cargo test -p kf-<crate>` for any `kf-*` crate |
| GPU gates (no VM) | one NVIDIA GPU, the open driver | `bash scripts/bench/v3_gates.sh [logfile]`; passing is `V3_GATES_EXIT pass=9 fail=0 ...` and exit status 0 |
| Host baseline, no VM | the same | `bash scripts/fastguest/bare_metal_suite.sh <tag> [budget] [arms...]`: 30 arms pass on the host's own driver |
| 30-arm thin guest | the kayfabe QEMU | `KF_DEVICE=kf3 bash scripts/fastguest/fast_suite.sh <tag> 180`; passing is `FAST_SUITE_PASS=30 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0 NOTRUN=0` |
| One thin-guest run | the same | `KF_ARMS="--timer --engines" bash scripts/fastguest/run_fast_guest.sh <tag> <budget-seconds>`; passing is `FAST_VERDICT=PASS` |
| CUDA ladder (full guest) | the kayfabe QEMU and a guest | `bash scripts/bench/cuda_ladder.sh host\|guest <tag> [reps] [rungs]`; the four rungs `cup2`, `cup3`, `cup8`, `cup8bench` pass |
| Open applications on Linux | a guest image | [open FFmpeg and DCGM rows](open-linux-rows.md), [Vulkan conformance subset](vulkan-cts.md), [open ML rows](open-ml-rows.md) |
| Open probes on Windows | a Windows guest | [open Windows rows](windows-open-rows.md) |
| All of the above in one run | a rented or owned machine | [Run everything in one go](full-standalone-run.md) |

Run GPU checks strictly one at a time on one machine: the scripts take a lock because two concurrent runs of the fast lane kill each other's QEMU. The scripts need a prepared test tree; the appendix of [Install](installing.md#appendix-optional-helper-scripts) lists what prepares one.

Rules the scripts apply, which are worth copying to any setup of your own:

- **A timeout is a crash.** The thin guest gets one hard budget for the whole run, from QEMU start to power-off.
- **A green run must have reached hardware.** The thin guest run fails if a guest channel was never forwarded to the GPU.
- **If bare metal passes and the guest fails, the defect is kayfabe's.** Run the host baseline first.
- **Watch the host's kernel log for Xid messages across a run.** A host GPU fault does not show up in the guest. `bash scripts/bench/host_xid_watch.sh <tag> -- <command>` runs a command and exits 6 if a new host Xid appeared.
- **Write a start marker and an exit line** for every job; a killed job and a running one look alike otherwise.

## How to choose what to test

Pick by what you care about:

- You changed the build or the device: the GPU-free tests, then the GPU gates, then the thin-guest suite.
- You want to know whether your card works: the GPU gates and the host baseline, then the CUDA ladder in a guest.
- You want application coverage with open software: the open rows listed above.
- You want one answer for everything: the standalone full run.

An interactive chooser that asks what to run does not exist yet; it is planned as a mode of the existing standalone runner rather than a new command.

## Bring your own images

Applications that cannot be redistributed (commercial software, a licensed Windows image) are tested on images you build yourself.

- [Proprietary test images](proprietary-test-images.md) says what the maintainers' private images are and what the repository does not contain.
- [Recreate the proprietary Windows image](recreate-the-proprietary-image.md) builds your own copy with your own accounts and licences.
- [Bring your own Windows images](run-windows-images.md) runs applications on images you built.
- A configuration file that names each image (Windows or Linux), where its archive is and which applications it holds, with a validator and a resolver, is described in [Proprietary test images](proprietary-test-images.md#bring-your-own-image-configuration). No image is named by a letter in the model.

## Licences

The Windows image that the open Windows checks download is Microsoft's evaluation edition; running the harness installs it from Microsoft's own download link and its unattended setup accepts Microsoft's terms on your behalf. A prompt that asks you first does not exist yet. NVIDIA's driver is assumed to be under the terms you accepted when you installed it on the host. Kayfabe is not affiliated with NVIDIA and redistributes no NVIDIA binary ([Credits and notices](credits.md)).
