#!/usr/bin/env bash
# host.py saves bindings, binds BOTH group functions, and restores after QEMU exit.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
exec python3 "$here/host.py" run --arm C --config "${1:?configuration file}"
