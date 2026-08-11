#!/usr/bin/env bash
set -uo pipefail
S=/tmp/claude-0/-workspace-nvidia-gpu-passthrough/85f9c0ab-d91a-47ab-b068-c099d453dd8f/scratchpad
cd /workspace/wt-w259 || exit 2
echo "START $(date -Is)"; df -h / | tail -1
while IFS=$'\t' read -r f l n; do
  "$S/adj.sh" "$f" "$l" "$n"
done < "$1"
echo "TERMINATOR_OK $(date -Is)"; df -h / | tail -1
echo "tree: [$(git status --porcelain | tr '\n' ' ')]"
