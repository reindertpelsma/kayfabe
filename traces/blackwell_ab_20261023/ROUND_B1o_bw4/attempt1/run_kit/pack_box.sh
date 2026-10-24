#!/bin/bash
# Box-wide private capture before teardown: /root/prov, every kfwin VM log, the full host
# kernel journal; sha256 manifest. Private (dev box only), never published.
set -euo pipefail
umask 077
out=/root/capture/box
mkdir -p "$out"
tar -czf "$out/prov.tgz" -C /root prov
tar -czf "$out/kfwin_logs.tgz" -C /workspace/bw-capc-validation/kfwin logs
timeout 30 journalctl -k -b -o short-iso-precise --no-pager >"$out/host_kernel_full.txt"
ls -1 /workspace/bench/run_bw4_* 2>/dev/null | tar -czf "$out/bench_runs.tgz" -T - || true
(cd "$out" && find . -type f ! -name MANIFEST.sha256 -printf '%P\n' | sort |
 xargs -r sha256sum >MANIFEST.sha256)
echo "PACK_BOX_DONE $(wc -l <"$out/MANIFEST.sha256") files $(date -u -Is)"
