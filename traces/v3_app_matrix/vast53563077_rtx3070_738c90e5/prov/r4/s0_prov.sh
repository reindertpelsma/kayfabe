#!/bin/bash
# s0_prov.sh — after provision_full.sh READY: the app bundle + apps guest (a COPY of guest.qcow2, taken
# before any graphics provisioning) and the graphics set (host Khronos side + the fat guest image).
. /root/r4/common.sh
echo "S0_START $(date -Is) rev=$REV"
until grep -q '^EXIT' /root/prov/prov.log 2>/dev/null; do
  pgrep -f '[p]rovision_full.sh' >/dev/null || { sleep 5; grep -q '^EXIT' /root/prov/prov.log || { echo "PROVFULL_DIED (no EXIT line)"; exit 3; }; }
  sleep 20
done
[ -f /root/prov/READY ] || { echo "PROVFULL_NOT_READY"; grep -a '_RC=\|READY' /root/prov/prov.log; exit 4; }
grep -a '_RC=\|READY\|REBOOT' /root/prov/prov.log
[ -x "$QB" ] || { echo "NO_KF3_BIN $QB"; exit 5; }
alive_qemu && { echo "A QEMU is up before s0"; exit 6; }
step img_copy cp --sparse=always $B/guest.qcow2 $APPIMG
ls -la $B/guest.qcow2 $APPIMG
step gfx_host bash scripts/bench/provision_guest_gfx.sh host
[ -e /opt/apps ] || ln -sfn /workspace/apps /opt/apps
step bundle bash scripts/apps/build_bundle.sh
grep -a 'BUILD_.*FAIL\|BUNDLE ' $L/bundle.log
step setup_host bash scripts/apps/setup_side.sh
grep -a 'SETUP_\|IMPORT_OK\|HF_MODEL' $L/setup_host.log
step guest_apps env KF_GUEST_IMG=$APPIMG bash scripts/apps/provision_guest_apps.sh
step gfx_guest bash scripts/bench/provision_guest_gfx.sh
step gset_prov bash scripts/bench/gfxset/provision.sh
cat $B/gfxset.receipt 2>/dev/null
df -h / | tail -1
echo "S0_EXIT rc=0 $(date -Is)"
