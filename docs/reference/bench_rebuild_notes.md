# Rebuilding the kayfabe bench from nothing — the first-person log

**What this file is.** The `kayfabe` equivalent of the C artifact's
`/workspace/nvidia-gpu-passthrough/docs/BENCH_REBUILD_NOTES.md`: a first-person record of what it
actually took to stand a bench up on a blank rented box, written so the *next* rebuild is cheap.
★ **Read it before rebuilding a bench; do not re-derive it.** Where it disagrees with a design
doc, this file wins and the design doc gets amended — it records behaviour, not intent.

★★ The C file remains authoritative for the **C** bench (`/opt/qemu-nvkvm`, `nvkvm_gpu_emul.c`,
9p shares, `mode2-overlay.qcow2`). This one is authoritative for the **kayfabe** bench
(`/workspace/bench`, QEMU 10.2.4 + the QOM shim overlay, tap networking, one `guest.qcow2`).
They are different benches; only the host-side driver phase is shared, and that phase is where
the C file's traps still bite.

Tags carry the meanings `rm_semantics_measured.md` §0 already defines for them, and this file
does not restate them — restating the legend is how a doc acquires a claim word with no run
behind it. Every **[measured]** below names its boot, its box and its revision inline.

---

## 2026-08-01 — REBUILD #2, on a second machine. ★ The first cross-machine reproduction.

**Why it mattered.** Until this rebuild, every kayfabe boot had run on a box with **no GPU at
all**, so `docs/design/boot_measured_2026_08_01.md` had to say, four separate times, *"No host
GPU. This box has none, forwarding is off, and the isolate factory is `StillbornIsolates`."*
That made two questions unanswerable: is the boot wall a property of the port or of that one
machine, and is the forwarding half — `kayfabe-fwd`, 2304 lines, *"built — never exercised
against a real GPU"* — anything more than code that compiles.

### 0. Provenance, stated first

| | |
|---|---|
| box | vast.ai instance 46494693, RTX 3060 (GA106, `10de:2504`), 15 cores / 49 GB / 186 GB free, Ubuntu 22.04 host, itself a VM (nested KVM), GPU passthrough at `00:07.0` |
| host driver, as rented | **575.51.03 CLOSED**, apt/dpkg-managed |
| host driver, after phase 2 | **580.159.04 Open Kernel Module**, `license: Dual MIT/GPL`, `nvidia-smi` healthy |
| source revision | **`419afe8`**, clean (no `-dirty` suffix) |
| how the revision was verified | `strings … | grep -o 'kayfabe-rev:[0-9a-f-]*'` on **both** `target/release/libkayfabe_qemu_raw.a` **and** `qemu-build/qemu-system-x86_64` → `kayfabe-rev:419afe84f4c0da27a2b9489c7d21aa73dc194475` in each |
| hypervisor | QEMU **10.2.4** from `download.qemu.org`, `scripts/build_qom_shim.sh` overlay, rc 0 |
| guest | Ubuntu 24.04.4 noble cloudimg, kernel **6.8.0-136-generic**, **stock unpatched** NVIDIA 580.159.04 open kernel module built in-guest by the `.run` (`vermagic: 6.8.0-136-generic`, `license: Dual MIT/GPL`) |

⚠ `419afe8` is **not** the revision the previous box's `stateload2` boot ran (that was `7819839`,
a pre-rebase form of `0856958`). The delta `0856958..419afe8` touches three `crates/` files, and
the only non-comment hunk is two `x % n != 0` → `!x.is_multiple_of(n)` rewrites in
`crates/kayfabe-abi/src/gvaspacepdes.rs`. ⇒ **[inferred]** the two binaries are behaviourally
identical on the boot path, which is what makes §3's comparison meaningful rather than lucky.

### 1. The phases, in the order that worked

Nothing here is parallel *by accident*: the Rust/QEMU build (track A) and the guest-disk
provisioning (track B) genuinely are independent and were run concurrently. Everything else is
serial.

| # | phase | what | wall time |
|---|---|---|---|
| 1 | host deps | apt: `build-essential ninja-build meson python3-{venv,pip,tomli} pkg-config libglib2.0-dev libpixman-1-dev flex bison qemu-utils cloud-image-utils genisoimage …`; rustup stable; three parallel downloads (QEMU 10.2.4 tarball, noble cloudimg, the 397 MB NVIDIA `.run`) | ~4 min |
| 2 | **host driver 575 closed → 580.159.04 open** | see §2 — this is the phase with the trap | ~5 min |
| 3 | bench tree | `apt install qemu-system-x86 ovmf musl-tools`; untar QEMU; `git clone` the repo from a bundle; `rustup target add x86_64-unknown-linux-musl`; generate `guest_key` | ~2 min |
| A | Rust + shim | `scripts/build_qom_shim.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build`, `CARGO_BUILD_JOBS=10` | ~13 min |
| B1 | guest disk | cloudimg → `guest.qcow2` +30 G; `cloud-localds` seed; boot on the **stock** hypervisor with slirp `hostfwd 2222` | ~3 min |
| B2 | guest driver | `apt install build-essential linux-headers-$(uname -r)`; scp the `.run`; `--silent --no-x-check --no-nouveau-check --no-questions -m=kernel-open -j4`; clean `poweroff` | ~9 min |
| L | rm-ladder | `cargo build --bin kayfabe-rm-ladder` | ~4 min |
| 4 | boot | `scripts/bench/boot_capture.sh vb1`, then `vb2` | ~2 min each |

**Total, blank box → first captured boot: about 45 minutes.**

### 2. ★★ The host-driver trap the C recipe does not have — apt HOLDS

The C file records that the 580 `.run` *"REFUSED to install because the vast box ships the 575
driver via apt/dpkg"*, and prescribes purging the apt set first. On this template that purge
**fails**, and it fails in a way that looks like it worked:

```
apt-get purge -y nvidia-driver-575 nvidia-dkms-575 'libnvidia-*-575' …
  E: Held packages were changed and -y was used without --allow-change-held-packages
```

`apt-get` exits **100**, nothing is removed, and the `.run` then does exactly what the C file
predicts — `ERROR: The installation was canceled due to the availability or presence of an
alternate driver installation` — so the visible symptom is the *documented* one and points at
the wrong cause. **[measured]** 2026-08-01 on vast instance 46494693 (RTX 3060, host
575.51.03 as rented): the whole `nvidia-*-575` set is `apt-mark hold`-ed on the
"Ubuntu 22.04 VM" template (`--template_hash b7942f6bbc4374893ff66eb78145bbac`).

**The fix, and it is one line before the purge:**

```sh
apt-mark unhold $(apt-mark showhold | tr '\n' ' ')
apt-get purge -y --allow-change-held-packages nvidia-driver-575 nvidia-dkms-575 \
    nvidia-kernel-source-575 nvidia-kernel-common-575 nvidia-compute-utils-575 \
    nvidia-utils-575 'libnvidia-*-575' xserver-xorg-video-nvidia-575 \
    nvidia-firmware-575-575.51.03 nvidia-settings
rm -rf /var/lib/dkms/nvidia
sh NVIDIA-Linux-x86_64-580.159.04.run --silent --no-x-check --no-nouveau-check --dkms -m=kernel-open -j8
```

Keep `nvidia-container-toolkit` and `nvidia-modprobe` — harmless, and `nvidia-modprobe` is what
creates `/dev/nvidia0` on first open. ⚠ `rmmod` may return non-zero on the second attempt
because the first already succeeded; that is not a failure, check `lsmod` instead of `$?`.

**Verify on content, never on the installer's exit code** — `/proc/driver/nvidia/version` must
say **"Open Kernel Module"**, not "Kernel Module":

```
NVRM version: NVIDIA UNIX Open Kernel Module for x86_64  580.159.04  Release Build …
modinfo nvidia → version: 580.159.04, license: Dual MIT/GPL
```

★ And then **`open()` the nodes**, because their existence is not the property that matters:
`/dev/nvidiactl`, `/dev/nvidia0`, `/dev/nvidia-uvm` all opened `O_RDWR` cleanly here. An
`open()` returning **EIO** means the GPU never completed GFW boot — a hardware state, not a
build error.

### 3. ★★★ The boot, MEASURED at rev `419afe8` — the SAME wall, line for line, on a second machine

Two boots, `vb1` and `vb2`, one fresh QEMU each, both at `419afe8`:

```
[boot_capture:vb1] captured 36 dmesg lines, 33 NVRM, 3 adapter → /workspace/bench/run_vb1_dmesg.log
[   28.977375] NVRM: RmInitAdapter: osVerifySystemEnvironment failed, bailing!
[   29.849016] NVRM: GPU 0000:00:03.0: RmInitAdapter failed! (0x11:0x45:2134)
```

`0x11:0x45:2134` = `RM_INIT_SYS_ENVIRONMENT_FAILED` / `NV_ERR_IRQ_NOT_FIRING` — the wall the
previous box reached. Three comparisons, all **[measured]** 2026-08-01 at rev `419afe8` on the
RTX 3060 box, against the previous box's `stateload2` capture:

| pair | verdict |
|---|---|
| `vb1` vs `vb2` (same box, two boots) | dmesg **IDENTICAL** with timestamps stripped; device report **byte-identical** |
| `vb1` vs the previous box's `stateload2` — dmesg | **IDENTICAL**, 36/36 lines, timestamps stripped. Different files (`md5` `7bb75694…` vs `0d37d27d…`), different absolute times, same content |
| `vb1` vs `stateload2` — the device's own report | **one line of eight differs**, and only in *read* counters: `registers: 3648 reads … gsp 347r … UNCLAIMED 752r` vs `3616 / 341r / 750r`. Writes `58443`, `faults 0`, `interrupt requests dropped 89`, the framebuffer/BAR2/command blocks and all 20 unserviced control ids are identical |

★ **The write plane is deterministic across machines; only the poll-read counts move.** That is
the same shape the C artifact recorded for its own captures (*"the guest's PTIMER/mailbox poll
loop desynchronises"*), and it is the right thing to diff on: **diff the writes, tolerate the
reads.**

⇒ **The boot wall is a property of the port, not of one machine.** `#98`'s warning (a Mode-2
symptom that was 1/3 one day and 9/9 the next on a bit-identical binary) is not in play for
*this* rung: 4 boots across 2 machines, 4 identical outcomes.

⊘ **What this does not establish.** Nothing here reaches `cuInit`. `nvidia-smi` in the guest
still prints *"No devices were found"* (`SMI_RC=6`), and `interrupt requests dropped 89` in the
device report is the wall named from our side — the device warns at realize time that it *"does
not deliver vectors yet"*.

### 4. ★★★ The forwarding half, exercised against a real GPU for the first time

`crates/kayfabe-isolate-host/src/bin/rmladder.rs` (`--bin kayfabe-rm-ladder`) is
**[src]** the only code path in the tree that issues a real NVIDIA RM ioctl
(`scripts/run_full_suite.sh:299-309` says so in its own comment). It had never been run.
**[measured]**, `419afe8`, host 580.159.04 open, **exit 0**, full transcript in
`bench_evidence/rm-ladder-419afe8.out`:

```
ok    R2 version         = "580.159.04"
ok    R9 host GPU VA     = 0x0000000200200000 (FIXED, as requested)
★     R14 device memory   = wrote 0xa5a51234/0x5a5aedcb through mapping A, read both back through an INDEPENDENT mapping B
★     R15 SEM LANDED      = sem 0xbeef5ea1 (want 0xbeef5ea1), GP_GET 1 -> caught GP_PUT 1 — the GPU consumed our ring and released our semaphore
★     R17 CE COPY         = 4096 bytes: dst[0] 0x3f0011ff -> 0xc0ffee00, dst[last] 0xc0fff1ff (want 0xc0fff1ff)
★     R13b VERDICT        = 4 DISTINCT runlists {0, 1, 2, 8} — engineType routes
ok    R10 isolate         = 4 workers
ok    R11 through-isolate = host GPU VA 0x0000000200200000, hMemory 0xcafe0006
★     R16 sandboxed doorbell = the capability-less isolate CPU-mapped the ring, USERD and the usermode BAR0 window, and rang channel 0xcafe000c token 0x00000004
```

**What that settles.** A real `HostIsolateFactory` isolate **can** be built, **can** be sandboxed,
and **can** drive a real GA106 to the point where the GPU consumes our pushbuffer and releases
our semaphore. The host execution plane is not hypothetical; `docs/design/host_execution_plane.md`'s
§3 acceptance ("validated against `/dev/nvidiactl` on the bench") is **met**.

⊘ **What it does not settle.** This is the ladder's own process talking to the driver. The
hypervisor is not in the picture, and no guest intent was translated. See §5.

### 5. ★★★ What actually stands between this box and a first FORWARDED operation

The gap is **wiring, not capability** — and it is small enough to enumerate. Every row is
`[src]` at `419afe8`.

| # | gap | citation |
|---|---|---|
| 1 | The shipped composition root installs a factory that refuses everything, unconditionally | `crates/kayfabe-qemu-raw/src/shim.rs:1304` — `StillbornIsolates::new("this build has no forwarding plane…")` |
| 2 | The hypervisor crate cannot even **name** the real factory | `crates/kayfabe-qemu-raw/Cargo.toml` has no `kayfabe-isolate-host`, no `kayfabe-fwd`, no `kayfabe-rt` |
| 3 | There is **no selector of any kind** — no cargo feature, no QOM property, no env var. `RmMode::Real` is reachable only as a Rust constructor argument | `crates/kayfabe-isolate-host/src/isolate.rs:132-143`; QOM props at `qemu/hw/misc/nvkvm/nvkvm.c:1593-1615` are `bar*-size`, `window-size`, `shareable-ram`, `msix-size`, `chip-device-id` — none touches forwarding |
| 4 | No verb/doorbell entry point exists on the shim ABI at all | the 18 `extern "C"` entries, `crates/kayfabe-qemu-raw/src/shim_unsafe.rs:628-1129`, are realize/region/BAR/window/regs only |
| 5 | Even wired, the shipped `Arch` refuses the data plane | `crates/kayfabe-chips/src/ga10x.rs:149` `decode_doorbell → None`; `:171-175` `gmmu()`/`gsp() → None` |
| 6 | `kayfabe-fwd` has no host egress — it terminates at the `kayfabe_isolate::Worker` port | `crates/kayfabe-fwd/Cargo.toml` |
| 7 | A **failed** real isolate is indistinguishable from the stillborn one at the seam | `crates/kayfabe-isolate-host/src/isolate.rs:916` returns `HostIsolate::stillborn` on any build error |
| 8 | ★ And the boot does not get far enough to *want* forwarding anyway: it stops at `RmInitAdapter` (§3), long before a channel exists | `bench_evidence/run_vb1_dmesg.log` |

★★ **Row 8 is the one that orders the work.** Rows 1-7 are a day of wiring; there is no point
paying for them until the emulated boot reaches a doorbell, because until then there is no guest
intent to forward. **[inferred]** the honest next rung is still `IRQ_NOT_FIRING` (interrupt
delivery), and the value of this box is that it can now measure *both* halves in one place the
day they meet.

⊘ And a caveat that must travel with any future forwarding claim: `docs/design/c_rust_trace_differential.md`
already records that **forwarding-mode traces are non-hermetic by construction** — `pci_dma_map`
is an uninstrumented channel, the host GPU DMAs into guest RAM behind every recorder. A green
forwarding diff will not mean what a green emulation diff means.

### 5b. ★★ Two things this box measured at rev `419afe8` that no other box could have

Both **[measured]** 2026-08-01 at rev `419afe8` on the RTX 3060 box, host 580.159.04 open.
Transcripts: `bench_evidence/rm-ladder-concurrency-419afe8.out`,
`bench_evidence/sandbox-probe-419afe8.out`.

**(a) RM serialization is DEVICE-WIDE, not per-client.** `kayfabe-rm-ladder --concurrency`
(rung R12, another binary path nothing had ever run) drives 4 threads × 200 `alloc_vaspace` +
`free` verbs:

```
ok    R12 1 thread (base)  = 800 verbs sequential, 1496 ms
ok    R12 one client       = 430 overlapping pairs, 1602 ms
ok    R12 4 clients      = 454 overlapping pairs, 1594 ms
★     R12 SPEEDUP         = one client x4 workers: 0.93x   |   4 clients: 0.94x   (ideal 4.00x)
```

The overlap counters prove the requests really were in flight together — 430 and 454
overlapping pairs — and the wall clock did not move. `rm_semantics_measured.md` records "RM
serializes ALL ioctls **per client**"; ★ that is **too weak**. Four *separate* RM clients get
**0.94x**, statistically the same as one client's 0.93x, so the lock is not the client's.

⇒ **[inferred], and load-bearing for the ExecPlane:** an isolate *pool* cannot buy verb
throughput on one GPU, at any client granularity. Its value is isolation and fault
containment, not parallelism, and any design that budgets N× from N workers is budgeting
against a measurement that says 1×. ⊘ This does **not** generalise to the data plane — in the
same 2026-08-01 run at rev `419afe8`, R15/R17 show the GPU consuming pushbuffers, which is a
different path from the ioctl one.

**(b) The `O_PATH` `/dev` escape is CLOSED here.** `o_path_dev_escape` records the C finding
that `openat(devfd, "../etc/shadow")` **opens**, and that the Rust re-opened it. On this box,
in the real sandboxed child, all eight traversal probes are refused:

```
SANDBOX ok
PROBE nvidiactl OPENED          PROBE kvm DENIED 2        PROBE mem DENIED 2
PROBE ../etc/shadow DENIED 2    PROBE ../root DENIED 2    PROBE ../proc/1/maps DENIED 2
PRIV eff=0…0 prm=0…0 inh=0…0 bnd=0…0 amb=0…0 nnp=1 dumpable=0
```

All five capability sets empty, `NoNewPrivs=1`, `dumpable=0`, and the *only* thing reachable
under `/dev` is `nvidiactl`. ⚠ `Seccomp: 0` — there is no syscall filter, so the containment
here is namespace + capability + dirfd, not seccomp. Say which one you mean when citing this.

### 6. Traps encoded, so the next rebuild does not pay for them again

- ★★ **apt holds** — §2. The symptom is the C file's documented `.run` refusal; the cause is not.
- ★★ `pgrep -x qemu-system-x86_64` **can never match** (`/proc/PID/comm` truncates to 15 chars).
  Use `pgrep -x qemu-system-x86` **and** `ss -tln`. `scripts/bench/boot_capture.sh` already does.
  ⊘ Never `pgrep -f` — it matches the script's own command line.
- ★ **The shim build is `--disable-slirp`** (`scripts/build_qom_shim.sh:62`), so it *cannot* do
  user networking and cannot provision a guest that needs `apt`. Install the distro
  `qemu-system-x86` and provision on that, on `hostfwd 2222`; only the bench boot uses the tap.
- ★ **The tap must exist and be NAT-ed before the first bench boot**, or `gssh_nv` fails and
  reads as "the guest never booted":
  ```sh
  ip tuntap add dev nvktap0 mode tap; ip addr add 192.168.77.1/24 dev nvktap0; ip link set nvktap0 up
  sysctl -w net.ipv4.ip_forward=1
  iptables -t nat -A POSTROUTING -s 192.168.77.0/24 ! -o nvktap0 -j MASQUERADE
  iptables -I FORWARD 1 -i nvktap0 -j ACCEPT; iptables -I FORWARD 1 -o nvktap0 -j ACCEPT
  ```
  `nvktap0` reads `DOWN`/`NO-CARRIER` with no QEMU attached. That is normal, not a fault.
- ★ **The guest needs one interface config that works on BOTH networks.** It is provisioned on
  slirp (`10.0.2.15`) and then booted on the tap (`192.168.77.2`). A netplan `ethernets` block
  matching `name: "en*"` with **both** `dhcp4: true` and `addresses: [192.168.77.2/24]`, plus
  `optional: true` so `systemd-networkd-wait-online` does not stall the boot, covers both.
  ⚠ Also write `/etc/cloud/cloud.cfg.d/99-disable-network-config.cfg` **and** delete
  `/etc/netplan/50-cloud-init.yaml` — the disable file lands too late to stop cloud-init writing
  it on the very first boot.
- ★ **`nvidia` autoloads in the guest.** `boot_capture.sh` records `WAS_LOADED` and handles it
  (`rmmod` → `dmesg -C` → cold `modprobe` → `nvidia-smi` to force the `open()`), which is exactly
  why the capture is interpretable. ⊘ Do not read the ring buffer without unloading first.
- ★ `--template_hash b7942f6bbc4374893ff66eb78145bbac` ("Ubuntu 22.04 VM") is **mandatory** when
  renting: there is no `--vm` flag, and a plain `--image` gives a Docker container with a working
  GPU and **no `/dev/kvm`**. The real ssh port comes from the instance's `ports` map
  (`22/tcp → …`), **not** from `direct_port_start`, which reports `-1`.
- ⊘ **Nothing on a vast box is durable.** The `bench_evidence/` directory next to this file is
  committed for exactly that reason: the box that produced it will be destroyed, and a captured
  log that lives only there is not evidence.

### 7. Bench state left as found

`/workspace/bench` on the box: `guest.qcow2` (provisioned, driver installed, powered down
cleanly), `qemu-build/qemu-system-x86_64` stamped `kayfabe-rev:419afe8…`, `kayfabe-wt` on branch
`realbench` at `419afe8` clean, `guest_key`/`guest_key.pub`, `gssh` (port 2222, provisioning) and
`gssh_nv` (tap, bench), `boot_nvkvm.sh`, `boot_prov.sh`, `BUILD_REV.txt`. **No QEMU running**
(verified with `pgrep -x qemu-system-x86` and `ss -tln`). The provisioning scripts are at
`/root/prov{1,2b,3}.sh`, `/root/guest{1,2}.sh`, `/root/build{A,L}.sh` with their logs beside them.
