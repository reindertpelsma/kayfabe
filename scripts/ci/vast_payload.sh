#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
set -euo pipefail
[[ "$#" -eq 2 && "$1" =~ ^[0-9a-f]{40}$ && "$2" =~ ^[0-9a-f]{32}$ ]] || exit 2
here=$(cd "$(dirname "$0")" && pwd)
python3 -I "$here/vast_setup.py" restore "$(cd "$here/../.." && pwd)"
export KF_APT_CACHE_ROOT=/workspace/kf-setup-cache CARGO_NET_OFFLINE=true
exec python3 -I "$here/vast_payload.py" "$@"
