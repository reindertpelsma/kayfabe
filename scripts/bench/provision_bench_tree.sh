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
  qemu-system-x86_64 -enable-kvm -m 8G -smp 8 -display none \
    -drive if=virtio,file="$BENCH/guest.qcow2",format=qcow2 \
    -drive if=virtio,file="$BENCH/seed.iso",format=raw \
    -netdev user,id=n0,hostfwd=tcp::2222-:22 -device virtio-net-pci,netdev=n0 \
    -serial file:"$BENCH/provision_serial.log" -daemonize -pidfile "$BENCH/prov.pid"
  say "B2: qemu pid=$(cat $BENCH/prov.pid 2>/dev/null)"

  GS="ssh -i $BENCH/guest_key -p 2222 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
      -o LogLevel=ERROR -o ConnectTimeout=8 ubuntu@127.0.0.1"
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

  say "B2: installing guest driver (kernel-open)"
  $GS "sudo apt-get update -qq && sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq build-essential linux-headers-\$(uname -r)" 2>&1 | tail -3
  scp -i "$BENCH/guest_key" -P 2222 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
      -o LogLevel=ERROR "$RUN" ubuntu@127.0.0.1:/tmp/nv.run >/dev/null 2>&1
  $GS "sudo sh /tmp/nv.run --silent --no-x-check --no-nouveau-check --no-questions -m=kernel-open -j8" 2>&1 | tail -5
  # ⚠ verify on CONTENT, not the installer's exit code
  MI=$($GS 'modinfo nvidia 2>/dev/null | grep -E "^version|^license|^vermagic" | tr "\n" " "' 2>&1)
  say "B2: guest modinfo = $MI"
  # ⊘ Do NOT power off over a failed install: a clean shutdown makes the failure look like a
  #    completed phase, and the next step boots a guest with no driver in it.
  case "$MI" in
    *580.159.04*) say "B2: guest driver VERIFIED on content" ;;
    *) say "⊘ B2: guest driver NOT verified -- leaving the guest UP for inspection"; return 5 ;;
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
say "BENCH_TREE_DONE"
