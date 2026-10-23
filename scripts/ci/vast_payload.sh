#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
set -euo pipefail
exec python3 -I "$(dirname "$0")/vast_payload.py" "$@"
