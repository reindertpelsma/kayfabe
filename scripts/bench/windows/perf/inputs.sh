#!/bin/bash
# inputs.sh click|f9|key QCODE  : send input to the guest through QMP (kayfabe run 2991 or the vfio session)
S=${QMPSOCK:-/var/lib/kf-windows-20261005/boundary-kayfabe-2991/qmp.sock}; Q=/data/claude-perf/wt/scripts/bench/windows/perf/qmp1.py
case $1 in
 click) X=${2:-16384}; Y=${3:-16384}
  python3 $Q $S "{\"execute\":\"input-send-event\",\"arguments\":{\"events\":[{\"type\":\"abs\",\"data\":{\"axis\":\"x\",\"value\":$X}},{\"type\":\"abs\",\"data\":{\"axis\":\"y\",\"value\":$Y}}]}}" "{\"execute\":\"input-send-event\",\"arguments\":{\"events\":[{\"type\":\"btn\",\"data\":{\"button\":\"left\",\"down\":true}}]}}" "{\"execute\":\"input-send-event\",\"arguments\":{\"events\":[{\"type\":\"btn\",\"data\":{\"button\":\"left\",\"down\":false}}]}}";;
 key) python3 $Q $S "{\"execute\":\"send-key\",\"arguments\":{\"keys\":[{\"type\":\"qcode\",\"data\":\"$2\"}]}}";;
esac
