#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Interactive image build; see image_session.sh --help and docs/design/V3_WINDOWS_IMAGE.md.
set +x
set -euo pipefail
umask 077
exec python3 "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")/image_tools.py" session "$@"
