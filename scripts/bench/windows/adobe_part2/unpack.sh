#!/bin/bash
# unpack.sh : wait for the download's EXIT rc=0 line (dl.py, sha256 checked against the pin), then the verified unpack of the stored zip
# (propimage.py unpack: members extracted and sha256 compared with manifest.json, state.json validated). Log /workspace/img/unpack.log, last line UNPACK_EXIT rc=N.
set -u
mkdir -p /workspace/img
exec > /workspace/img/unpack.log 2>&1
until grep -q '^.* EXIT rc=' /workspace/dl/image.zip.log; do sleep 15; done
grep -q 'EXIT rc=0' /workspace/dl/image.zip.log || { echo "UNPACK_EXIT rc=99 (download not ok)"; exit 99; }
until [ -f /root/kayfabe/scripts/bench/windows/propimage.py ]; do sleep 10; done
echo "START $(date -u +%FT%TZ)"
cd /root/kayfabe
python3 -B -I scripts/bench/windows/propimage.py unpack /workspace/dl/image.zip --dir /workspace/img/unpacked
rc=$?
ls -la /workspace/img/unpacked | head
echo "UNPACK_EXIT rc=$rc $(date -u +%FT%TZ)"
