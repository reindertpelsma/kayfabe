# kayfabe

**Share a consumer NVIDIA GPU with virtual machines that run the stock NVIDIA driver, while the host keeps using it.**

Kayfabe is a QEMU device that shows a guest an emulated NVIDIA GPU. The guest loads NVIDIA's real, unmodified driver (Windows or Linux);
kayfabe reads what that driver asks for and has the real host GPU do the work, from an **unprivileged** host process that talks to the host driver
as an ordinary client. Written in Rust.

**Status: research, 2026-10-17.** It runs real applications, but there is no installer, no stable interface, and no security audit yet.
Master is not the newest code: the latest Windows performance work lives on branches (see below).

![Windows 11 on kayfabe: desktop and nvidia-smi under the stock 580.88 driver (photo, 2026-10-09)](traces/windows_on_kayfabe.png)

## Why it is interesting

| | host keeps the GPU | guest driver | needs |
|---|---|---|---|
| VFIO passthrough | no, one VM owns it | stock | any GPU |
| NVIDIA vGPU | yes | vendor vGPU driver | data-centre GPU, licence |
| **kayfabe** | **yes** | **stock, unmodified** | a consumer GPU (Turing to Blackwell), the open NVIDIA host driver |

- **Nothing is patched.** No host or guest driver or kernel is modified; the guest uses NVIDIA's own installer.
- **The guest is treated as hostile.** Every host action is authored by kayfabe from a few unprivileged verbs, never forwarded from guest bytes.
- **Near-native speed is within reach.** On the same RTX 4070, Unigine Heaven in a Windows guest (1080p, Ultra):

  | | Direct3D 11 | OpenGL |
  |---|---|---|
  | native Windows (VFIO, earlier campaign) | 330 fps | 342 fps |
  | kayfabe, production path | 89 to 102 fps | 211 to 233 fps |
  | kayfabe, experimental build (not on master) | 296 to 299 fps | 353 to 359 fps |

  The experimental build skips reading the guest's command buffers once a channel is set up. It is research code, run on Heaven only; the safe version of the
  same idea is still being validated. Details and limits: [`traces/perf_status_20261017/`](traces/perf_status_20261017/README.md).
  On Linux guests, large GPU kernels and copies run at host speed (matrix multiply 0.997, llama.cpp prompt processing 0.998, token generation 0.88 of host,
  on a rented nested box at older revisions; [`details`](docs/design/V3_LINUX_PERF_BASELINE_6692e621.md)).

## What runs today

- **Windows 11:** Minecraft, Unigine Heaven (Direct3D 11 and OpenGL), GravityMark (Direct3D 12 and Vulkan), Edge with 4K60 hardware-decoded video
  ([matrix](traces/win11_userring_20261014/matrix-eb9f2aa3-20261014-2321.md)). Reported by the owner, evidence not yet committed: SolidWorks, Blender.
- **Linux:** the 30-arm test suite passes, a desktop with display and suspend/resume, CUDA applications (61 of 65 in an earlier run).
- Verified on an RTX 4070 (AD104); other dies are partly tested ([`docs/STATUS_DETAIL.md`](docs/STATUS_DETAIL.md)).

**Not finished:** Windows sleep/wake does not survive; Edge can reset after 7 to 16 minutes of playback; other GPU dies need more work under Windows; the
security model is designed but not audited, so no public binary ships. The full list is in [`docs/STATUS_DETAIL.md`](docs/STATUS_DETAIL.md).

## Try it

Needs Linux with KVM, the NVIDIA **open** kernel module, QEMU 10.2.4 with the overlay in `qemu/hw/misc/kf3`, and Rust (pinned in `rust-toolchain.toml`).

```sh
cargo build --workspace --release
cargo test --release $(ls crates | grep '^kf-' | sed 's/^/-p /')      # GPU-free tests
bash scripts/bench/v3_gates.sh                                         # on a box with a GPU, no QEMU: expect 9/9
```

Windows 11 guest: `scripts/bench/windows/win_vm.sh` ([`README`](scripts/bench/windows/README.md)). Renting a test box: [`scripts/bench/box/README.md`](scripts/bench/box/README.md).

## Learn more

[`ARCHITECTURE.md`](ARCHITECTURE.md) (one page) | [`docs/design/`](docs/design/THE_ARCHITECTURE_v3.md) | [`docs/STATUS_AND_HANDOFF.md`](docs/STATUS_AND_HANDOFF.md) |
[`docs/OWNER_RULINGS.md`](docs/OWNER_RULINGS.md) | [`docs/FAQ.md`](docs/FAQ.md) | [`docs/PROJECT_HISTORY.md`](docs/PROJECT_HISTORY.md) | [`AGENTS.md`](AGENTS.md) (rules for contributors)

Dual-licensed **Apache-2.0 OR GPL-2.0-or-later** ([`LICENSE`](LICENSE) lists exceptions; a QEMU binary built from this tree is not yet distributable under the GPL).
Provenance policy: [`docs/design/V3_PROVENANCE.md`](docs/design/V3_PROVENANCE.md). Security: not audited; report a suspected vulnerability to the owner privately through GitHub.
