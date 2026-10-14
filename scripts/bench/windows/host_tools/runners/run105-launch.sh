#!/bin/bash
cd /var/lib/kf-windows-20261005/wreset
export WR_SHOTS=90
echo "START105 $(date -u +%FT%T)" > /var/lib/kf-windows-20261005/run105-outer.log
flock -o /tmp/kayfabe-fastguest.lock bash /var/lib/kf-windows-20261005/wreset/traces/windows_reset_20261009/wr-run.sh 105 3e9bcdce "KF3_DISPLAY_CORE_AT_VBLANK KF3_DISPLAY_WRITE_TRACE KF3_DISPLAY_HDCP_STATE KF3_DISPLAY_PRIVATE_PROBE KF3_DISPLAY_HOTPLUG_EDID_SEEN KF3_DISPLAY_BLANK_STATE KF3_DISPLAY_ARMED_DEFAULTS KF3_DISPLAY_LOADV KF3_DISPLAY_CAPS_PROBE KF3_DISPLAY_LUT_MIRROR KF3_DISPLAY_ILUT_OFFSET_256 KF3_PT_STALL_SNAPSHOT KF3_DISPLAY_TRACE KF3_NO_BATCHED_MAP KF3_PT_NSI_RELAY=0" >> /var/lib/kf-windows-20261005/run105-outer.log 2>&1
echo "EXIT105 rc=$? $(date -u +%FT%T)" >> /var/lib/kf-windows-20261005/run105-outer.log
