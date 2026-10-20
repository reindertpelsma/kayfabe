#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Run on the trusted host, with all private paths supplied by the operator.
set -euo pipefail
: "${N1_ROOT:?lane host directory required}" "${N1_LOCK:?shared lock required}" ":${N1_LAUNCHER:?owner launcher required}"
script=run.py
if [[ ${1:-} == --campaign ]]; then script=campaign.py; shift; fi
exec flock -w 120 -o "$N1_LOCK" python3 "$(dirname "$0")/$script" "$@"
