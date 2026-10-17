#!/usr/bin/env bash
set -euo pipefail
cd /root/kayfabe
exec >/root/prov/ada590_broker_x11.log 2>&1
trap 'rc=$?; echo "EXIT rc=$rc $(date -u -Is)"' EXIT
test "$(git rev-parse --short=9 HEAD)" = 9d6f210ba
dmesg | grep 'NVRM: Xid' >/root/prov/ada590_broker_xids.before.txt || true
echo "START source=$(git rev-parse HEAD) host=$(nvidia-smi -i 0 --query-gpu=driver_version --format=csv,noheader) guest590.48.01 $(date -u -Is)"
xid_before=$(dmesg | grep -c "NVRM: Xid" || true)
[ -z "$(pgrep -x qemu-system-x86 || true)" ]
mkdir -p /root/codex-broker-ada590/bin /root/codex-nvpv-ada590
if [ ! -d /root/codex-nvpv-ada590/.git ]; then
  git -C /root/codex-nvpv-ada590 init -q
  git -C /root/codex-nvpv-ada590 fetch -q --depth 1 https://github.com/reindertpelsma/nvkvm-pv.git badf2d707da47a625e0ce69172a84785d20f81b8
fi
cat >/root/codex-broker-ada590/bin/loginctl <<'STUB'
#!/usr/bin/env bash
case "$*" in
  'show-seat seat0 -p ActiveSession --value') echo codex-test ;;
  'show-session codex-test -p Name --value') echo root ;;
  'show-session codex-test -p Type --value') echo x11 ;;
  'show-session codex-test -p LockedHint --value') echo no ;;
  *) exec /usr/bin/loginctl "$@" ;;
esac
STUB
chmod 755 /root/codex-broker-ada590/bin/loginctl
# Only supplies session metadata to the launcher's existing display-less test backend.
# No host seat/session is changed; the actual guest display and broker still run.
export PATH=/root/codex-broker-ada590/bin:/root/.cargo/bin:$PATH
export KF_INTERACTIVE_DIR=/root/codex-broker-ada590 KF_IMG=/root/codex-broker-ada590/desktop-reap.qcow2
export KF3_REV=$(git rev-parse --short=8 HEAD) KF_RAM_MB=8192 KF_FB_MB=4096 KF_SMP=8
export KF_GUEST_DRIVER=590.48.01 KF_BASE_GUEST_IMG=/workspace/bench/guest-590.48.01.qcow2
export KF_BROKER_BACKEND=x11 BROKER_ARGS=--present-mode=shm NVPV_DIR=/root/codex-nvpv-ada590
[ -p /root/codex-broker-ada590/input.fifo ] || mkfifo /root/codex-broker-ada590/input.fifo
export KF_BROKER_FIFO=/root/codex-broker-ada590/input.fifo
exec 7<>"$KF_BROKER_FIFO"
DEBIAN_FRONTEND=noninteractive apt-get install -y -qq libxcb1-dev libxcb-dri3-dev libxcb-present-dev libxcb-render0-dev libxcb-xinput-dev x11-apps xvfb x11-utils libwayland-dev wayland-protocols libgbm-dev >/root/prov/codex_x11_deps.log 2>&1
Xvfb :99 -screen 0 1920x1080x24 -ac -nolisten tcp >/root/prov/codex_xvfb.log 2>&1 &
xvfb=$!
echo "$xvfb" >/root/codex-broker-ada590/xvfb.pid
cat >/root/codex-broker-ada590/bin/pgrep <<'STUB'
#!/usr/bin/env bash
if [ "$*" = '-o -x Xorg' ]; then cat /root/codex-broker-ada590/xvfb.pid; else exec /usr/bin/pgrep "$@"; fi
STUB
chmod 755 /root/codex-broker-ada590/bin/pgrep
bash scripts/bench/display/interactive.sh prep >/root/prov/ada590_broker_x11_prep.log 2>&1
[ -s /root/codex-broker-ada590/vm.uuid ] || cat /proc/sys/kernel/random/uuid > /root/codex-broker-ada590/vm.uuid
export KF_EXTRA_QEMU="-uuid $(cat /root/codex-broker-ada590/vm.uuid)"
export KF_RUN_DIR=/root/codex-broker-ada590/validation-ada590
bash scripts/bench/display/interactive.sh run >/root/prov/ada590_broker_x11_run.log 2>&1 </dev/null &
launcher=$!

finish() {
  if declare -p G >/dev/null 2>&1; then
    timeout 20 "${G[@]}" sudo systemctl poweroff >/root/prov/ada590_poweroff.log 2>&1 || true
    for _ in $(seq 30); do kill -0 "$launcher" 2>/dev/null || break; sleep 1; done
  fi
  bash scripts/bench/display/interactive.sh stop >/root/prov/ada590_broker_x11_stop.log 2>&1 || true
  wait "$launcher" || true
  kill "$xvfb" 2>/dev/null || true
}
trap 'rc=$?; finish; echo "EXIT rc=$rc $(date -u -Is)"' EXIT
for _ in $(seq 60); do [ -s "$KF_INTERACTIVE_DIR/qemu.pid" ] && break; kill -0 "$launcher" || exit 2; sleep 1; done
python3 /root/ada590_broker_receipt.py "$KF_INTERACTIVE_DIR" "$KF_RUN_DIR" "$KF3_REV"
G=(ssh -i /workspace/bench/guest_key -o BatchMode=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=5 ubuntu@192.168.77.2)
ready=0
for i in $(seq 1 100); do
  kill -0 "$launcher" || { tail -8 /root/prov/ada590_broker_x11_run.log; exit 3; }
  if "${G[@]}" true 2>/dev/null; then ready=1; break; fi
  sleep 3
done
[ "$ready" = 1 ]
timeout 180 "${G[@]}" 'bash -s' >/root/prov/ada590_broker_x11_guest.log 2>&1 <<'GUEST'
set -u
export DISPLAY=:0 XAUTHORITY=/home/ubuntu/.Xauthority
for i in $(seq 1 40); do pgrep -x cinnamon >/dev/null && break; sleep 2; done
echo "CINNAMON $(pgrep -c -x cinnamon) XORG $(pgrep -c -x Xorg)"
test "$(nvidia-smi --query-gpu=driver_version --format=csv,noheader)" = 590.48.01 || exit 1
test "$(modinfo -F version nvidia)" = 590.48.01 || exit 1
cuda=$(readlink -f /usr/lib/x86_64-linux-gnu/libcuda.so.1)
echo "ACTUAL_LIBCUDA=$cuda ACTUAL_KERNEL=$(uname -r)"
test "${cuda##*.so.}" = 590.48.01 || exit 1
nvidia-smi --query-gpu=name,driver_version,memory.used --format=csv,noheader
glxinfo -B | grep -E 'OpenGL renderer|OpenGL vendor'
timeout 20 stdbuf -oL glxgears >/tmp/codex-gears.log 2>&1
cat /tmp/codex-gears.log
timeout 10 vkcube >/tmp/codex-vkcube.log 2>&1; echo "VKCUBE rc=$?"
echo "GUEST_DMESG flip_timeouts=$(sudo dmesg | grep -c 'Flip event timeout') nvassert=$(sudo dmesg | grep -ci nvAssert) xid=$(sudo dmesg | grep -c Xid)"
echo GUEST_DONE
GUEST
cat /root/prov/ada590_broker_x11_guest.log
grep -q '^CINNAMON 1 XORG 1$' /root/prov/ada590_broker_x11_guest.log
grep -q '^OpenGL renderer.*NVIDIA' /root/prov/ada590_broker_x11_guest.log
grep -q '^GUEST_DONE$' /root/prov/ada590_broker_x11_guest.log
grep -aE 'SHM|shm|frame|present|commit' /root/codex-broker-ada590/validation-ada590/broker.log | tail -8
DISPLAY=:99 xwininfo -root -tree >/root/prov/codex_x11_windows.log
win=$(sed -n 's/^[[:space:]]*\(0x[0-9a-f]*\) "kayfabe guest".*/\1/p' /root/prov/codex_x11_windows.log | head -1)
[ -n "$win" ]
DISPLAY=:99 xwd -id "$win" -silent -out /root/codex-broker-ada590/window.xwd
python3 - <<'PIXELS'
import hashlib, struct
p=open('/root/codex-broker-ada590/window.xwd','rb').read()
h=struct.unpack('>25I',p[:100]); width,height,bpp,stride=h[4],h[5],h[11],h[12]
assert h[1]==7 and bpp==32 and 0<width<=1920 and 0<height<=1080
pixels=p[h[0]+h[19]*12:]; assert len(pixels)>=stride*height
colors={pixels[y*stride+x*4:y*stride+x*4+4] for y in range(height) for x in range(width)}
print(f'X11_WINDOW width={width} height={height} colors={len(colors)} sha256={hashlib.sha256(pixels).hexdigest()}')
assert len(colors)>50, 'the presented desktop must contain rendered pixels'
PIXELS
# The guest's dmesg and host journal are checked independently of the launcher's status.
grep -q 'GUEST_DMESG flip_timeouts=0 .*xid=0' /root/prov/ada590_broker_x11_guest.log
echo X11_PRESENTATION_CHECKED=1
echo "DISPLAY_STALLS=$(grep -ac 'display: STALL' /root/codex-broker-ada590/validation-ada590/qemu.log || true)"
xid_after=$(dmesg | grep -c "NVRM: Xid" || true)
echo "HOST_XID before=$xid_before after=$xid_after"
[ "$xid_before" = "$xid_after" ]
dmesg | grep 'NVRM: Xid' >/root/prov/ada590_broker_xids.after.txt || true
cmp /root/prov/ada590_broker_xids.before.txt /root/prov/ada590_broker_xids.after.txt
echo BROKER_DESKTOP_VERDICT=PASS host595.91.07 guest590.48.01
