#!/bin/sh
# baton in-guest driver v3 (busybox ash), installed as /bin/kftool in the fast-guest initrd.
# KF_BATON_SEQ = comma list of  MODE:DUR[:key=val...]   (commas separate runs, ':' fields)
#   MODE  pair | tri | hl (heavylight) | solo | quad
#   keys  k=<ms> (5)  hk=<ms> (50, heavy)  burst=<N> (1)  blocks=<N>  threads=<N>  spin=1
#         bburst=<N> (burst of client B only)  wd=<ms> (2000)
# Every client runs with the self-watchdog (--wd-out) and heartbeat files shared in $O/hb.
export LD_LIBRARY_PATH=/usr/lib/x86_64-linux-gnu
O=/tmp/o; mkdir -p $O
B=/bin/baton
maj=$(awk '/nvidia-frontend|^ *[0-9]+ nvidia$/ {print $1; exit}' /proc/devices)
[ -e /dev/nvidia-modeset ] || mknod /dev/nvidia-modeset c "$maj" 254 2>&1
echo "BATON: guest date $(date +%s) uname $(uname -r) ncpu=$(grep -c ^processor /proc/cpuinfo) seq=$KF_BATON_SEQ"
CPM=$($B --calibrate-only --k-ms 5 2>$O/calib.err | sed -n 's/^CYCLES_PER_MS=//p')
echo "BATON: calibrate cpm=[$CPM] $(cat $O/calib.err | tr '\n' ' ')"
[ -n "$CPM" ] || { dmesg | grep -a -E 'Xid|NVRM' | tail -30 | sed 's/^/BATON: dmesg /'; echo "BATON: CALIBRATION FAILED"; exit 2; }
idx=0
for item in $(echo "$KF_BATON_SEQ" | tr ',' ' '); do
  idx=$((idx+1))
  IFS=: read mode dur rest <<EOT
$item
EOT
  k=5; hk=50; burst=1; bburst=; blocks=; threads=; spin=; wd=2000
  for kv in $(echo "$rest" | tr ':' ' '); do
    case $kv in k=*) k=${kv#k=};; hk=*) hk=${kv#hk=};; burst=*) burst=${kv#burst=};; bburst=*) bburst=${kv#bburst=};;
      blocks=*) blocks=${kv#blocks=};; threads=*) threads=${kv#threads=};; spin=1) spin=--spin;; wd=*) wd=${kv#wd=};; esac
  done
  name="${mode}${idx}"
  xid0=$(dmesg | grep -c Xid)
  START=$(( $(date +%s) + 8 ))
  PIDS=""
  rm -rf $O/hb; mkdir -p $O/hb
  launch() { # label k burst
    $B --label $1 --k-ms $2 --burst $3 --duration $dur --cycles-per-ms $CPM --start-at $START --dump-all $O/$name.$1.csv \
       --wd-ms $wd --wd-out $O/$name.$1.wd --hb-dir $O/hb ${blocks:+--blocks $blocks} ${threads:+--threads $threads} $spin \
       >$O/$name.$1.log 2>$O/$name.$1.err &
    PIDS="$PIDS $!"
  }
  case $mode in
    solo) launch A $k $burst;;
    pair) launch A $k $burst; launch B $k ${bburst:-$burst};;
    tri) launch A $k $burst; launch B $k $burst; launch C $k $burst;;
    quad) launch A $k $burst; launch B $k $burst; launch C $k $burst; launch D $k $burst;;
    hl) launch light $k $burst; launch heavy $hk $burst;;
  esac
  echo "BATON: RUN_BEGIN $name mode=$mode dur=$dur k=$k burst=$burst bburst=$bburst blocks=$blocks threads=$threads spin=$spin start_epoch=$START guest_date=$(date +%s) xid0=$xid0 pids=$PIDS"
  ( sleep $((dur+60)); echo "BATON: WATCHDOG FIRED $name"; kill -TERM $PIDS 2>/dev/null ) &
  WD=$!
  ( while :; do sleep 2; x=$(dmesg | grep -c Xid); if [ "$x" -gt "$xid0" ]; then echo "BATON: XID ROSE in guest ($x > $xid0)"; kill -TERM $PIDS 2>/dev/null; break; fi; done ) &
  XW=$!
  for p in $PIDS; do wait $p; echo "BATON: pid $p rc=$?"; done
  kill $WD $XW 2>/dev/null
  echo "BATON: RUN_END $name guest_date=$(date +%s) xid_now=$(dmesg | grep -c Xid)"
  dmesg | grep -a -E 'Xid|NVRM' | tail -20 | sed "s/^/BATON: dmesg $name /"
  for f in $O/$name.*.log $O/$name.*.csv $O/$name.*.err $O/$name.*.wd; do
    [ -s "$f" ] || continue
    echo "BATON_FILE_BEGIN $(basename $f)"
    cat "$f"
    echo "BATON_FILE_END $(basename $f)"
  done
  rm -f $O/$name.*
  sleep 3
done
echo "BATON: ALL DONE"
exit 0
