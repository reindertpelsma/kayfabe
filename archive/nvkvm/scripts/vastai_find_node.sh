#!/usr/bin/env bash
# vastai_find_node.sh — find suitable vast.ai nodes for nvkvm integration testing
#
# Requirements: vastai CLI, API key configured
# Usage: ./scripts/vastai_find_node.sh [--create]

set -euo pipefail

DISK_GB=40
MIN_INET_UP=300     # Mbps
REGION="EU"         # prefer West Europe nodes

echo "Searching for GPU instances with KVM support in $REGION..."

# Search for instances with:
#   - KVM enabled (nested virt support)
#   - Any NVIDIA GPU
#   - Reasonable disk space
#   - Low latency from EU
vastai search offers \
    'rentable=true vms_enabled=true disk_space>='"$DISK_GB"' inet_up>'"$MIN_INET_UP"' cuda_max_good>=12.0' \
    --type on-demand \
    --order 'dph_total+' \
    2>/dev/null | head -20

echo ""
echo "To create an instance (replace OFFER_ID):"
echo "  vastai create instance OFFER_ID \\"
echo "    --image nvidia/cuda:12.2.0-devel-ubuntu22.04 \\"
echo "    --disk $DISK_GB \\"
echo "    --env '-e JUPYTER_DIR=/' \\"
echo "    --jupyter-lab \\"
echo "    --direct \\"
echo "    --args \"\""
echo ""
echo "Then SSH in and run:"
echo "  ./scripts/vastai_setup.sh"
