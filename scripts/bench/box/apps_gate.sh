#!/bin/bash
# Linux apps gate: Python port; row/verdict contract in V3_KFCI_STEPS.md.
# DISABLED BY DEFAULT: vast_payload.py still owns stage selection.
# Usage: apps_gate.sh <tag> <prov_dir> [full|middle]
# The remaining matrix/provisioning helpers are unchanged.
set -u
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# -I ignores ambient Python imports; main.py bootstraps the trusted package.
exec python3 -I "$here/../../kfci/main.py" apps-gate "$@"
# Exit codes: PASS=0, FAIL=1, RETRY=3, usage=2.
