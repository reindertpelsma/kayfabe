#!/bin/sh
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Deprecated pointer only. Packaging no longer uses a shell.
printf '%s\n' 'Deprecated: use Python 3 directly (no shell is required):' 'python3 -I scripts/bench/windows/appmatrix/package_bundle.py check' 'Then fetch, build, verify-image; see docs/DOWNLOADS.md, Windows app bundle.'
exit 2
