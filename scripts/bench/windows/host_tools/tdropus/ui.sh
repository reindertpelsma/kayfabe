#!/bin/bash
# ui.sh RUN cmd args — drive a running guest: shot NAME | key QCODE | click PX PY (pixels of 1920x1080) | combo K1 K2
W=/var/lib/kf-windows-20261005; R=$1; shift; S=$W/boundary-kayfabe-$R/qmp.sock; O=$W/winprod/run$R
Q(){ timeout 15 python3 $W/boundary-tools/qmp.py $S "$@"; }
case $1 in
 shot) Q cmd screendump "{\"filename\":\"$O/$2.ppm\",\"device\":\"kf0\"}" >/dev/null && convert $O/$2.ppm $O/$2.png && rm -f $O/$2.ppm && echo $O/$2.png;;
 key) Q cmd send-key "{\"keys\":[{\"type\":\"qcode\",\"data\":\"$2\"}]}" >/dev/null;;
 combo) Q cmd send-key "{\"keys\":[{\"type\":\"qcode\",\"data\":\"$2\"},{\"type\":\"qcode\",\"data\":\"$3\"}]}" >/dev/null;;
 click) x=$(( $2 * 32767 / 1919 )); y=$(( $3 * 32767 / 1079 ));
   Q cmd input-send-event "{\"events\":[{\"type\":\"abs\",\"data\":{\"axis\":\"x\",\"value\":$x}},{\"type\":\"abs\",\"data\":{\"axis\":\"y\",\"value\":$y}}]}" >/dev/null; sleep 0.15
   Q cmd input-send-event "{\"events\":[{\"type\":\"btn\",\"data\":{\"down\":true,\"button\":\"left\"}}]}" >/dev/null; sleep 0.1
   Q cmd input-send-event "{\"events\":[{\"type\":\"btn\",\"data\":{\"down\":false,\"button\":\"left\"}}]}" >/dev/null; echo "abs $x,$y";;
esac
