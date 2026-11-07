#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Offline packaging, caller-signed upload/download; no Cloudflare credentials or Worker calls.
set +x
set -euo pipefail
umask 077
exec python3 "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")/image_tools.py" package "$@"
