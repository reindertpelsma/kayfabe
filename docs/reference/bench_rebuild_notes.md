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
| 7 | ✅ **CLOSED at E1 (`853a311`).** A **failed** real isolate was indistinguishable from the stillborn one at the seam | `crates/kayfabe-isolate-host/src/isolate.rs:916` returns `HostIsolate::stillborn` on any build error. `Isolate::refusal` now answers with a **kind** — `spawn-failed` vs `no-plane` — and the device prints it at teardown. `[measured]` run `e0bfail1`, RTX 3060 / 580.159.04 open, fault injected at the host with `sysctl -w user.max_user_namespaces=0`: `isolates: 1 materialized, 1 live, 1 refusing (0 no-plane, 1 spawn-failed)` + `isolate refusal [spawn-failed] spawning the embedded isolate: clone failed (errno 28)`, against a control arm of the **same archive** that printed `(1 no-plane, 0 spawn-failed)`. ⊘ From outside the process the two boots are identical: `rc=0`, zero children, the same `RmInitAdapter` wall |
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

---

## 2026-08-01 — REBUILD #3, a THIRD machine. ★ Full parity, in about an hour.

**Why it happened.** Hardware measurement and bench boots were *strictly serial* on one box. A
second GA106 was rented so they stop being. The mandate was to stop at **a working host driver
+ a running `kayfabe-rm-ladder`** and to add the bench *only if it turned out cheap* — with §1's
recipe already written, it did (§D), so this box ended up at **full parity with box 1**.

⚠ **Naming.** The ssh alias for this machine is **`vb2`**, which collides with REBUILD #2's
*boot* names `vb1`/`vb2` (two boots on **one** box, instance 46494693). Throughout this section
"box2" means the machine and `vb1`/`vb2` keep their §3 meaning as boots. The committed evidence
is therefore `rm-ladder-box2-*.out`, never `…-vb2-…`.

### A. Provenance, stated first

| | |
|---|---|
| box | vast.ai instance **46529600**, `ssh -p 27014 root@184.144.255.144`, RTX 3060 (GA106) at `00:08.0`, **21 cores / 49 GB / 187 GB free**, Ubuntu 22.04 host, kernel **6.8.0-59-generic**, nested (42 `vmx\|svm` CPUs, `/dev/kvm` present) |
| host driver, as rented | **575.51.03 CLOSED** (`NVRM version: NVIDIA UNIX x86_64 Kernel Module 575.51.03`), apt/dpkg-managed, apt-`hold`ed |
| host driver, after §B | **580.159.04 Open Kernel Module**, `license: Dual MIT/GPL`, `nvidia-smi` healthy |
| source revision measured | **`6e4f66f`** (= `origin/master` at the time), clean |
| hypervisor | QEMU **10.2.4** + `scripts/build_qom_shim.sh` overlay, stamped `kayfabe-rev:6e4f66f5bdcf…` |
| guest | Ubuntu 24.04 noble cloudimg, kernel **6.8.0-136-generic**, **stock unpatched** 580.159.04 open module built in-guest |
| wall time, blank → `rm-ladder` exit 0 | **~23 min**, of which ~7 min was the 397 MB `.run` download |
| wall time, blank → first captured bench boot | **~55 min** |

⚠ The box arrives on **`00:08.0`**, not REBUILD #2's `00:07.0`. Nothing in this phase cares, but
anything that hardcodes a BDF will.

### B. The host-driver swap — the recipe in §2 is CORRECT and it worked FIRST TRY

★★ **The apt-hold trap of §2 reproduced exactly**, on a different instance of the same template.
**[measured]** 2026-08-01 on instance 46529600, `apt-mark showhold` **before** touching anything
returned **nine** held packages:

```
libnvidia-cfg1-575  libnvidia-common-575  libnvidia-compute-575  libnvidia-decode-575
libnvidia-encode-575  libnvidia-extra-575  libnvidia-fbc1-575  libnvidia-gl-575
nvidia-driver-575
```

⇒ §2's trap is a **property of the "Ubuntu 22.04 VM" template, not of one rental**. Because the
`apt-mark unhold` line was run *first*, the purge exited **0** and the `.run` installed with **no
`alternate driver installation` error at all** — the failure §2 documents never appeared. ★ That
is the whole value of that section: it converted a ~1 h debugging episode into one line.

⚠ **Two refinements §2 does not carry**, both **[measured]** here:

1. `apt-mark showhold` is the *only* honest input to the unhold. §2's `apt-mark unhold $(apt-mark
   showhold …)` is right precisely because the held set is **not** the set you would guess: it is
   nine packages, and it does **not** include `nvidia-dkms-575`, `nvidia-utils-575`,
   `nvidia-kernel-common-575` or `nvidia-firmware-575-575.51.03` even though all four are
   installed and all four must be purged. A hand-written unhold list would have missed nothing
   here only by luck — derive the list, never type it.
2. **`rmmod` the modules before the purge, not after.** The purge runs `update-initramfs` twice
   and `nvidia-persistenced` may hold `/dev/nvidia*`. The sequence that worked, verbatim:
   ```sh
   systemctl stop nvidia-persistenced
   rmmod nvidia_uvm nvidia_drm nvidia_modeset nvidia      # order matters: dependents first
   apt-mark unhold $(apt-mark showhold | tr '\n' ' ')
   apt-get purge -y --allow-change-held-packages nvidia-driver-575 nvidia-dkms-575 \
       nvidia-kernel-source-575 nvidia-kernel-common-575 nvidia-compute-utils-575 \
       nvidia-utils-575 'libnvidia-*-575' xserver-xorg-video-nvidia-575 \
       nvidia-firmware-575-575.51.03 nvidia-settings
   rm -rf /var/lib/dkms/nvidia /usr/src/nvidia-575.51.03
   sh NVIDIA-Linux-x86_64-580.159.04.run --silent --no-x-check --no-nouveau-check --dkms -m=kernel-open -j8
   ```
   Survivors, and they are the right survivors: `nvidia-container-toolkit{,-base}`,
   `libnvidia-container{-tools,1}`, `nvidia-modprobe`.

**[measured]** on `vb2` (vast instance **46529600**, RTX 3060 GA106, host 580.159.04 open) at
rev `6e4f66f`, 2026-08-01 — verification on **content**, not on exit codes:

```
NVRM version: NVIDIA UNIX Open Kernel Module for x86_64  580.159.04  Release Build …
modinfo nvidia → version 580.159.04, license Dual MIT/GPL,
                 filename /lib/modules/6.8.0-59-generic/updates/dkms/nvidia.ko
dkms status    → nvidia/580.159.04, 6.8.0-59-generic, x86_64: installed
nvidia-smi     → NVIDIA GeForce RTX 3060, 580.159.04, 0MiB / 12288MiB, 00000000:00:08.0
```

★ And the `open()` acceptance §2 insists on, with a 12-line C probe rather than `ls`:

```
/dev/nvidiactl           OPENED O_RDWR
/dev/nvidia0             OPENED O_RDWR
/dev/nvidia-uvm          OPENED O_RDWR
```

⇒ **no `EIO`** — this GPU completes GFW boot, unlike the project's `vh`
(`kgspWaitForGfwBootOk_TU102: failed to wait for GFW boot complete: 0x65`). ⚠ **The
pre-swap 575 driver already proved this**: `nvidia-smi` listed the card *as rented*, before
anything was changed. **Check GFW health on the driver the box arrives with** — it costs one
command and it decides whether the next 20 minutes are worth spending.

★ **Stock-module cleanliness, verified rather than assumed** — this box has never carried an
instrumented module and the check says so:

```
nm -a on nvidia{,-modeset,-drm,-uvm}.ko, egrep -ic 'kfv_|kayfabe|nvkvm|instrument|kf_probe'
    → 0, 0, 0, 0
same pattern over /proc/kallsyms → 0
/sys/module/{nvidia,nvidia_uvm,nvidia_modeset,nvidia_drm}/taint → OE  (O=out-of-tree, E=unsigned)
```

⚠ **and one thing that looks like a failure and is not.** `/proc/sys/kernel/tainted` reads
**12289** = `1 | 4096 | 8192`, and bit 0 is `TAINT_PROPRIETARY_MODULE`. There is **no
proprietary module loaded** — `grep -l P /sys/module/*/taint` returns **nothing**. The global
word is **sticky**: bit 0 was set by the **575 closed** driver that autoloaded at boot, before
the swap, and the kernel never clears it. ⇒ **Read per-module `taint`, never the global word,**
when the question is "is the open module loaded". A reboot would clear it; nothing else will.

### C. ★★★ `kayfabe-rm-ladder` on a second GA106 — ONE LINE of 33 differs, and it is a handle

Built on the box itself (⊘ never on the 4-core dev box): `rustup` stable + `rustup target add
x86_64-unknown-linux-musl` + `apt install musl-tools`, then `cargo build --release --bin
kayfabe-rm-ladder`. ⚠ **That build is 8 crates and 10.2 s** — the tree has essentially no
external dependencies, so a build that finishes implausibly fast is *correct*, not cached. The
`build.rs` line `embedded isolate image: 680608 bytes, x86_64-unknown-linux-musl` is the proof
it did the real work.

**[measured]** `./target/release/kayfabe-rm-ladder --gpu 0 --engines`, rev `6e4f66f`, host
580.159.04 open, **exit 0**, full transcript `bench_evidence/rm-ladder-box2-6e4f66f.out`:

```
★     R14 device memory   = wrote 0xa5a51234/0x5a5aedcb through mapping A, read both back through an INDEPENDENT mapping B
★     R15 SEM LANDED      = sem 0xbeef5ea1 (want 0xbeef5ea1), GP_GET 1 -> caught GP_PUT 1
★     R17 CE COPY         = 4096 bytes: dst[0] 0x3f0011ff -> 0xc0ffee00, dst[last] 0xc0fff1ff (want 0xc0fff1ff)
★     R13b VERDICT        = 4 DISTINCT runlists {0, 1, 2, 8} — engineType routes
★     R16 sandboxed doorbell = the capability-less isolate CPU-mapped the ring, USERD and the usermode BAR0 window, and rang channel 0xcafe000c token 0x00000004
```

★★★ **And then the thing no single box could establish.** `diff bench_evidence/rm-ladder-419afe8.out
bench_evidence/rm-ladder-box2-6e4f66f.out` — two **different physical GA106s**, two **different
source revisions** — is **one line**:

```
2c2
< ok    R4 hClient         = 0xc1d000c0        (box 1, rev 419afe8)
> ok    R4 hClient         = 0xc1d00034        (box 2, rev 6e4f66f)
```

`hClient` is **RM-allocated**, and the *same* one-line diff appears between **two consecutive
runs of the same binary on the same box** — **[measured]** on `vb2` (vast instance **46529600**,
RTX 3060 GA106) at rev `6e4f66f`, 2026-08-01: `diff /tmp/a.out /tmp/b.out` after
two back-to-back invocations gave exactly `< 0xc1d0006f` / `> 0xc1d0007a` and nothing else. So
it is the transcript's one legitimately non-deterministic field, and the cross-machine diff is
**not larger than the within-machine one**. Everything else — the semaphore value,
both CE-copy words, the channel tokens `0x00000004`/`0x00000005`, all eight `R13b` engineType
rows, the runlist set `{0,1,2,8}`, the `Other(87)` refusals for COPY(5..7) — is **byte-identical
across machines**. ⇒ **[inferred], and it is the whole point of the box:** the RM bring-up
ladder is a *deterministic* oracle up to handle allocation, so `diff`-ing a future ladder
transcript against a committed one is a real regression test, with exactly one line to mask.

⚠ **Expected `dmesg` noise, so nobody debugs it later.** The `R13b` sweep deliberately asks for
`NV2080_ENGINE_TYPE_COPY(5..7)` on a part that has five, and the driver logs
`nvAssertOkFailedNoLog … kfifoEngineInfoXlate_HAL … kernel_fifo_gm107.c:368` for each. The
ladder reports those as `info … refused Other(87)` and still exits 0. **The NVRM assertions in
the ring buffer after an `--engines` run are the ladder working.**

**R12 reproduces §5b(a) on independent hardware.** `--concurrency`, transcript
`bench_evidence/rm-ladder-concurrency-box2-6e4f66f.out`:

```
ok    R12 1 thread (base)  = 800 verbs sequential, 1545 ms
ok    R12 one client       = 456 overlapping pairs, 1645 ms
ok    R12 4 clients        = 455 overlapping pairs, 1602 ms
★     R12 SPEEDUP          = one client x4 workers: 0.94x  |  4 clients: 0.96x   (ideal 4.00x)
```

vs box 1's `0.93x / 0.94x` on **15** cores where this box has **21**. ⇒ §5b(a)'s claim — RM's
lock is **device-wide, not per-client**, and an isolate pool buys **no** verb throughput — is now
**[measured] on two machines with different core counts**, which removes "it was that box's
scheduler" as an explanation.

### D. ★★ The bench got built too — 21 cores made it cheap, so §C is not the end

The task that provisioned this box descoped the hypervisor/guest ("stop at a working host
driver unless the bench is cheap"). With §1's recipe in hand and 21 cores it was cheap —
**~25 min wall, both tracks concurrent, no debugging** — so it was built. Deltas from §1:

| track | what changed on this box | time |
|---|---|---|
| 1 host deps | §1's apt list verbatim, plus `qemu-system-x86 ovmf` up front; QEMU 10.2.4 + noble cloudimg downloaded in parallel with it | ~4 min |
| A shim | `scripts/build_qom_shim.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build`, `CARGO_BUILD_JOBS=16` | **~6 min** (vs §1's 13 on 15 cores) |
| B1 guest disk | as §1. ★ The dual-network netplan of §6 was written **into the cloud-init `user-data`** rather than applied afterwards — `write_files` for `/etc/netplan/01-bench.yaml` + the `99-disable-network-config.cfg`, and a `runcmd` that **deletes `/etc/netplan/50-cloud-init.yaml` then `netplan apply`**. That collapses §6's "write both files AND delete the cloud-init one" trap into the seed image and it worked first try | ~3 min |
| B2 guest driver | as §1: `build-essential linux-headers-$(uname -r)`, scp the same `.run`, `--silent --no-x-check --no-nouveau-check --no-questions -m=kernel-open -j8`, clean `poweroff` | ~9 min |

**[measured]** in-guest result, identical to §0's row: kernel **6.8.0-136-generic**, `modinfo
nvidia` → `version 580.159.04`, `license Dual MIT/GPL`, `vermagic 6.8.0-136-generic`,
`filename /lib/modules/6.8.0-136-generic/kernel/drivers/video/nvidia.ko`. **Stock, unpatched.**

⚠ **`boot_nvkvm.sh` and `gssh_nv` are NOT in the repo** — §7 lists them as box-1 files and they
were lost with that box's scope. They had to be **re-derived**, and that is a real gap: the
harness `scripts/bench/boot_capture.sh` hard-requires both (`die precondition`) and neither is
version-controlled. What was reconstructed here, and it works:

```sh
# gssh_nv
exec ssh -i /workspace/bench/guest_key -o StrictHostKeyChecking=no \
     -o UserKnownHostsFile=/dev/null -o ConnectTimeout=5 ubuntu@192.168.77.2 "$@"

# boot_nvkvm.sh <tag> [extra qemu args]
exec "$BENCH/qemu-build/qemu-system-x86_64" -enable-kvm -m 8G -smp 8 -machine q35 \
  -drive file="$BENCH/guest.qcow2",if=virtio,format=qcow2 \
  -netdev tap,id=n0,ifname=nvktap0,script=no,downscript=no -device virtio-net-pci,netdev=n0 \
  -device nvkvm-gpu,bar1-size=268435456,bar2-size=33554432,id=kf0 \
  -nographic -display none -monitor none \
  -serial file:"$BENCH/run_${TAG}_serial.log" "$@" 2> "$BENCH/run_${TAG}_qemu.log"
```

★ **These two files should be promoted into `scripts/bench/`.** A harness that refuses to run
without two uncommitted files re-invents them on every rebuild, and REBUILD #4 will otherwise
guess the device line again. ⊘ Not done here — this rebuild was doc-only by mandate.

### D2. ★★★ The boot, MEASURED at `6e4f66f` — and ⊘ what it may NOT be compared to

Two boots, `box2a` and `box2b`, one fresh QEMU each, rev `6e4f66f`, evidence
`bench_evidence/run_box2a_{dmesg,qemu,probe}.log`:

```
[boot_capture:box2a] captured 26 dmesg lines, 22 NVRM, 3 adapter
[   30.002476] NVRM: _memmgrMemUtilsScrubInitScheduleChannel: Unable to schedule channel, status: 56
[   30.029795] NVRM: RmInitNvDevice: *** Cannot load state into the device
[   30.991971] NVRM: GPU 0000:00:03.0: RmInitAdapter failed! (0x25:0xffff:1249)
```

`box2a` vs `box2b`: dmesg **IDENTICAL** with timestamps stripped, and the device's own report
**byte-identical** — `registers: 3630 reads / 56309 writes … interrupt requests dropped 91`,
`framebuffer: 122r/55106w`, `BAR2: 111r/21128w`, `commands: 92 decoded, 20 UNSERVICED`. ★ Note
this is *stronger* than §3's within-box pair, where the read counters drifted; here even the
polls matched.

⊘⊘ **This is NOT a cross-machine reproduction of §3, and must not be cited as one.** §3 was
rev `419afe8` and stops at `0x11:0x45:2134` (`RM_INIT_SYS_ENVIRONMENT_FAILED` /
`NV_ERR_IRQ_NOT_FIRING`) in **36** lines; this is rev `6e4f66f` and stops at `0x25:0xffff:1249`
in **26**. **Two variables moved at once** — the revision, and the fact that `boot_nvkvm.sh`
here is a *reconstruction* whose QEMU command line may not match box 1's (`-machine q35` is a
guess; box 1's line was never committed). ⊘ I did not run `419afe8` on this box, and I was not
permitted to touch box 1, so **the machine variable was never isolated**. Recording that
absence rather than the comparison it looks like.

★ What *can* be said, and it is worth something: the wall reached here is the one the project's
own current notes name — the failure is `memmgr`'s scrubber failing to schedule a CE channel,
**not** `IRQ_NOT_FIRING`, and `0x20800301` (`EVENT_SET_NOTIFICATION`) is **absent from the 17
distinct unserviced commands**, i.e. it is being serviced now. ⇒ **[inferred]** the boot has
advanced past §3's wall between `419afe8` and `6e4f66f`, and this independently-built bench
lands exactly where HEAD is expected to. That is a *consistency* check on the new bench, and a
good one; it is not a controlled comparison.

### E. Box state left as found

`/workspace/kayfabe` — the repo at `6e4f66f` on a local branch `bench` (fetched from a `git
bundle` scp'd in, since the box has no GitHub credentials), `target/release/kayfabe-rm-ladder`
built. `/workspace/bench` — `guest.qcow2` (provisioned, stock 580 open module installed, powered
down cleanly), `qemu-build/qemu-system-x86_64` **stamped `kayfabe-rev:6e4f66f5bdcf…`, verified
with `strings` on both it and `libkayfabe_qemu_raw.a`**, `guest_key{,.pub}`, `gssh` (slirp 2222,
provisioning), `gssh_nv` (tap, bench), `boot_prov.sh`, `boot_nvkvm.sh`, `seed.iso`,
`run_box2{a,b}_*` captures. `nvktap0` up at `192.168.77.1/24` with NAT (⚠ **not persistent —
re-run §6's five lines after any host reboot**). `/root`: `dl/NVIDIA-Linux-x86_64-580.159.04.run`
(**keep it** — a re-install after a kernel bump costs 7 min of download otherwise), and
`drvswap.sh`, `prov1.sh`, `guest1.sh`, `guest2.sh`, `mkboot.sh`, `openprobe.c` each with its log
beside it. **No QEMU running** (`pgrep -x qemu-system-x86` empty, `ss -tln` shows no 2222/2223).
Host driver **580.159.04 open, stock DKMS, zero instrumented symbols** (§B).

### F. §5b(b) re-measured — the `O_PATH` escape is closed on this box too

**[measured]** `kayfabe-sandbox-probe`, rev `6e4f66f`, exit 0,
`bench_evidence/sandbox-probe-box2-6e4f66f.out`. All ten traversal probes refused, `nvidiactl`
the only reachable `/dev` entry besides `null`, all five capability sets empty, `NoNewPrivs=1`,
`dumpable=0`, `Seccomp: 0`. Identical to §5b(b) — ⚠ including its caveat: the containment is
**namespace + capability + dirfd, not seccomp**, and this run says `Seccomp: 0` too.

⇒ **Every phase of §1's table now exists on this box.** It is equivalent to box 1 and can be
used for hardware measurement and for bench boots, in parallel with it.

---

## 2026-08-02 — the §8.2.2 measurement: a pushbuffer ring address is a GPU VA, not a GPA

**[measured]** rev `c93930d`, vast instance **46529600** (RTX 3060 / GA106 `10de:2504`,
21 cores / 49 GB, host driver **580.159.04 Open Kernel Module**), guest Ubuntu 24.04 with a
**stock unpatched** 580.159.04 open kernel module. Two boots through
`scripts/bench/boot_capture.sh`, one QEMU each, powered down between.

Both boots stop at the same wall as every boot since `5c1f501` —
`_memmgrMemUtilsScrubInitScheduleChannel … status: 56` → `RmInitAdapter failed! (0x25:0xffff:1249)`
— so nothing here is a claim about progress.

| tag | `-m` | guest's own `e820` | `nvkvm: gpfifo rings:` |
|---|---|---|---|
| `e5ring1` | `8G` | usable `0x1_0000_0000-0x2_7fff_ffff` | `1 declared, 1 with a non-zero address; first 0x0000000120064000 (4096 entries)` |
| `e5ring2g` | `2G` | usable to `0x7ffd_bfff`; **no entry above 4 GiB** | `1 declared, 1 with a non-zero address; first 0x0000000120064000 (4096 entries)` |

Files: `bench_evidence/c93930d_run_e5ring1_{qemu,dmesg,probe}.log`,
`bench_evidence/c93930d_run_e5ring2g_{qemu,dmesg,probe}.log`.

⇒ The address the guest names for its GPFIFO ring is **byte-identical across a 4× change in
guest RAM**, and at 2 GiB it names memory the guest does not have. It is a GPU virtual
address; it is not a guest-physical one. Full reading, and what it does and does not block,
in `docs/design/execution_plane_increments.md` §8.2.3.

### ★★ Two bench facts worth having next time

- **The differential is the instrument, not the single boot.** `[measured]` boots `e5ring1`
  and `e5ring2g` at rev `c93930d`: at 8 GiB the ring address `0x1_2006_4000` *is* a legal
  GPA, so a boot that only asked *"does `gpa_read` succeed?"* reads green. Changing `-m`
  is what separates a coincidence from an observation, and it costs one extra four-minute
  boot.
- ⚠ **`/workspace/bench/boot_nvkvm.sh` on this box had drifted to `-m 8G`** while
  `scripts/bench/boot_nvkvm.sh` in the tree says `-m 2048`. `boot_capture.sh` runs the
  **bench copy**, so the tree's value is not what boots. The 2 GiB run was taken by
  `sed -i "s/-m 8G/-m 2G/"` on the bench copy with a `.8g.bak` beside it, and the backup was
  restored afterwards — so the box is as it was found. ⊘ Do not read the repo's boot script
  as a description of what the bench ran; read the QEMU log's own
  `memory plane realized (bar0=… bar1=… bar2=…)` line, which moves with the RAM size
  (`bar1=0x280000000` at 8 GiB, `0xe0000000` at 2 GiB).
