#!/bin/bash
set -euo pipefail
cd /root/kayfabe
journalctl -k -b -o short-iso-precise --no-pager >/root/prov/post_windows_kernel_snapshot.log
python3 scripts/bench/native_xid_guard.py check /root/prov/post_windows_kernel_snapshot.log /root/prov/native_xid_baseline.json
