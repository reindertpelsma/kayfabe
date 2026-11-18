#!/bin/bash
# ogr_boot.sh TAG "name=listfile ..." : one kf3 guest boot (apps image), the vkcts cases in $2 one after the other (ogr_hook.sh). Serial on the bench lock like apps_matrix.sh.
TAG=$1; export CASES=$2
R=/workspace/kf-ci-src; OUT=/workspace/apps/results/$TAG; mkdir -p "$OUT"
QB=${KF3_BIN:-$(ls -td /workspace/bench/kf3-bins/*/qemu-system-x86_64 | head -1)}
echo "OGR_BOOT tag=$TAG kf3=$QB cases=[$CASES] $(date -u +%T)"
exec 9>/tmp/kayfabe-fastguest.lock; flock 9
env KF_DEVICE=kf3 QEMU_BIN="$QB" NVKVM_RAM_MB=16384 KF_SMP=6 GQ_TIMEOUT=300 KF_GUEST_IMG=/workspace/bench/guest_apps.qcow2 CASES="$CASES" \
    APPS_OUT="$OUT" POST_CAPTURE_HOOK=/workspace/ogr/ogr_hook.sh bash "$R/scripts/bench/boot_capture.sh" "$TAG" > "$OUT/boot_$TAG.driver.log" 2>&1
echo "OGR_BOOT_END rc=$? $(date -u +%T)"
for x in dmesg dmesg_after probe hostdmesg serial; do cp -f "/workspace/bench/run_${TAG}_$x.log" "$OUT/boot_$TAG.$x.log" 2>/dev/null; done
grep -a "OGR_" "/workspace/bench/run_${TAG}_probe.log" | cut -c1-300
