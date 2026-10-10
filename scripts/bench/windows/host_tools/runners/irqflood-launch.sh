#!/bin/bash
# irqflood-launch.sh N [FLOODWORD] — run N of the interrupt-flood matrix (run 104 flags; binary b728b480)
N=$1; FW=${2:-}
cd /var/lib/kf-windows-20261005/wreset
export WR_SHOTS=90
O=/var/lib/kf-windows-20261005/run$N-outer.log
echo "START$N $(date -u +%FT%T) flood=[$FW] xid=$(dmesg | grep -c "NVRM: Xid")" > $O
flock -o /tmp/kayfabe-fastguest.lock bash /var/lib/kf-windows-20261005/wreset/traces/windows_reset_20261009/wr-run.sh $N b728b480 "KF3_DISPLAY_CORE_AT_VBLANK KF3_DISPLAY_WRITE_TRACE KF3_DISPLAY_HDCP_STATE KF3_DISPLAY_PRIVATE_PROBE KF3_DISPLAY_HOTPLUG_EDID_SEEN KF3_DISPLAY_BLANK_STATE KF3_DISPLAY_ARMED_DEFAULTS KF3_DISPLAY_LOADV KF3_DISPLAY_CAPS_PROBE KF3_DISPLAY_LUT_MIRROR KF3_DISPLAY_ILUT_OFFSET_256 KF3_PT_STALL_SNAPSHOT KF3_DISPLAY_TRACE KF3_NO_BATCHED_MAP $FW" >> $O 2>&1
echo "EXIT$N rc=$? $(date -u +%FT%T) xid=$(dmesg | grep -c "NVRM: Xid")" >> $O
