#!/bin/bash
set -euo pipefail
exec python3 scripts/ci/vast_dummy.py "$@"
