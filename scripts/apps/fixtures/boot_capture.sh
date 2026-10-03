#!/usr/bin/env bash
# test_hook.sh fixture: boot_capture.sh <tag> as apps_matrix.sh calls it, with no QEMU and no guest.
#   FAKE_DIE=1      dies before writing anything (as boot_capture.sh's phase-0 precondition does)
#   otherwise       writes the tag's kf3 log ($BENCH_DIR/run_<tag>_qemu.log: the status line rendered
#                   by kf3_lines.py, rc[unarmed=$FAKE_UNARMED none=0]) and one PASS APPRES row per app
#                   in $APPS to $APPS_OUT/guest.res, as apps_hook.sh would
[ "${FAKE_DIE:-0}" = 1 ] && { echo "FAILED precondition (fixture)"; exit 1; }
TAG=${1:?tag}; U=${FAKE_UNARMED:-0}
python3 "${FAKE_FIX:?}/../kf3_lines.py" rc_status "$U" 0 > "${BENCH_DIR:?}/run_${TAG}_qemu.log"
for a in ${APPS:?}; do
  echo "APPRES side=guest app=$a verdict=PASS rc=0 secs=1 quiet=0 note=- boot=$TAG guest_xid=0 kf3_lines=1 kf3_refusals=0 rc_unarmed=$U rc_none=0 rc_silent_births=0 loud=-" >> "${APPS_OUT:?}/guest.res"
done
