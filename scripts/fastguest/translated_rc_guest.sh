#!/bin/sh
# Guest initrd client. No change to the frozen ladder.
set -eu
if [ "${1:-}" = --gpu ]; then
    [ "${2:-}" = 0 ] || exit 2
    shift 2
fi
mode=${1:?on or off}
run=${2:?run identifier}
case "$mode" in on|off) ;; *) exit 2;; esac
# Opening the device initializes the adapter before reading its derived UUID.
exec 3</dev/nvidia0
set -- /proc/driver/nvidia/gpus/*/information
[ "$#" = 1 ] || exit 2
uuid=$(sed -n 's/^GPU UUID:[[:space:]]*GPU-//p' "$1" | tr -d '-')
[ "${#uuid}" = 32 ] || exit 2
/bin/kf_rc_owner /lib/modules/kf_rc_probe.ko "$uuid" "$run" "$mode"
cat /proc/uptime >/dev/null
echo "RC_CONTROL_HEALTH run=$run result=PASS"
