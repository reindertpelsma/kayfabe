#!/usr/bin/env bash
# POST_CAPTURE_HOOK: run llm_graph_probe.py stage by stage in the fat guest (one process per stage),
# with the HOST's Xid count read before and after each stage (a guest GR fault surfaces as a host
# Xid on the qemu process). Usage as a hook; stages from LGP_STAGES.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; G="$HERE/gssh_nv"
STAGES=${LGP_STAGES:-side_small side_model static_pre static_step capture}
xids() { sudo dmesg 2>/dev/null | grep -c "Xid (PCI" || true; }
$G 'cat > /opt/llm/llm_graph_probe.py' < "$HERE/llm_graph_probe.py" || { echo LGP_OUTCOME=install-failed; exit 0; }
for st in $STAGES; do
  b=$(xids)
  out=$($G "cd /opt/llm && HF_HOME=/opt/llm/hf timeout 600 /home/ubuntu/llmvenv/bin/python llm_graph_probe.py $st 2>&1" 2>&1 | tr -d '\r')
  a=$(xids)
  echo "$out" | grep -a '^PROBE_' | sed "s/^/LGP /"
  echo "LGP_XID stage=$st host_xid_lines_before=$b after=$a"
  [ "$a" != "$b" ] && sudo dmesg | grep -a "Xid (PCI" | tail -$((a - b)) | sed "s/^/LGP_XIDLINE stage=$st /"
done
