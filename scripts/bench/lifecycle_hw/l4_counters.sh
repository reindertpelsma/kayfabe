#!/bin/bash
# l4_counters.sh <benchdir> <tag> : per arm, from the QEMU log, the channel-birth census and every lifecycle / USER-ring line.
B=$1; T=$2
printf '%-24s %6s %6s %5s %5s | %9s %9s %6s %6s\n' ARM births USER priv refus USER_RING LIFECYCLE pbviol modeSW
tb=0; tu=0; tp=0; tr=0; tl=0; tu2=0; tv=0; tm=0
for q in "$B"/fast_${T}_*_qemu.log; do
  a=$(basename "$q" _qemu.log); a=${a#fast_${T}_}
  b=$(grep -a 'kf-host: channel birth ' "$q" | wc -l)
  u=$(grep -a 'kf-host: channel birth ' "$q" | grep -ac ' PRIVILEGED_CHANNEL=0 privilege=USER ')
  p=$(grep -a 'kf-host: channel birth ' "$q" | grep -ac 'PRIVILEGED_CHANNEL=1')
  r=$(grep -acE 'PRIVILEGED CHANNEL REFUSED|CHANNEL BIRTH REFUSED|CHANNEL CLASS REFUSED|CUDA THREAD REFUSED' "$q")
  ur=$(grep -ac 'USER_RING' "$q")
  lc=$(grep -aci 'LIFECYCLE' "$q")
  pv=$(grep -a 'pointer_pb_read_violation' "$q" | grep -avc 'pointer_pb_read_violation: *0\|pointer_pb_read_violation=0')
  ms=$(grep -aEc 'mode=(Scan|Pointer|Window)|to_pointer|to_window|entries_(scan|pointer|window)' "$q")
  printf '%-24s %6s %6s %5s %5s | %9s %9s %6s %6s\n' "$a" $b $u $p $r $ur $lc $pv $ms
  tb=$((tb+b)); tu=$((tu+u)); tp=$((tp+p)); tr=$((tr+r)); tl=$((tl+lc)); tu2=$((tu2+ur)); tv=$((tv+pv)); tm=$((tm+ms))
done
echo "TOTAL tag=$T births=$tb user=$tu privileged=$tp refused=$tr USER_RING_lines=$tu2 LIFECYCLE_lines=$tl pb_violation_lines=$tv mode/transition_lines=$tm"
