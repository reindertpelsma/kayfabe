#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
#
# ★ win_vm.sh — a Windows 11 guest on kayfabe (kf3): installed ONCE, booted many times.
#
# Owner, 2026-10-04: "a script to later boot windows + kayfabe is actually useful to not repeat all
# manual steps or a docker container, also exposing a ssh to windows". One script now; a container
# can wrap it later. Design and evidence: docs/design/V3_WINDOWS_DISCOVERY.md. README.md beside this
# file is the user guide.
#
#   win_vm.sh install [--authorized-key FILE]...   one time: media (sha256-pinned), the VM's Secure
#                    Boot vars (Microsoft keys + this host's kayfabe db cert), its TPM (manufactured
#                    ONCE), a disk, an unattended install with the Red Hat virtio drivers, BitLocker
#                    prevented, OpenSSH keyed to the harness; then a check boot; then the disk is sealed
#   win_vm.sh run [--overlay NAME] [--once] [--arm A|B] [--no-kf3] [--guest-driver V]
#                 [--kf3-extra PROPS] [--tag TAG] [--ssh-port N] [--ssh-bind ADDR] [--detach]
#                 [--no-rpc-trace]
#                    boot the installed disk with kf3 (its signed GOP ROM, display on). kf3 has no
#                    reset path, so every guest reboot is a fresh QEMU: -action reboot=shutdown and a
#                    restart loop; the SAME OVMF vars and the SAME TPM state every time
#   win_vm.sh ssh [CMD...]        ssh to the guest (powershell is its default shell)
#   win_vm.sh scp-to SRC DST | scp-from SRC DST    (guest paths like C:/kf/x)
#   win_vm.sh check [--kf3]       run check.ps1 in the running guest (WIN_* lines)
#   win_vm.sh nv-install [--no-enable]   stage 2: the NVIDIA driver with GSP forced on (gsp_on.ps1)
#   win_vm.sh collect TAG         collect.ps1 in the guest, evidence pulled to logs/evidence/TAG
#   win_vm.sh shot [NAME]         screendump of the console, as PNG under logs/shots/
#   win_vm.sh stop                graceful: guest shutdown, then QMP powerdown, then QMP quit
#   win_vm.sh status              what runs, by process AND socket
#   win_vm.sh sign                export + sign this checkout's kf-gop for gop-efi=
#   win_vm.sh fetch               download + verify the pinned media into the cache
#
# Environment: WINVM_ROOT (/workspace/winvm), WINVM_NAME (kfwin), BENCH (/workspace/bench: kf3
# binaries per revision), WINVM_QEMU (override the binary), WINVM_SSH_PORT (2224),
# WINVM_SSH_BIND (127.0.0.1), WINVM_RAM_MB (8192), WINVM_SMP (8), WINVM_KF3_FB_MB (8192).
#
# ⊘ NO SECRETS IN THIS FILE OR THE REPO. Everything secret is generated on the host into the VM
# directory (0700/0600) and never leaves it: the Windows password, the harness ssh key, the swtpm
# state (owner: "the state file is the secret"), the OVMF vars. The kayfabe Secure Boot db key is
# generated once per host in $WINVM_ROOT/secureboot (0700).
#
# Layout (umask 077):
#   $WINVM_ROOT/cache/          pinned media (ISOs, MSI, NVIDIA package)
#   $WINVM_ROOT/secureboot/     db.key db.crt db.guid, kf-gop.<rev>.efi, kf-gop.<rev>.signed.efi
#   $WINVM_ROOT/<name>/
#     firmware/OVMF_VARS.fd     per-VM Secure Boot state; created once, never regenerated
#     tpm/                      swtpm state; manufactured once (swtpm_setup), never re-manufactured
#     disk/base.qcow2           the installed system (sealed 0400 after the check); overlays beside it
#     secrets/                  win_password, ssh_ed25519[.pub], unattend.iso (deleted after the check)
#     run/                      sockets (qmp, qmp-ev, qga, swtpm, vnc), pids, locks, known_hosts
#     logs/                     every boot's QEMU log, QMP events, screenshots, checks, evidence
#     installed                 written last by a passing install
# shellcheck disable=SC2054  # QEMU arguments carry commas by design
set -euo pipefail
umask 077

HERE=$(cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")" && pwd)
REPO=$(cd "$HERE/../../.." && pwd)
WINVM_ROOT=${WINVM_ROOT:-/workspace/winvm}
NAME=${WINVM_NAME:-kfwin}
VM=$WINVM_ROOT/$NAME
CACHE=$WINVM_ROOT/cache
SB=$WINVM_ROOT/secureboot
BENCH=${BENCH:-/workspace/bench}
SSH_PORT=${WINVM_SSH_PORT:-2224}
SSH_BIND=${WINVM_SSH_BIND:-127.0.0.1}
RAM_MB=${WINVM_RAM_MB:-8192}
SMP=${WINVM_SMP:-8}
DISK_GB=${WINVM_DISK_GB:-80}
KF3_FB_MB=${WINVM_KF3_FB_MB:-8192}
GPU_LOCK=${KF_GPU_LOCK:-/tmp/kayfabe-fastguest.lock}
OVMF_CODE=${WINVM_OVMF_CODE:-/usr/share/OVMF/OVMF_CODE_4M.secboot.fd}
OVMF_VARS_TEMPLATE=${WINVM_OVMF_VARS:-/usr/share/OVMF/OVMF_VARS_4M.fd}
WIN_USER=kf
MAC=52:54:00:4b:46:57
QMP="python3 $HERE/qmp.py"
QBIN=""
export PATH="$HOME/.cargo/bin:$PATH"

# ★ The media, each pinned by sha256 at its first fetch (2026-10-04, box vwin). Only official
# sources: Microsoft's evaluation fwlink, the virtio-win project's own versioned archive (the
# Fedora/Red Hat distribution point the virtio-win README names), Microsoft's Win32-OpenSSH
# release, NVIDIA's download server. To move a pin, change the URL and the hash together.
#   file | url | sha256 | bytes
MEDIA=(
  "win11_ltsc_2024_eval_x64_en-us.iso|https://go.microsoft.com/fwlink/?linkid=2289029&clcid=0x409&culture=en-us&country=us|67cec5865eaa037a72ddc633a717a10a2bed50778862267223ddb9c60ef5da68|5112850432"
  "virtio-win-0.1.302.iso|https://fedorapeople.org/groups/virt/virtio-win/direct-downloads/archive-virtio/virtio-win-0.1.302-1/virtio-win-0.1.302.iso|303f7ae40dad495d6ae474fdc571df58958a4dbc5c37a522d80f9a203867949d|877373440"
  "OpenSSH-Win64-v10.0.0.0.msi|https://github.com/PowerShell/Win32-OpenSSH/releases/download/10.0.0.0p2-Preview/OpenSSH-Win64-v10.0.0.0.msi|ddec9c53864280759cf9f74791cefd387100e3946aa849a1c138a4ed1b96b7d9|6586368"
  "580.88-desktop-win10-win11-64bit-international-dch-whql.exe|https://us.download.nvidia.com/Windows/580.88/580.88-desktop-win10-win11-64bit-international-dch-whql.exe|90c49f925b41ee062e02cc8094a16b6d057abfde4e85235a76d63ffe267f63a9|890807536"
)
WIN_ISO=$CACHE/win11_ltsc_2024_eval_x64_en-us.iso
VIRTIO_ISO=$CACHE/virtio-win-0.1.302.iso
OPENSSH_MSI=$CACHE/OpenSSH-Win64-v10.0.0.0.msi
NV_EXE=$CACHE/580.88-desktop-win10-win11-64bit-international-dch-whql.exe
NV_DIR=$CACHE/nv580.88

# ------------------------------------------------------------------------------------------------
# logging: every step writes START and EXIT; a log with a START and no EXIT is a dead step
# ------------------------------------------------------------------------------------------------
LOGFILE=""
ts() { date -u +%Y-%m-%dT%H:%M:%SZ; }
log() {
  local l
  l="[$(ts)] $*"
  echo "$l"
  if [ -n "$LOGFILE" ]; then echo "$l" >> "$LOGFILE"; fi
  return 0
}
die() { log "REFUSED: $*" >&2; exit 2; }
# ⊘ The body runs in a subshell OUTSIDE any `||`/`if`: bash ignores `set -e` for everything called in
# such a context, so `"$@" || rc=$?` reported rc=0 for a step whose python had died (measured on vwin,
# 2026-10-04: the unattend ISO shipped without autounattend.xml and Setup stopped at its first page).
# A step therefore cannot set variables in the caller; none needs to.
step() {
  local n=$1 rc
  shift
  log "START $n"
  set +e
  (
    set -e
    "$@"
  )
  rc=$?
  set -e
  log "EXIT $n rc=$rc"
  return "$rc"
}

# ------------------------------------------------------------------------------------------------
# prerequisites, refused by name
# ------------------------------------------------------------------------------------------------
declare -A PKG=(
  [qemu-img]=qemu-utils [swtpm]=swtpm [swtpm_setup]=swtpm-tools [sbsign]=sbsigntool
  [sbverify]=sbsigntool [virt-fw-vars]=python3-virt-firmware [xorriso]=xorriso [pnmtopng]=netpbm
  [7z]=p7zip-full [openssl]=openssl [ssh-keygen]=openssh-client [ssh]=openssh-client
  [scp]=openssh-client [python3]=python3 [curl]=curl [sha256sum]=coreutils [flock]=util-linux
  [cargo]="rustup (scripts/bench/provision_box.sh)" [git]=git
)
need() {
  local c miss=()
  for c in "$@"; do command -v "$c" >/dev/null 2>&1 || miss+=("$c (${PKG[$c]:-?})"); done
  [ ${#miss[@]} -eq 0 ] || die "missing: ${miss[*]}"
}
need_file() { [ -e "$1" ] || die "missing $1 — $2"; }
check_prereqs() {
  need qemu-img swtpm swtpm_setup sbsign sbverify virt-fw-vars xorriso pnmtopng 7z openssl \
       ssh-keygen ssh scp python3 curl sha256sum flock git
  need_file /dev/kvm "KVM is required"
  need_file "$OVMF_CODE" "apt install ovmf (the Secure Boot 4M build)"
  need_file "$OVMF_VARS_TEMPLATE" "apt install ovmf"
  [ -n "${QBIN:-}" ] || die "internal: QBIN unset"
  "$QBIN" -device help 2>/dev/null | grep -q '"tpm-crb"' \
    || die "$QBIN has no tpm-crb: rebuild with scripts/bench/build_kf3.sh (it configures --enable-tpm since 2026-10-04)"
  "$QBIN" -netdev help 2>/dev/null | grep -qx 'user' \
    || die "$QBIN has no user networking: rebuild with scripts/bench/build_kf3.sh (--enable-slirp since 2026-10-04)"
}

kf3_rev() {
  local r
  r=$(git -C "$REPO" rev-parse --short=8 HEAD)
  [ -z "$(git -C "$REPO" status --porcelain --untracked-files=no)" ] || r="$r-dirty"
  echo "$r"
}
qemu_bin() {
  local q=${WINVM_QEMU:-$BENCH/kf3-bins/$(kf3_rev)/qemu-system-x86_64}
  [ -x "$q" ] || die "no kf3 QEMU for revision $(kf3_rev) at $q — run: bash scripts/bench/build_kf3.sh $BENCH/qemu-10.2.4 (from $REPO)"
  echo "$q"
}
host_driver() { nvidia-smi --query-gpu=driver_version --format=csv,noheader 2>/dev/null | head -1 || echo none; }

# ------------------------------------------------------------------------------------------------
# media
# ------------------------------------------------------------------------------------------------
cmd_fetch() {
  install -d -m 0755 "$CACHE"
  local row f url sum bytes have
  for row in "${MEDIA[@]}"; do
    IFS='|' read -r f url sum bytes <<<"$row"
    if [ ! -s "$CACHE/$f" ]; then
      log "fetch $f from $url"
      curl -fL --retry 3 -o "$CACHE/$f.part" "$url"
      mv "$CACHE/$f.part" "$CACHE/$f"
    fi
    have=$(sha256sum "$CACHE/$f" | cut -d' ' -f1)
    [ "$have" = "$sum" ] || die "$f: sha256 $have, pinned $sum — refusing a medium that is not the pinned one"
    [ "$(stat -c %s "$CACHE/$f")" = "$bytes" ] || die "$f: $(stat -c %s "$CACHE/$f") bytes, pinned $bytes"
    log "MEDIUM_OK $f sha256=$sum bytes=$bytes"
  done
}

# ------------------------------------------------------------------------------------------------
# the VM directory, secrets, Secure Boot, TPM, disk
# ------------------------------------------------------------------------------------------------
make_layout() {
  install -d -m 0700 "$VM" "$VM/firmware" "$VM/tpm" "$VM/disk" "$VM/secrets" "$VM/run" "$VM/logs" "$VM/logs/shots"
  if [ ! -e "$VM/manifest.env" ]; then
    {
      echo "# win_vm.sh VM manifest (non-secret)"
      echo "NAME=$NAME"
      echo "CREATED=$(ts)"
      echo "MAC=$MAC"
      echo "WIN_USER=$WIN_USER"
      echo "CREATED_AT_REPO_REV=$(kf3_rev)"
      for row in "${MEDIA[@]}"; do IFS='|' read -r f _ sum _ <<<"$row"; echo "MEDIUM=$f:$sum"; done
    } > "$VM/manifest.env"
  fi
}

apparmor_swtpm() {
  # jammy's swtpm runs under an enforced AppArmor profile scoped to libvirt paths; allow this
  # harness's tree (state, sockets, logs) through the profile's local include, once.
  local prof=/etc/apparmor.d/usr.bin.swtpm local=/etc/apparmor.d/local/usr.bin.swtpm rule
  [ -e "$prof" ] || { log "no swtpm AppArmor profile"; return 0; }
  grep -q 'include.*local/usr.bin.swtpm' "$prof" || die "$prof does not include $local"
  rule="$WINVM_ROOT/** rwk,"
  if ! grep -qxF "$rule" "$local" 2>/dev/null; then
    printf '# kayfabe scripts/bench/windows/win_vm.sh: per-VM swtpm state, sockets, logs\n%s\n' "$rule" >> "$local"
    apparmor_parser -r "$prof"
    log "AppArmor: swtpm may use $WINVM_ROOT"
  fi
}

make_secrets() {
  if [ ! -s "$VM/secrets/win_password" ]; then
    openssl rand -base64 24 | tr -d '\n' > "$VM/secrets/win_password"
    chmod 0600 "$VM/secrets/win_password"
    log "generated the Windows password (secrets/win_password, 0600)"
  fi
  if [ ! -s "$VM/secrets/ssh_ed25519" ]; then
    ssh-keygen -q -t ed25519 -N '' -C "kayfabe-winvm-$NAME" -f "$VM/secrets/ssh_ed25519"
    log "generated the harness ssh key (secrets/ssh_ed25519)"
  fi
}

sb_key() {
  install -d -m 0700 "$SB"
  if [ ! -s "$SB/db.key" ]; then
    openssl req -new -x509 -newkey rsa:3072 -sha256 -nodes -days 7300 \
      -subj "/CN=kayfabe GOP db key ($(hostname) $(date -u +%F))/" \
      -keyout "$SB/db.key" -out "$SB/db.crt" 2>/dev/null
    python3 -c 'import uuid; print(uuid.uuid4())' > "$SB/db.guid"
    chmod 0600 "$SB/db.key" "$SB/db.crt" "$SB/db.guid"
    log "generated this host's kayfabe Secure Boot db key ($SB/db.key, never leaves the host)"
  fi
  log "SB_DB_CERT $(openssl x509 -in "$SB/db.crt" -noout -subject -fingerprint -sha256 | tr '\n' ' ')"
}

make_vars() {
  local vars=$VM/firmware/OVMF_VARS.fd
  if [ -s "$vars" ]; then
    log "VARS_EXIST $vars — reused, never regenerated (it is per-VM Secure Boot state)"
  else
    virt-fw-vars -i "$OVMF_VARS_TEMPLATE" -o "$vars.tmp" \
      --enroll-generate "kayfabe VM $NAME PK/KEK" \
      --add-db "$(cat "$SB/db.guid")" "$SB/db.crt" --secure-boot
    chmod 0600 "$vars.tmp"
    mv "$vars.tmp" "$vars"
    log "VARS_CREATED $vars"
  fi
  # The certificate subjects in PK, KEK and db (non-secret), checked on every install run.
  virt-fw-vars -i "$vars" --print --verbose 2>&1 \
    | grep -E '^name=|subject|siglist|SecureBootEnable' > "$VM/logs/ovmf_vars_print.txt" || true
  grep -q 'subject CN=kayfabe GOP db key' "$VM/logs/ovmf_vars_print.txt" \
    || die "the VM's db carries no kayfabe GOP db cert (logs/ovmf_vars_print.txt)"
  log "VARS_DB $(awk '/^name=db /{f=1;next} /^name=/{f=0} f && /subject/{sub(/^ *subject /,""); printf "%s; ", $0}' "$VM/logs/ovmf_vars_print.txt")"
}

tpm_manufactured() { [ -e "$VM/tpm/tpm2-00.permall" ]; }
tpm_manufacture() {
  # ★ ONCE per VM (owner, 2026-10-04: "the state file is the secret" and "tpm should persist
  # reboot"). The primary seeds come from swtpm's CSPRNG; nothing is derived from the VM's name or
  # any other non-secret value. An existing state is REFUSED, never overwritten.
  if tpm_manufactured; then
    log "REFUSED: the TPM of $NAME is already manufactured ($VM/tpm/tpm2-00.permall); it is never re-manufactured"
    return 2
  fi
  install -d -m 0700 "$VM/tpm"
  swtpm_setup --tpm2 --tpmstate "$VM/tpm" --create-ek-cert --create-platform-cert --lock-nvram \
    --pcr-banks sha256 --not-overwrite --logfile "$VM/logs/swtpm_setup.log"
  tpm_manufactured || { log "swtpm_setup wrote no tpm2-00.permall (logs/swtpm_setup.log)"; return 1; }
  chmod 0600 "$VM/tpm"/*
  log "TPM_MANUFACTURED $VM/tpm ($(stat -c %s "$VM/tpm/tpm2-00.permall") bytes of state)"
}

make_disk() {
  if [ -e "$VM/disk/base.qcow2" ]; then log "DISK_EXISTS base.qcow2"; return 0; fi
  qemu-img create -q -f qcow2 "$VM/disk/base.qcow2" "${DISK_GB}G"
  log "DISK_CREATED base.qcow2 ${DISK_GB}G"
}

make_unattend() {
  local d=$VM/secrets/unattend.d k=1 L drv key
  rm -rf "$d"
  install -d -m 0700 "$d"
  local winpe="" offline=""
  for L in D E F G H I J; do
    winpe+="    <PathAndCredentials wcm:action=\"add\" wcm:keyValue=\"$k\"><Path>$L:\\viostor\\w11\\amd64</Path></PathAndCredentials>"$'\n'
    k=$((k + 1))
  done
  k=1
  for drv in NetKVM vioserial viorng pvpanic fwcfg smbus; do
    for L in D E F G H I J; do
      offline+="    <PathAndCredentials wcm:action=\"add\" wcm:keyValue=\"$k\"><Path>$L:\\$drv\\w11\\amd64</Path></PathAndCredentials>"$'\n'
      k=$((k + 1))
    done
  done
  WINPE="$winpe" OFFLINE="$offline" PASSWORD_FILE="$VM/secrets/win_password" USER_NAME="$WIN_USER" \
  COMPUTER="KFWIN" python3 - "$HERE/autounattend.xml.in" "$d/autounattend.xml" <<'PY'
import os, sys
src, dst = sys.argv[1], sys.argv[2]
t = open(src, encoding="utf-8").read()
pw = open(os.environ["PASSWORD_FILE"], encoding="utf-8").read().strip()
for k, v in (("@@WINPE_DRIVER_PATHS@@", os.environ["WINPE"].rstrip("\n")),
             ("@@OFFLINE_DRIVER_PATHS@@", os.environ["OFFLINE"].rstrip("\n")),
             ("@@USER@@", os.environ["USER_NAME"]), ("@@COMPUTERNAME@@", os.environ["COMPUTER"]),
             ("@@PASSWORD@@", pw)):
    assert k in t, k
    t = t.replace(k, v)
assert "@@" not in t
fd = os.open(dst, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
os.write(fd, t.encode("utf-8"))
os.close(fd)
PY
  cp "$HERE/firstlogon.ps1" "$d/kf-firstlogon.ps1"
  cp "$OPENSSH_MSI" "$d/"
  cat "$VM/secrets/ssh_ed25519.pub" > "$d/administrators_authorized_keys"
  for key in "$@"; do
    grep -qE '^(ssh-ed25519|ssh-rsa|ecdsa-sha2-nistp(256|384|521)) ' "$key" || die "--authorized-key $key is not an OpenSSH public key"
    cat "$key" >> "$d/administrators_authorized_keys"
  done
  xorriso -as mkisofs -quiet -J -R -V KFUNATTEND -o "$VM/secrets/unattend.iso" "$d"
  chmod 0600 "$VM/secrets/unattend.iso"
  rm -rf "$d"
  log "UNATTEND_ISO secrets/unattend.iso ($(stat -c %s "$VM/secrets/unattend.iso") bytes; $(wc -l < <(cat "$VM/secrets/ssh_ed25519.pub" "$@")) authorized key(s))"
}

# ------------------------------------------------------------------------------------------------
# QEMU
# ------------------------------------------------------------------------------------------------
cpu_flags() {
  case "$1" in
    A) echo "host,-vmx" ;;
    B) echo "host,-vmx,hv-relaxed,hv-vapic,hv-spinlocks=0x1fff,hv-time,hv-vpindex,hv-synic,hv-stimer,hv-crash" ;;
    *) die "--arm must be A (no Hyper-V enlightenments) or B (with)" ;;
  esac
}
qemu_common() {
  QA=(
    -name "$NAME,debug-threads=on"
    -object "memory-backend-memfd,id=ram0,size=${RAM_MB}M,share=on"
    -machine "pc-q35-10.2,accel=kvm,smm=on,memory-backend=ram0" -m "$RAM_MB"
    -smp "$SMP,sockets=1,cores=$SMP,threads=1"
    -global driver=cfi.pflash01,property=secure,value=on
    -drive "if=pflash,format=raw,unit=0,readonly=on,file=$OVMF_CODE"
    -drive "if=pflash,format=raw,unit=1,file=$VM/firmware/OVMF_VARS.fd"
    -global ICH9-LPC.disable_s3=1 -global ICH9-LPC.disable_s4=1
    -rtc base=utc,driftfix=slew -global kvm-pit.lost_tick_policy=discard
    -chardev "socket,id=chrtpm,path=$VM/run/swtpm.sock" -tpmdev emulator,id=tpm0,chardev=chrtpm
    -device tpm-crb,tpmdev=tpm0
    -netdev "user,id=n0,restrict=on,hostfwd=tcp:$SSH_BIND:$SSH_PORT-:22"
    -device "virtio-net-pci,netdev=n0,mac=$MAC"
    -device virtio-serial-pci,id=vser0
    -chardev "socket,id=qga0,path=$VM/run/qga.sock,server=on,wait=off"
    -device virtserialport,bus=vser0.0,chardev=qga0,name=org.qemu.guest_agent.0
    -object rng-random,id=rng0,filename=/dev/urandom -device virtio-rng-pci,rng=rng0
    -device pvpanic
    -display none -vnc "unix:$VM/run/vnc.sock"
    -qmp "unix:$VM/run/qmp.sock,server=on,wait=off" -qmp "unix:$VM/run/qmp-ev.sock,server=on,wait=off"
    -msg timestamp=on
  )
}

swtpm_start() {
  local pid
  pid=$(cat "$VM/run/swtpm.pid" 2>/dev/null || true)
  if [ -n "$pid" ] && [ -d "/proc/$pid" ] && grep -q swtpm "/proc/$pid/comm" 2>/dev/null; then
    die "a swtpm (pid $pid) still serves $VM/tpm; never two on one state"
  fi
  tpm_manufactured || die "the TPM of $NAME was never manufactured (run install)"
  rm -f "$VM/run/swtpm.sock" "$VM/run/swtpm.pid"
  # ⚠ --terminate: swtpm exits when QEMU disconnects, so every QEMU start gets a fresh process on
  # the SAME state. Never --flags startup-clear (the firmware sends TPM2_Startup).
  swtpm socket --tpm2 --tpmstate "dir=$VM/tpm,mode=0600" \
    --ctrl "type=unixio,path=$VM/run/swtpm.sock,mode=0600" \
    --log "file=$VM/logs/swtpm.log,level=20" --pid "file=$VM/run/swtpm.pid" --terminate --daemon
  for _ in $(seq 1 50); do [ -S "$VM/run/swtpm.sock" ] && return 0; sleep 0.2; done
  die "swtpm never opened $VM/run/swtpm.sock (logs/swtpm.log)"
}

qemu_running() {
  local pid
  pid=$(cat "$VM/run/qemu.pid" 2>/dev/null || true)
  [ -n "$pid" ] && [ -d "/proc/$pid" ] && [ "$(cat "/proc/$pid/comm" 2>/dev/null)" = qemu-system-x86 ] \
    && [ "$(awk '{print $3}' "/proc/$pid/stat" 2>/dev/null)" != Z ]
}

# vm_start TAG BOOT EXTRA-QEMU-ARGS...   (QA must be set; sets QPID, EVPID, SHOTPID, BOOT_EV)
vm_start() {
  local tag=$1 n=$2 q=$QBIN
  shift 2
  qemu_running && die "QEMU for $NAME is already running (pid $(cat "$VM/run/qemu.pid"))"
  swtpm_start
  rm -f "$VM/run/qmp.sock" "$VM/run/qmp-ev.sock" "$VM/run/qga.sock" "$VM/run/vnc.sock"
  BOOT_EV=$VM/logs/${tag}_b${n}_qmp_events.jsonl
  local qlog=$VM/logs/${tag}_b${n}_qemu.log
  printf '%q ' "$q" "${QA[@]}" "$@" > "$VM/logs/${tag}_b${n}_cmdline.txt"
  "$q" "${QA[@]}" "$@" > "$qlog" 2>&1 &
  QPID=$!
  echo "$QPID" > "$VM/run/qemu.pid"
  for _ in $(seq 1 100); do [ -S "$VM/run/qmp-ev.sock" ] && break; kill -0 "$QPID" 2>/dev/null || break; sleep 0.2; done
  $QMP "$VM/run/qmp-ev.sock" events "$BOOT_EV" >/dev/null 2>&1 &
  EVPID=$!
  log "QEMU_START tag=$tag boot=$n pid=$QPID log=logs/${tag}_b${n}_qemu.log"
  # one screendump every SHOT_EVERY seconds while this QEMU lives
  # (named by this boot's start time too, so a re-run of a tag never overwrites an earlier boot's)
  local started
  started=$(date -u +%Y%m%dT%H%M%SZ)
  ( local s=0
    while kill -0 "$QPID" 2>/dev/null; do
      sleep "${SHOT_EVERY:-30}"; s=$((s + 1))
      shot_to "$VM/logs/shots/${tag}_b${n}_${started}_$(printf %04d "$s").png" >/dev/null 2>&1 || true
    done ) &
  SHOTPID=$!
}

# vm_wait [TIMEOUT_S] → sets BOOT_RC and BOOT_REASON (the last QMP SHUTDOWN reason of this boot)
vm_wait() {
  local timeout=${1:-0} waited=0 rc=0
  BOOT_RC=0
  if [ "$timeout" -gt 0 ]; then
    while kill -0 "$QPID" 2>/dev/null && [ "$(awk '{print $3}' "/proc/$QPID/stat" 2>/dev/null)" != Z ]; do
      sleep 5
      waited=$((waited + 5))
      if [ "$waited" -ge "$timeout" ]; then
        log "TIMEOUT after ${timeout}s: screendump, then QMP quit"
        shot_to "$VM/logs/shots/timeout_$(date -u +%H%M%S).png" || true
        $QMP "$VM/run/qmp.sock" cmd quit >/dev/null 2>&1 || true
        break
      fi
    done
  fi
  while :; do
    rc=0
    wait "$QPID" || rc=$?
    if [ "$rc" -gt 128 ] && qemu_running; then continue; fi
    break
  done
  BOOT_RC=$rc
  wait "$EVPID" 2>/dev/null || true
  kill "$SHOTPID" 2>/dev/null || true
  wait "$SHOTPID" 2>/dev/null || true
  BOOT_REASON=$($QMP - last-shutdown "$BOOT_EV")
  [ "$waited" -ge "$timeout" ] && [ "$timeout" -gt 0 ] && BOOT_REASON="timeout"
  rm -f "$VM/run/qemu.pid"
  return 0
}

shot_to() {  # FILE.png — the default graphic console (std VGA in install, kf3's head in run)
  local png=$1 ppm=${1%.png}.ppm
  $QMP "$VM/run/qmp.sock" cmd screendump "{\"filename\":\"$ppm\"}" >/dev/null 2>&1 || return 1
  for _ in 1 2 3 4 5 6 7 8 9 10; do [ -s "$ppm" ] && break; sleep 0.2; done
  pnmtopng "$ppm" > "$png" 2>/dev/null && rm -f "$ppm"
}

# ------------------------------------------------------------------------------------------------
# ssh to the guest (key only; powershell is the guest's default shell)
# ------------------------------------------------------------------------------------------------
ssh_opts() {
  SSHO=(-i "$VM/secrets/ssh_ed25519" -o StrictHostKeyChecking=accept-new
        -o "UserKnownHostsFile=$VM/run/known_hosts" -o ConnectTimeout=8 -o ServerAliveInterval=15
        -o ServerAliveCountMax=4 -o BatchMode=yes -o LogLevel=ERROR)
}
gssh() { ssh_opts; timeout "${SSH_TIMEOUT:-900}" ssh -n -p "$SSH_PORT" "${SSHO[@]}" "$WIN_USER@127.0.0.1" "$@"; }
gscp_to() { ssh_opts; timeout "${SSH_TIMEOUT:-1800}" scp -q -r -P "$SSH_PORT" "${SSHO[@]}" "$1" "$WIN_USER@127.0.0.1:$2"; }
gscp_from() { ssh_opts; timeout "${SSH_TIMEOUT:-1800}" scp -q -r -P "$SSH_PORT" "${SSHO[@]}" "$WIN_USER@127.0.0.1:$1" "$2"; }
wait_ssh() {  # TIMEOUT_S
  local t=$1 w=0
  while [ "$w" -lt "$t" ]; do
    qemu_running || { log "QEMU exited while waiting for ssh"; return 1; }
    if SSH_TIMEOUT=20 gssh 'echo KF_SSH_UP' 2>/dev/null | grep -q KF_SSH_UP; then
      log "SSH_UP after ${w}s"
      return 0
    fi
    sleep 10
    w=$((w + 10))
  done
  log "ssh never answered in ${t}s"
  return 1
}
qga_diag() {
  $QMP "$VM/run/qga.sock" qga-ping && $QMP "$VM/run/qga.sock" qga-exec \
    'C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe' -NoProfile -Command \
    'Get-Content C:\kf\install-done.txt,C:\kf\firstlogon.log -Tail 60 -ErrorAction SilentlyContinue; Get-Service sshd,QEMU-GA -ErrorAction SilentlyContinue | Format-Table -AutoSize' \
    2>&1 | tail -80 || log "the guest agent does not answer either"
}

run_check() {  # TAG [-ExpectKf3]
  local tag=$1 out
  shift
  gssh 'New-Item -ItemType Directory -Force -Path C:\kf | Out-Null' >/dev/null || return 1
  gscp_to "$HERE/check.ps1" "C:/kf/check.ps1" || return 1
  out=$VM/logs/${tag}_check.txt
  local rc=0
  gssh "powershell -NoProfile -ExecutionPolicy Bypass -File C:\\kf\\check.ps1 $*" > "$out" 2>&1 || rc=$?
  tr -d '\r' < "$out" | grep -E '^WIN_' | sed 's/^/  /'
  grep -h '^WIN_TPM_EK' "$out" | tr -d '\r' | sed "s|^|$(ts) $tag |" >> "$VM/logs/tpm_ek_history.txt" || true
  tr -d '\r' < "$out" | grep -q '^WIN_CHECK_DONE fail=0' || rc=1
  return "$rc"
}

guest_shutdown_and_wait() {  # TIMEOUT_S
  touch "$VM/run/stop-requested"
  gssh 'shutdown /s /t 0' >/dev/null 2>&1 || true
  local w=0
  while qemu_running && [ "$w" -lt "${1:-180}" ]; do sleep 5; w=$((w + 5)); done
  if qemu_running; then
    log "guest did not power off in ${1:-180}s: QMP system_powerdown"
    $QMP "$VM/run/qmp.sock" cmd system_powerdown >/dev/null 2>&1 || true
    w=0
    while qemu_running && [ "$w" -lt 120 ]; do sleep 5; w=$((w + 5)); done
  fi
  if qemu_running; then
    log "still running: QMP quit"
    $QMP "$VM/run/qmp.sock" cmd quit >/dev/null 2>&1 || true
  fi
}

# ------------------------------------------------------------------------------------------------
# install
# ------------------------------------------------------------------------------------------------
install_boot() {
  if [ -e "$VM/logs/install_boot.done" ]; then log "install boot already completed ($(cat "$VM/logs/install_boot.done"))"; return 0; fi
  qemu_common
  QA+=(-cpu "$(cpu_flags B)" -vga std -action panic=none
       -drive "file=$VM/disk/base.qcow2,if=none,id=sys,format=qcow2,discard=unmap"
       -device virtio-blk-pci,drive=sys,bootindex=1
       -drive "file=$WIN_ISO,if=none,id=cd0,media=cdrom,readonly=on" -device ide-cd,drive=cd0,bus=ide.0,bootindex=0
       -drive "file=$VIRTIO_ISO,if=none,id=cd1,media=cdrom,readonly=on" -device ide-cd,drive=cd1,bus=ide.1
       -drive "file=$VM/secrets/unattend.iso,if=none,id=cd2,media=cdrom,readonly=on" -device ide-cd,drive=cd2,bus=ide.2
       -serial "file:$VM/logs/install_b1_serial.log")
  SHOT_EVERY=60 vm_start install 1
  # "Press any key to boot from CD or DVD": answered for the first boot only. Later in-QEMU reboots
  # let the prompt time out, so the installed disk boots.
  $QMP "$VM/run/qmp.sock" keys ret 30 >/dev/null 2>&1 || log "send-key failed (see the screenshots)"
  vm_wait 14400
  log "INSTALL_BOOT_END rc=$BOOT_RC reason=$BOOT_REASON"
  [ "$BOOT_REASON" = guest-shutdown ] || { log "the install did not end in a guest shutdown"; return 1; }
  echo "$(ts) reason=$BOOT_REASON" > "$VM/logs/install_boot.done"
}

check_boot() {
  qemu_common
  QA+=(-cpu "$(cpu_flags B)" -vga std -action panic=none
       -drive "file=$VM/disk/base.qcow2,if=none,id=sys,format=qcow2,discard=unmap"
       -device virtio-blk-pci,drive=sys,bootindex=0
       -serial "file:$VM/logs/installcheck_b1_serial.log")
  rm -f "$VM/run/stop-requested"
  SHOT_EVERY=60 vm_start installcheck 1
  local rc=0
  if wait_ssh 1800; then
    log "first logon said: $(gssh 'Get-Content C:\kf\install-done.txt' 2>/dev/null | tr -d '\r')"
    run_check installcheck || rc=$?
    gscp_from "C:/kf/firstlogon.log" "$VM/logs/firstlogon.log" || true
  else
    rc=1
    qga_diag
  fi
  guest_shutdown_and_wait 180
  vm_wait 600
  log "CHECK_BOOT_END rc=$BOOT_RC reason=$BOOT_REASON check_rc=$rc"
  return "$rc"
}

seal_base() {
  rm -f "$VM/secrets/unattend.iso"
  chmod 0400 "$VM/disk/base.qcow2"
  log "SEALED disk/base.qcow2 (0400); unattend.iso (it held the password) deleted; every run uses an overlay"
}

cmd_install() {
  local keys=()
  while [ $# -gt 0 ]; do
    case "$1" in
      --authorized-key) [ -r "${2:-}" ] || die "--authorized-key needs a readable file"; keys+=("$2"); shift 2 ;;
      *) die "install: unknown option $1" ;;
    esac
  done
  install -d -m 0700 "$WINVM_ROOT"
  if [ -e "$VM/installed" ]; then
    log "INSTALL_ALREADY_DONE $NAME ($(cat "$VM/installed")) — nothing to do; boot it with 'run'"
    return 0
  fi
  make_layout
  LOGFILE=$VM/logs/install.log
  exec 7>"$VM/run/vm.lock"
  flock -n 7 || die "$NAME is in use by another win_vm.sh"
  # ★ The binary and the revision are resolved ONCE: a checkout updated under a running install or
  # run must not change which QEMU the next boot of it starts.
  QBIN=$(qemu_bin)
  log "WINVM_INSTALL_START name=$NAME repo=$(kf3_rev) qemu=$QBIN host_driver=$(host_driver)"
  step prereqs check_prereqs
  step fetch cmd_fetch
  step apparmor apparmor_swtpm
  step secrets make_secrets
  step secureboot-key sb_key
  step ovmf-vars make_vars
  if tpm_manufactured; then
    log "TPM_EXISTS $VM/tpm — reused; the harness never re-manufactures a TPM"
  else
    step tpm-manufacture tpm_manufacture
  fi
  step disk make_disk
  if [ ! -e "$VM/logs/install_boot.done" ]; then step unattend make_unattend "${keys[@]}"; fi
  step install-boot install_boot
  step check-boot check_boot
  step seal seal_base
  echo "$(ts) repo=$(kf3_rev)" > "$VM/installed"
  log "WINVM_INSTALL_DONE name=$NAME"
}

# ------------------------------------------------------------------------------------------------
# sign: this checkout's kf-gop, signed with this host's db key, for kf3's gop-efi=
# ------------------------------------------------------------------------------------------------
gop_signed() {
  local rev out tgt
  rev=$(kf3_rev)
  out=$SB/kf-gop.$rev.signed.efi
  if [ ! -s "$out" ]; then
    [ -s "$SB/db.key" ] || die "no Secure Boot key at $SB (run install first)"
    ( cd "$REPO" && cargo build --release -q -p kf-gop-image --bin kf-gop-export ) >&2
    tgt=${CARGO_TARGET_DIR:-$REPO/target}
    rm -f "$SB/kf-gop.$rev.efi"
    "$tgt/release/kf-gop-export" "$SB/kf-gop.$rev.efi" >&2
    sbsign --key "$SB/db.key" --cert "$SB/db.crt" --output "$out.tmp" "$SB/kf-gop.$rev.efi" >&2
    sbverify --cert "$SB/db.crt" "$out.tmp" >&2
    chmod 0600 "$out.tmp"
    mv "$out.tmp" "$out"
  fi
  echo "$out"
}
cmd_sign() {
  local out
  out=$(gop_signed)
  log "KF_GOP_SIGNED $out sha256=$(sha256sum "$out" | cut -d' ' -f1) unsigned_sha256=$(sha256sum "${out%.signed.efi}.efi" | cut -d' ' -f1)"
}

# ------------------------------------------------------------------------------------------------
# run: the kf3 boot loop
# ------------------------------------------------------------------------------------------------
cmd_run() {
  local argv=("$@") overlay=main once=0 arm=A kf3=1 extra="" gdrv="" tag="" detach=0 max_boots=12
  while [ $# -gt 0 ]; do
    case "$1" in
      --overlay) overlay=$2; shift 2 ;;
      --once) once=1; shift ;;
      --arm) arm=$2; shift 2 ;;
      --no-kf3) kf3=0; shift ;;
      --kf3-extra) extra=$2; shift 2 ;;
      --guest-driver) gdrv=$2; shift 2 ;;
      --tag) tag=$2; shift 2 ;;
      --ssh-port) SSH_PORT=$2; shift 2 ;;
      --ssh-bind) SSH_BIND=$2; shift 2 ;;
      --max-boots) max_boots=$2; shift 2 ;;
      --detach) detach=1; shift ;;
      --no-rpc-trace) export KF3_RPC_TRACE=0; shift ;;
      *) die "run: unknown option $1" ;;
    esac
  done
  [ -e "$VM/installed" ] || die "$NAME is not installed (win_vm.sh install)"
  [[ "$overlay" =~ ^[A-Za-z0-9_.-]+$ ]] || die "--overlay must be a plain name"
  cpu_flags "$arm" >/dev/null
  [ -n "$tag" ] || tag="run_$(date -u +%Y%m%dT%H%M%SZ)"
  if [ "$detach" = 1 ]; then
    local a fwd=()
    for a in "${argv[@]}"; do [ "$a" = --detach ] || fwd+=("$a"); done
    nohup setsid "$HERE/win_vm.sh" run "${fwd[@]}" --tag "$tag" > "$VM/logs/${tag}_run.out" 2>&1 < /dev/null &
    log "detached: tag=$tag, log logs/${tag}_run.log (stdout logs/${tag}_run.out)"
    return 0
  fi
  LOGFILE=$VM/logs/${tag}_run.log
  exec 7>"$VM/run/vm.lock"
  flock -n 7 || die "$NAME is in use by another win_vm.sh"
  QBIN=$(qemu_bin)
  check_prereqs
  local disk=$VM/disk/$overlay.qcow2
  if [ ! -e "$disk" ]; then
    qemu-img create -q -f qcow2 -F qcow2 -b base.qcow2 "$disk"
    log "OVERLAY_CREATED disk/$overlay.qcow2 over base.qcow2"
  fi
  qemu_common
  QA+=(-cpu "$(cpu_flags "$arm")" -action reboot=shutdown,panic=none
       -drive "file=$disk,if=none,id=sys,format=qcow2,discard=unmap" -device virtio-blk-pci,drive=sys,bootindex=0)
  local gop="" dev=""
  if [ "$kf3" = 1 ]; then
    gop=$(gop_signed)
    dev="kf3-gpu,fb-mb=$KF3_FB_MB,bar1-size=134217728,bar2-size=33554432,display=on,gop=on,gop-efi=$gop,id=kf0"
    [ -z "$gdrv" ] || dev+=",guest-driver=$gdrv"
    [ -z "$extra" ] || dev+=",$extra"
    QA+=(-vga none -device "$dev")
    # every command kf3's chain answers, logged with its id and result (kf-rm census, log only)
    export KF3_RPC_TRACE=${KF3_RPC_TRACE:-1}
    exec 8>"$GPU_LOCK"
    log "waiting for the GPU lock $GPU_LOCK (GPU work is strictly serial)"
    flock -w 7200 8 || die "the GPU lock $GPU_LOCK was not free in 2 h"
  else
    QA+=(-vga std)
  fi
  rm -f "$VM/run/stop-requested"
  local stopping=0
  trap 'stopping=1; touch "$VM/run/stop-requested"; log "signal: asking the guest to power down"; $QMP "$VM/run/qmp.sock" cmd system_powerdown >/dev/null 2>&1 || true' TERM INT
  log "WINVM_RUN_START tag=$tag repo_rev=$(kf3_rev) kf3=$([ "$kf3" = 1 ] && echo on || echo off) qemu=$QBIN host_driver=$(host_driver) overlay=$overlay arm=$arm guest_driver=${gdrv:-default} gop_efi=${gop:-none} sha256(gop_efi)=$([ -n "$gop" ] && sha256sum "$gop" | cut -c1-16 || echo -)"
  [ -z "$dev" ] || log "kf3 device: $dev"
  local n=0 reason="" dm0
  while :; do
    n=$((n + 1))
    dm0=$(dmesg 2>/dev/null | wc -l || echo 0)
    nvidia-smi -q > "$VM/logs/${tag}_b${n}_host_smi_before.txt" 2>&1 || true
    vm_start "$tag" "$n" -serial "file:$VM/logs/${tag}_b${n}_serial.log"
    vm_wait 0
    reason=$BOOT_REASON
    dmesg 2>/dev/null | tail -n "+$((dm0 + 1))" > "$VM/logs/${tag}_b${n}_host_dmesg.txt" || true
    nvidia-smi -q > "$VM/logs/${tag}_b${n}_host_smi_after.txt" 2>&1 || true
    log "WINVM_BOOT_END boot=$n rc=$BOOT_RC reason=$reason kf3_lines=$(grep -c 'kf3\|kf-rm\|kf-gsp' "$VM/logs/${tag}_b${n}_qemu.log" || true) host_xid=$(grep -c 'NVRM: Xid' "$VM/logs/${tag}_b${n}_host_dmesg.txt" || true)"
    grep -q GUEST_PANICKED "$BOOT_EV" && log "  GUEST_PANICKED in boot $n: $(grep GUEST_PANICKED "$BOOT_EV" | tail -1)"
    [ "$reason" = guest-reset ] || break
    [ "$once" = 1 ] && { log "--once: not restarting after the guest's reboot"; break; }
    [ -e "$VM/run/stop-requested" ] || [ "$stopping" = 1 ] && break
    [ "$n" -ge "$max_boots" ] && { log "max boots ($max_boots) reached"; break; }
    log "guest rebooted: kf3 has no reset path, so a FRESH QEMU (same vars, same TPM state)"
  done
  trap - TERM INT
  log "WINVM_RUN_END tag=$tag boots=$n reason=$reason"
}

# ------------------------------------------------------------------------------------------------
# conveniences
# ------------------------------------------------------------------------------------------------
cmd_ssh() { ssh_opts; exec ssh -p "$SSH_PORT" "${SSHO[@]}" "$WIN_USER@127.0.0.1" "$@"; }
cmd_check() {
  local a=()
  [ "${1:-}" = --kf3 ] && a=(-ExpectKf3)
  LOGFILE=$VM/logs/checks.log
  run_check "check_$(date -u +%Y%m%dT%H%M%SZ)" "${a[@]}"
}
nv_extract() {
  [ -s "$NV_DIR/Display.Driver/nv_dispi.inf" ] && return 0
  install -d -m 0755 "$NV_DIR"
  7z x -y -bd -o"$NV_DIR" "$NV_EXE" 'Display.Driver/*' >/dev/null
  [ -s "$NV_DIR/Display.Driver/nv_dispi.inf" ] || die "no Display.Driver/nv_dispi.inf in $NV_EXE"
  log "NV_EXTRACTED $NV_DIR/Display.Driver ($(du -sh "$NV_DIR/Display.Driver" | cut -f1))"
}
cmd_nv_install() {
  local noen=""
  [ "${1:-}" = --no-enable ] && noen="-NoEnable"
  LOGFILE=$VM/logs/nv_install.log
  step nv-extract nv_extract
  if ! gssh 'Test-Path C:\kf\nv\Display.Driver\nv_dispi.inf' 2>/dev/null | grep -q True; then
    gssh 'New-Item -ItemType Directory -Force -Path C:\kf\nv | Out-Null' >/dev/null
    step nv-copy gscp_to "$NV_DIR/Display.Driver" "C:/kf/nv/"
  fi
  gscp_to "$HERE/gsp_on.ps1" "C:/kf/gsp_on.ps1"
  local out
  out=$VM/logs/nv_install_$(date -u +%Y%m%dT%H%M%SZ).txt
  step gsp-on gssh "powershell -NoProfile -ExecutionPolicy Bypass -File C:\\kf\\gsp_on.ps1 -Inf C:\\kf\\nv\\Display.Driver\\nv_dispi.inf $noen" > "$out" 2>&1 || true
  tr -d '\r' < "$out" | grep -E '^GSP_ON|START|EXIT' | sed 's/^/  /'
}
cmd_collect() {
  local tag=${1:?collect TAG}
  [[ "$tag" =~ ^[A-Za-z0-9_.-]+$ ]] || die "TAG must be a plain name"
  LOGFILE=$VM/logs/collect.log
  gssh 'New-Item -ItemType Directory -Force -Path C:\kf | Out-Null' >/dev/null
  gscp_to "$HERE/collect.ps1" "C:/kf/collect.ps1"
  step collect gssh "powershell -NoProfile -ExecutionPolicy Bypass -File C:\\kf\\collect.ps1 -Tag $tag" | tr -d '\r' | tail -5
  install -d -m 0700 "$VM/logs/evidence"
  step pull gscp_from "C:/kf/evidence/$tag" "$VM/logs/evidence/"
  log "EVIDENCE $VM/logs/evidence/$tag ($(ls "$VM/logs/evidence/$tag" | wc -l) files)"
}
cmd_shot() {
  local name=${1:-manual_$(date -u +%Y%m%dT%H%M%SZ)}
  [[ "$name" =~ ^[A-Za-z0-9_.-]+$ ]] || die "NAME must be a plain name"
  qemu_running || die "no QEMU running for $NAME"
  shot_to "$VM/logs/shots/$name.png" && echo "$VM/logs/shots/$name.png"
}
cmd_stop() {
  LOGFILE=$VM/logs/stop.log
  qemu_running || { log "not running"; return 0; }
  log "stopping $NAME (pid $(cat "$VM/run/qemu.pid"))"
  guest_shutdown_and_wait 180
  local w=0
  while qemu_running && [ "$w" -lt 30 ]; do sleep 2; w=$((w + 2)); done
  if qemu_running; then
    log "QEMU did not stop; NOT killing it (kf3's exit census would be lost) — stop it by hand"
    return 1
  fi
  log "stopped"
}
cmd_status() {
  echo "VM $VM installed=$([ -e "$VM/installed" ] && cat "$VM/installed" || echo no)"
  echo "qemu (this VM): $(qemu_running && echo "running pid $(cat "$VM/run/qemu.pid")" || echo none)"
  echo "qemu (all, pgrep -x qemu-system-x86): $(pgrep -x qemu-system-x86 | tr '\n' ' ' || true)"
  echo "swtpm: $(pgrep -x swtpm | tr '\n' ' ' || true)"
  local s
  for s in qmp qmp-ev qga swtpm vnc; do echo "socket $s: $([ -S "$VM/run/$s.sock" ] && echo present || echo absent)"; done
  echo "ssh $SSH_BIND:$SSH_PORT: $(ss -tln 2>/dev/null | grep -c ":$SSH_PORT " || true) listener(s)"
  echo "last run log: $(ls -t "$VM"/logs/*_run.log 2>/dev/null | head -1)"
  ls -t "$VM"/logs/*_run.log 2>/dev/null | head -1 | xargs -r tail -3
}

main() {
  local sub=${1:-}
  shift || true
  case "$sub" in
    install) cmd_install "$@" ;;
    run) cmd_run "$@" ;;
    ssh) cmd_ssh "$@" ;;
    scp-to) gscp_to "${1:?SRC}" "${2:?DST}" ;;
    scp-from) gscp_from "${1:?SRC}" "${2:?DST}" ;;
    check) cmd_check "$@" ;;
    nv-install) cmd_nv_install "$@" ;;
    collect) cmd_collect "$@" ;;
    shot) cmd_shot "$@" ;;
    stop) cmd_stop ;;
    status) cmd_status ;;
    sign) cmd_sign ;;
    fetch) cmd_fetch ;;
    ""|-h|--help|help) sed -n '3,46p' "$0" | sed 's/^# \{0,1\}//' ;;
    *) die "unknown subcommand '$sub' (win_vm.sh help)" ;;
  esac
}
main "$@"
