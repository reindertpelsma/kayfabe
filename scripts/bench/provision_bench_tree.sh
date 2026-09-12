#!/usr/bin/env bash
# ★★★ PHASES 3/A/B of the bench rebuild — the hypervisor tree and the guest disk.
# Recipe: docs/reference/bench_rebuild_notes.md §1 (phases 3, A, B1, B2) + §6's traps.
# Run AFTER provision_host_driver.sh reports OPEN_MODULE=yes. ~25-45 min on 20+ cores.
#
# Tracks A (hypervisor+Rust) and B (guest disk) are genuinely independent and run concurrently;
# everything else is serial.
set -uo pipefail
BENCH=/workspace/bench
REPO=${KAYFABE_REPO:-/root/kayfabe}
QEMU_VER=10.2.4
RUN=/root/NVIDIA-Linux-x86_64-580.159.04.run
say(){ echo "[$(date -Is)] $*"; }
say "BENCH_TREE_START"

# ---- 0. preconditions, checked on CONTENT ----------------------------------------------
grep -q "Open Kernel Module" /proc/driver/nvidia/version 2>/dev/null \
  || { say "⊘ host driver is not the OPEN module -- run provision_host_driver.sh first"; exit 2; }
[ -s "$RUN" ] || { say "⊘ missing $RUN (the guest needs the SAME .run)"; exit 2; }
mkdir -p "$BENCH"

# ---- 1. host deps ----------------------------------------------------------------------
say "phase 3: host deps"
export DEBIAN_FRONTEND=noninteractive
for i in $(seq 1 60); do fuser /var/lib/dpkg/lock-frontend >/dev/null 2>&1 || break; sleep 10; done
apt-get install -y -qq ninja-build meson python3-venv python3-pip python3-tomli \
  libglib2.0-dev libpixman-1-dev flex bison qemu-utils cloud-image-utils genisoimage \
  qemu-system-x86 ovmf musl-tools libslirp-dev >/dev/null 2>&1
say "apt rc=$?  (ninja=$(command -v ninja||echo NO) meson=$(command -v meson||echo NO) cloud-localds=$(command -v cloud-localds||echo NO))"

# ---- 2. downloads, in parallel ----------------------------------------------------------
say "phase 3: downloads"
[ -s "$BENCH/qemu-$QEMU_VER.tar.xz" ] || \
  curl -fsSL -o "$BENCH/qemu-$QEMU_VER.tar.xz" "https://download.qemu.org/qemu-$QEMU_VER.tar.xz" &
P1=$!
[ -s "$BENCH/noble-server-cloudimg-amd64.img" ] || \
  curl -fsSL -o "$BENCH/noble-server-cloudimg-amd64.img" \
    "https://cloud-images.ubuntu.com/noble/current/noble-server-cloudimg-amd64.img" &
P2=$!
wait $P1; say "qemu tarball rc=$?  $(ls -la $BENCH/qemu-$QEMU_VER.tar.xz 2>/dev/null | awk '{print $5}')"
wait $P2; say "cloudimg rc=$?  $(ls -la $BENCH/noble-server-cloudimg-amd64.img 2>/dev/null | awk '{print $5}')"

# ---- 3. tap + NAT ----------------------------------------------------------------------
# ★ MUST exist and be NAT-ed before the first bench boot, or gssh_nv fails and the failure
#   reads as "the guest never booted". ⚠ NOT persistent across a host reboot.
say "phase 3: tap"
ip link show nvktap0 >/dev/null 2>&1 || ip tuntap add dev nvktap0 mode tap
ip addr add 192.168.77.1/24 dev nvktap0 2>/dev/null
ip link set nvktap0 up
sysctl -qw net.ipv4.ip_forward=1
iptables -t nat -C POSTROUTING -s 192.168.77.0/24 ! -o nvktap0 -j MASQUERADE 2>/dev/null || \
  iptables -t nat -A POSTROUTING -s 192.168.77.0/24 ! -o nvktap0 -j MASQUERADE
iptables -C FORWARD -i nvktap0 -j ACCEPT 2>/dev/null || iptables -I FORWARD 1 -i nvktap0 -j ACCEPT
iptables -C FORWARD -o nvktap0 -j ACCEPT 2>/dev/null || iptables -I FORWARD 1 -o nvktap0 -j ACCEPT
say "nvktap0: $(ip -br addr show nvktap0 2>&1)   (DOWN/NO-CARRIER with no QEMU attached is NORMAL)"

[ -s "$BENCH/guest_key" ] || ssh-keygen -q -t ed25519 -N "" -f "$BENCH/guest_key"
say "guest_key: $(ls -la $BENCH/guest_key | awk '{print $5}') bytes"

# ---- TRACK A: hypervisor + Rust archive ------------------------------------------------
track_a() {
  say "A: untar qemu"
  [ -d "$BENCH/qemu-$QEMU_VER" ] || tar -C "$BENCH" -xf "$BENCH/qemu-$QEMU_VER.tar.xz"
  say "A: build_qom_shim (this is the long pole)"
  . "$HOME/.cargo/env" 2>/dev/null
  CARGO_BUILD_JOBS=16 bash "$REPO/scripts/build_qom_shim.sh" \
      "$BENCH/qemu-$QEMU_VER" "$BENCH/qemu-build" > /tmp/trackA.log 2>&1
  echo "A_RC=$?"
  say "A: binary = $(ls -la $BENCH/qemu-build/qemu-system-x86_64 2>/dev/null | awk '{print $5}' || echo MISSING)"
  tail -5 /tmp/trackA.log
}

# ---- TRACK B: guest disk ---------------------------------------------------------------
track_b() {
  say "B1: guest disk"
  if [ ! -s "$BENCH/guest.qcow2" ]; then
    qemu-img convert -O qcow2 "$BENCH/noble-server-cloudimg-amd64.img" "$BENCH/guest.qcow2"
    qemu-img resize "$BENCH/guest.qcow2" +30G
  fi
  PUB=$(cat "$BENCH/guest_key.pub")
  # ★ §6's dual-network config, written INTO the seed rather than applied afterwards.
  #   metric 500 is load-bearing: on slirp the DHCP default route must keep winning; on the
  #   tap the static one is the only route. One config, both networks, no post-hoc edit.
  cat > "$BENCH/user-data" <<UD
#cloud-config
users:
  - name: ubuntu
    sudo: ALL=(ALL) NOPASSWD:ALL
    shell: /bin/bash
    ssh_authorized_keys: [ "$PUB" ]
ssh_pwauth: false
write_files:
  - path: /etc/netplan/01-bench.yaml
    permissions: '0600'
    content: |
      network:
        version: 2
        ethernets:
          bench:
            match: { name: "en*" }
            dhcp4: true
            optional: true
            addresses: [192.168.77.2/24]
            routes: [{to: default, via: 192.168.77.1, metric: 500}]
            nameservers: { addresses: [8.8.8.8, 1.1.1.1] }
  - path: /etc/cloud/cloud.cfg.d/99-disable-network-config.cfg
    content: |
      network: {config: disabled}
runcmd:
  - rm -f /etc/netplan/50-cloud-init.yaml
  - netplan apply
  - touch /var/lib/cloud/BENCH_SEED_OK
UD
  echo "instance-id: bench1" > "$BENCH/meta-data"
  cloud-localds "$BENCH/seed.iso" "$BENCH/user-data" "$BENCH/meta-data"
  say "B1: seed.iso = $(ls -la $BENCH/seed.iso | awk '{print $5}') bytes"

  # boot on the STOCK hypervisor with slirp; provisioning never uses the tap.
  say "B2: booting guest on stock qemu (slirp hostfwd 2222)"
  # ⚠ `-nographic` and `-daemonize` are MUTUALLY EXCLUSIVE ("-nographic cannot be used with
  #   -daemonize"). -nographic is a bundle that includes -serial stdio, which a daemonized
  #   process has no stdio for. Use `-display none` and let -serial file: carry the console.
  # ⚠ `-cpu host` IS REQUIRED, and its absence is silent until something needs x86-64-v2.
  # Measured 2026-09-06: without it QEMU picks `qemu64`, which lacks the v2 baseline, and the
  # guest's NumPy dies with
  #     RuntimeError: NumPy was built with baseline optimizations: (X86_V2)
  #                   but your machine doesn't support: (X86_V2)
  # ⊘ Nothing earlier fails -- apt, pip and the driver install are all fine on qemu64 -- so the
  # defect surfaces only at the first numeric import, far from its cause. `boot_nvkvm.sh` has
  # always used `-cpu host`; the PROVISIONING boots did not, so the guest was built on one CPU
  # model and benched on another.
  qemu-system-x86_64 -enable-kvm -cpu host -m 8G -smp 8 -display none \
    -drive if=virtio,file="$BENCH/guest.qcow2",format=qcow2 \
    -drive if=virtio,file="$BENCH/seed.iso",format=raw \
    -netdev user,id=n0,hostfwd=tcp::2222-:22 -device virtio-net-pci,netdev=n0 \
    -serial file:"$BENCH/provision_serial.log" -daemonize -pidfile "$BENCH/prov.pid"
  say "B2: qemu pid=$(cat $BENCH/prov.pid 2>/dev/null)"

  # ⊘⊘⊘ **`timeout` AND `-n`, because `ConnectTimeout` BOUNDS NOTHING HERE.**
  #
  # `[measured w443]` this loop wedged for **55 minutes** on a guest that was up, idle and
  # answering an interactive ssh the whole time. Provisioning had reached `TRACK_A done` and
  # then produced no output for 45 minutes; killing the one hung session by hand let the script
  # finish in seconds.
  #
  # ⚠ `ConnectTimeout` bounds the TCP CONNECT, and under slirp `hostfwd` the host side accepts
  # immediately whether or not anything is listening in the guest. So the connect always
  # succeeds and the SESSION is what hangs — the timeout it looks like it has is not a timeout
  # on the thing that failed. ⊘ And `-n` because an ssh sharing the script's stdin can block on
  # it forever; both are needed and neither is sufficient.
  #
  # ⇒ Bound the WHOLE session. A guest step that legitimately takes longer than this should say
  # so with its own longer bound, not by removing the bound.
  GS="timeout 120 ssh -n -i $BENCH/guest_key -p 2222 -o StrictHostKeyChecking=no \
      -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=8 \
      -o ServerAliveInterval=15 -o ServerAliveCountMax=3 ubuntu@127.0.0.1"
  # ★★★★★ **AND HERE IS THE STEP THAT NEEDED ITS OWN BOUND AND DID NOT HAVE ONE.**
  #
  # `[measured w474, 2026-09-11]` the driver install below ran under the 120 s `$GS` and was
  # **KILLED MID-INSTALL**. `/var/log/nvidia-installer.log` in the guest ended at
  #     -> Kernel module compilation complete.
  #     -> Unable to determine if Secure Boot is enabled: No such file or directory
  # with **zero ERROR lines** — the modules BUILT and were never INSTALLED, because the
  # installer's next phase (sign + copy into /lib/modules + depmod) is where the ssh died.
  # Re-run detached and timed on the same box: **~3.5 minutes**, `EXIT_STATUS=0`.
  #
  # ⊘ Every downstream signal misdirects. `modinfo nvidia` is EMPTY, which reads as "the build
  # failed"; the next boot reports `MODPROBE_RC=1`, which reads as a kernel mismatch; and the
  # phase's own hint says to check for `cc` — but `cc`, `gcc`, `make` and the headers were all
  # present and the compile had SUCCEEDED. A clean log with no error line is the signature of a
  # KILL, not of a failure: **look at where the log stops, not at what it says.**
  #
  # ⇒ A long guest step gets a LONG bound, per the rule three lines up. It does not get to
  # inherit the interactive one, and it does not get to have none.
  GSL="timeout ${GUEST_LONG_TIMEOUT:-1800} ssh -n -i $BENCH/guest_key -p 2222 -o StrictHostKeyChecking=no \
      -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=8 \
      -o ServerAliveInterval=15 -o ServerAliveCountMax=3 ubuntu@127.0.0.1"
  # ★ a guest needs ~20-25s to a login prompt and -serial output LAGS. A slow boot is not a crash.
  for i in $(seq 1 40); do
    $GS true >/dev/null 2>&1 && { say "B2: guest ssh up after $((i*10))s"; break; }
    sleep 10
  done
  $GS true >/dev/null 2>&1 || { say "⊘ B2: guest never answered ssh"; tail -20 "$BENCH/provision_serial.log"; return 3; }

  # ⚠⚠ SSH BEING UP IS NOT THE GUEST BEING READY, and the thing that breaks it is OUR OWN SEED.
  # Measured 2026-09-06: sshd answers at ~20s, but cloud-init's runcmd then runs `netplan apply`
  # -- which RESTARTS THE GUEST'S NETWORKING and drops every established session. The symptom is
  #   Connection to 127.0.0.1 closed by remote host
  #   kex_exchange_identification: Connection closed by remote host
  # in the middle of `apt-get`, i.e. it looks like a flaky network or a dying guest. It is
  # neither: it is the seed doing exactly what it was told, one step after we started using the
  # connection. ⇒ WAIT FOR CLOUD-INIT TO FINISH before touching the guest at all.
  say "B2: waiting for cloud-init to finish (it will bounce the network)"
  for i in $(seq 1 60); do
    st=$($GS "cloud-init status 2>/dev/null | head -1" 2>/dev/null | tr -d '\r')
    case "$st" in *done*) say "B2: cloud-init done after $((i*10))s"; break ;; esac
    sleep 10
  done
  $GS "test -f /var/lib/cloud/BENCH_SEED_OK" 2>/dev/null \
    && say "B2: seed marker present" \
    || { say "⊘ B2: BENCH_SEED_OK missing -- the seed's runcmd did not complete"; return 4; }

  # ★★★★★ **THE GUEST'S APT NEEDS THE SAME MIRROR FIX AS THE HOST'S, AND FOR THE SAME REASON.**
  #
  # `[measured w443]` this step ran for **55 minutes** and then failed with
  # `ERROR: Unable to find the development tool \`cc\` in your path`. The guest had no
  # compiler because THIS apt never finished: the cloud image ships `archive.ubuntu.com`, and
  # on this box that mirror served **13 kB/s** while another served **2.5 MB/s** — the same
  # 190x gap `provision_box.sh` already races around on the host side.
  #
  # ⊘ The failure surfaced as *"the driver installer wants gcc"*, which reads as a missing
  # package list, not as a slow mirror two layers down. And the boot AFTER it failed with
  # `MODPROBE_RC=1`, which reads as a driver/kernel mismatch. Three symptoms, one cause.
  #
  # ⚠ Fix the guest the same way, and MEASURE rather than hardcode — mirror speed is a
  # property of where the box is, not of the mirror.
  # ⊘⊘ **THIS MEASURED AND THEN IGNORED THE MEASUREMENT** — fixed 2026-09-12 (w537).
  #
  # It benchmarked three mirrors, printed the speeds, and then rewrote the guest's sources to
  # `mirrors.edge.kernel.org` **unconditionally**, whichever won. `[measured w537]` on a fresh
  # box that host was UNREACHABLE (`Unable to connect`) while `archive.ubuntu.com` — the one
  # being replaced — was fastest at 527 kB/s. The result: `build-essential` had no candidate,
  # the guest had no `cc`, the kernel-open driver never built, and the first graded boot came
  # back with an EMPTY guest dmesg and `do not grade this boot`.
  #
  # ⚠ A check that reports is not a check that gates. The host half of this provisioning
  # (`provision_box.sh:pick_apt_mirror`) has always picked the winner and refused below a
  # floor; this half printed the same numbers as decoration.
  # ⊘⊘ **THIS MEASURED AND THEN IGNORED THE MEASUREMENT** — fixed 2026-09-12 (w537).
  #
  # It benchmarked three guest apt mirrors, printed the speeds, and then rewrote the guest's
  # sources to `mirrors.edge.kernel.org` **unconditionally**, whichever won. `[measured w537]`
  # on a fresh box that host was UNREACHABLE while `archive.ubuntu.com` — the one being
  # replaced — was fastest at 527 kB/s. The chain: `build-essential` had no candidate, the
  # guest had no `cc`, the kernel-open driver never built, and the first graded boot returned
  # an EMPTY guest dmesg with the capture's own "do not grade this boot".
  #
  # ⚠ A check that reports is not a check that gates. The HOST half of this provisioning
  # (`provision_box.sh:pick_apt_mirror`) has always picked the winner and refused below a
  # floor; this half printed the identical numbers as decoration.
  #
  # ⊘ And the FIRST fix was wrong in its own way: it captured the winner with `2>&1`, which
  # merged the progress lines back into the value, so the mirror became a whole log line. The
  # speeds are emitted as parseable `MIRROR <host> <bytes>` rows and the winner is chosen
  # HERE, on the host, where it can be seen.
  say "B2: picking the guest's apt mirror (the host's fix, applied one layer down)"
  MIRROR_ROWS=$($GS "for m in archive.ubuntu.com/ubuntu azure.archive.ubuntu.com/ubuntu mirrors.edge.kernel.org/ubuntu; do
         s=\$(timeout 12 curl -s -o /dev/null -w '%{speed_download}' http://\$m/dists/noble/Release 2>/dev/null || echo 0)
         echo \"MIRROR \$m \${s%%.*}\"
       done" 2>/dev/null | tr -d '\r')
  echo "$MIRROR_ROWS" | sed 's/^/    /'
  GUEST_MIRROR=$(echo "$MIRROR_ROWS" | awk '$1=="MIRROR" && $3+0 >= 100000 {print $3, $2}' | sort -rn | head -1 | awk '{print $2}')
  if [ -z "$GUEST_MIRROR" ]; then
    say "⊘ B2: NO USABLE GUEST APT MIRROR (none reached 100 kB/s). The driver cannot build in the guest."
    return 5
  fi
  say "B2: guest apt mirror = $GUEST_MIRROR"
  $GS "sudo sed -i -E 's|http://[A-Za-z0-9.-]+/ubuntu|http://$GUEST_MIRROR|g' \
         /etc/apt/sources.list /etc/apt/sources.list.d/*.sources /etc/apt/sources.list.d/*.list 2>/dev/null; true"
  say "B2: installing guest driver (kernel-open)"
  $GSL "sudo apt-get update -qq && sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq build-essential linux-headers-\$(uname -r)" 2>&1 | tail -3
  # ⚠ assert the COMPILER before spending five minutes finding out it is missing. `cc` absent
  # here means the apt above never finished, which on a fresh cloud image is the mirror.
  CCOK=$($GS "command -v cc >/dev/null && echo CC_PRESENT || echo CC_MISSING" 2>/dev/null | tr -d '\r')
  say "B2: guest compiler = $CCOK"
  [ "$CCOK" = "CC_PRESENT" ] || { say "⊘ B2: no \`cc\` in the guest -- the apt above did not finish (mirror)"; return 5; }
  # ⊘ /tmp is a tmpfs on this image: a 400 MB .run there competes with the guest's RAM and does
  # not survive a reboot. /var/tmp is on disk.
  scp -i "$BENCH/guest_key" -P 2222 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
      -o LogLevel=ERROR "$RUN" ubuntu@127.0.0.1:/var/tmp/nv.run >/dev/null 2>&1
  # ⚠ ~3.5 min on a fast box, longer on a slow one -- this is the step w474 found truncated.
  $GSL "sudo sh /var/tmp/nv.run --silent --no-x-check --no-nouveau-check --no-questions -m=kernel-open -j8; echo NVRUN_RC=\$?" 2>&1 | tail -5
  # ⚠ verify on CONTENT, not the installer's exit code
  # ★★ cuda.h — bench_rebuild_notes.md §D item 2. cup3/cup8 compile against <cuda.h> IN THE
  # GUEST; without it the workload never builds and w297 reports (D) UNMEASURED. Omitting this
  # cost a full cup3 run on 2026-09-06.
  # ⚠ VERIFY BY CONTENT — `CUresult` x601, NOT an existence test. The guest already ships
  #   THREE other cuda.h files and all three are the PowerMac ADB header, so `test -f` passes
  #   on the wrong file and the compile then fails for an unrelated-looking reason.
  # ⊘ Do NOT write `tar -tf ... | grep -m1 ...` under `set -o pipefail`: grep matches, closes
  #   the pipe, tar dies of SIGPIPE (141), and the pipeline reports FAILURE OVER A SUCCESSFUL
  #   MATCH. Measured 2026-09-06 — cuda.h was in the archive and the script said it was not.
  if [ ! -s "$BENCH/cuda.h" ]; then
    curl -fsSL -o /tmp/cudart.tar.xz "${CUDART_URL:-https://developer.download.nvidia.com/compute/cuda/redist/cuda_cudart/linux-x86_64/cuda_cudart-linux-x86_64-12.6.77-archive.tar.xz}"
    tar -tf /tmp/cudart.tar.xz > /tmp/cudart.list
    awk -F/ '$NF=="cuda.h"{print; exit}' /tmp/cudart.list > /tmp/cudah.path
    if [ -s /tmp/cudah.path ]; then
      tar -C /tmp -xf /tmp/cudart.tar.xz "$(cat /tmp/cudah.path)"
      cp "/tmp/$(cat /tmp/cudah.path)" "$BENCH/cuda.h"
    fi
  fi
  if [ -s "$BENCH/cuda.h" ] && [ "$(grep -c CUresult "$BENCH/cuda.h")" -ge 500 ]; then
    scp -i "$BENCH/guest_key" -P 2222 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
        -o LogLevel=ERROR "$BENCH/cuda.h" ubuntu@127.0.0.1:/tmp/cuda.h >/dev/null 2>&1
    $GS "sudo cp /tmp/cuda.h /usr/include/cuda.h && sudo chmod 644 /usr/include/cuda.h"
    say "B2: guest cuda.h CUresult x$($GS 'grep -c CUresult /usr/include/cuda.h' 2>/dev/null | tr -d '\r')  ⇒ MUST be >= 500"
  else
    say "⊘ B2: no usable cuda.h -- cup3/cup8 will report (D) UNMEASURED, not a failure value"
  fi

  MI=$($GS 'modinfo nvidia 2>/dev/null | grep -E "^version|^license|^vermagic" | tr "\n" " "' 2>&1)
  say "B2: guest modinfo = $MI"
  # ⊘ Do NOT power off over a failed install: a clean shutdown makes the failure look like a
  #    completed phase, and the next step boots a guest with no driver in it.
  case "$MI" in
    *580.159.04*) say "B2: guest driver VERIFIED on content" ;;
    *) say "⊘ B2: guest driver NOT verified -- leaving the guest UP for inspection"
    say "   ⚠ FIRST check the guest has a COMPILER: \`cc\` missing means this step's apt never"
    say "     finished, which on a fresh cloud image is almost always the mirror. The next boot"
    say "     will report MODPROBE_RC=1, which reads as a kernel mismatch and is not one."
    say "   ★★ SECOND -- and this is what actually happened in w474 -- read WHERE the guest's"
    say "     /var/log/nvidia-installer.log STOPS, not what it says. It ends at"
    say "     \`Kernel module compilation complete\` with ZERO error lines when the driving ssh"
    say "     was killed mid-install: the modules built and were never installed. A clean log"
    say "     with no error line is the signature of a KILL, not of a failure."
    $GS "tail -3 /var/log/nvidia-installer.log" 2>&1 | sed 's/^/     installer-log-tail: /'
    return 5 ;;
  esac
  $GS "sudo poweroff" >/dev/null 2>&1 &
  sleep 20
  say "B2: guest powered off (qemu alive? $(kill -0 $(cat $BENCH/prov.pid 2>/dev/null) 2>/dev/null && echo yes || echo no))"
}

track_a > /tmp/trackA.out 2>&1 &
A=$!
track_b > /tmp/trackB.out 2>&1 &
B=$!
wait $A; say "TRACK_A done"
wait $B; say "TRACK_B done"
echo "----- TRACK A -----"; cat /tmp/trackA.out
echo "----- TRACK B -----"; cat /tmp/trackB.out
# ★★★★★ **BUILD THE GUEST-SIDE CLIENT, or the rung that grades this whole project cannot run.**
#
# `[measured w443]` a fully provisioned bench — driver verified, guest booting, MODPROBE_RC=0,
# all three invariants reporting real numbers — produced
# `W392D_GUEST_OUTCOME=(N) ⊘ UNMEASURED_NO_BINARY`. The mean client is a **musl** binary
# (`kayfabe-rm-ladder`, static, copied into the guest) and provisioning added the musl TARGET
# without ever building it.
#
# ⊘ The hook is honest about it — `(N)` is UNMEASURED, not a failure — which is exactly why it
# is easy to miss: a boot that measured NOTHING and a boot that measured a pass differ by one
# letter in one line, and every other line in that ledger was full of real numbers.
say "B4: building the guest-side mean client (musl)"
# ⊘ `[measured w474]` this step failed with `cargo: command not found` and the script CARRIED ON
# to print BENCH_TREE_DONE. cargo lives in ~/.cargo/bin, which a NON-INTERACTIVE ssh does not
# have on PATH (~/.bashrc returns early before the cargo env line). Export it here rather than
# relying on the caller's shell being interactive.
export PATH="$HOME/.cargo/bin:$PATH"
command -v cargo >/dev/null || say "⊘ B4: cargo is not on PATH even after \$HOME/.cargo/bin -- the ladder cannot build"
( cd "${KAYFABE_REPO:-/root/kayfabe}" \
  && cargo build --release --target x86_64-unknown-linux-musl --bin kayfabe-rm-ladder 2>&1 | tail -2 )
GUEST_LADDER="${KAYFABE_REPO:-/root/kayfabe}/target/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder"
if [ -x "$GUEST_LADDER" ]; then
  say "B4: guest ladder built: $(stat -c %s "$GUEST_LADDER") bytes"
else
  say "⊘ B4: guest ladder MISSING — every later boot will report UNMEASURED_NO_BINARY"
fi

say "BENCH_TREE_DONE"
