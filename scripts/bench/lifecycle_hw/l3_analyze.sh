#!/bin/bash
# l3_analyze.sh <evdir> <tag>... : per broker-lane run, the lifecycle/USER-ring census, the birth census, the REFUSED/refused signature
# (numbers stripped), the relay/broker counters and the guest checks, so two revisions can be diffed.
EV=$1; shift
norm() { sed -E 's/0x[0-9a-fA-F_]+/0xN/g; s/[0-9]+/N/g; s/VasKey\([^)]*\)/VasKey(N)/' ; }
for T in "$@"; do
  R=$(grep RUN_DIR "$EV/l3_$T.txt" | cut -d= -f2); Q=$R/qemu.log
  rev=$(grep -o 'kf3=[0-9a-f]*' "$EV/l3_$T.txt" | head -1)
  echo "=== $T $rev run=$R"
  echo "births=$(grep -a 'kf-host: channel birth ' "$Q" | wc -l) user=$(grep -a 'kf-host: channel birth ' "$Q" | grep -ac ' PRIVILEGED_CHANNEL=0 privilege=USER ')"
  grep -a 'kf-host: channel birth ' "$Q" | grep -av ' PRIVILEGED_CHANNEL=0 privilege=USER ' | cut -c1-200 | sed 's/^/  non-USER-birth: /'
  echo "USER_RING_lines=$(grep -ac 'USER_RING' "$Q") LIFECYCLE_lines=$(grep -aci 'LIFECYCLE' "$Q") pointer_pb_read_violation_lines=$(grep -ac 'pointer_pb_read_violation' "$Q")"
  echo "REFUSED_signature_count=$(grep -aE 'REFUSED|refused by' "$Q" | norm | sort | uniq -c | wc -l)"
  grep -aE 'REFUSED|refused by' "$Q" | norm | sort | uniq -c | sed 's/^/  /' | cut -c1-200
  echo "broker_client: $(grep -a 'client detached' "$R/broker.log" | tail -1 | cut -c1-160)"
  echo "reclaimed_frame_slots(last)=$(grep -ao 'reclaimed frame slot [0-9]: .* ([0-9]* reclaimed so far)' "$Q" | tail -1 | grep -o '([0-9]* reclaimed' )"
  echo "display: $(grep -a 'kf3: display fps' "$Q" | tail -1 | cut -c1-200)"
  echo "guest: $(grep -a 'NVRM version' "$EV/guest_checks_$T.txt" | head -1 | cut -c1-70) | $(grep -a 'GL_RENDERER' "$EV/guest_checks_$T.txt" | head -1) | glxgears: $(grep -a 'frames in' "$EV/guest_checks_$T.txt" | sed 's/.*= //' | tr '\n' ' ')"
  echo "KMS: $(grep -a 'modeset=' "$EV/guest_checks_$T.txt" | head -1) | $(grep -a 'status=' "$EV/guest_checks_$T.txt" | head -1)"
  echo "desktop: $(grep -ac 'cinnamon' "$EV/guest_checks_$T.txt") cinnamon lines; Xorg=$(grep -ac 'Xorg' "$EV/guest_checks_$T.txt")"
  echo "shutdown: $(grep -a 'L3_GUEST_POWEROFF_CLEAN\|L3_ACPI\|L3_NOT_STOPPED\|L3_GUEST_POWEROFF_NOT' "$EV/l3_$T.txt" | tr '\n' ';')"
  echo "xid_journal_lines=$(grep -a 'XID_JOURNAL' "$EV/l3_$T.txt" | grep -o 'lines=[0-9]*')"
  echo "qemu_exit: $(grep -a 'QEMU exited' "$EV/launcher_$T.out" | sed 's/(log.*//')"
  gd="$EV/guest_dmesg_$T.txt"
  [ -s "$gd" ] && echo "guest_dmesg: $(grep -aciE 'NVRM|nvidia' "$gd") nvidia lines; NVRM-signature=$(grep -a 'NVRM' "$gd" | sed 's/^\[ *[0-9.]*\] //' | norm | sort -u | md5sum | cut -c1-12) drm-signature=$(grep -aE '\[drm\]|nvidia-drm' "$gd" | sed 's/^\[ *[0-9.]*\] //' | norm | sort -u | md5sum | cut -c1-12)"
done
