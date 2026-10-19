#!/usr/bin/env bash
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
exec python3 "$here/host.py" sample --pid "${1:?exact owned QEMU PID}" --seconds "${2:-600}"
