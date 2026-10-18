#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Host-lane entry point. No VM/GPU action was run by L5.
set -euo pipefail
HERE=$(cd -- "$(dirname -- "$0")" && pwd)
# Exit 75 if the shared resource is held; never break or bypass its lock.
exec flock -w 60 -E 75 -o /tmp/kayfabe-fastguest.lock python3 "$HERE/run.py" "$@"
