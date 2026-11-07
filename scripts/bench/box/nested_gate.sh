#!/usr/bin/env bash
# DESIGN-ONLY hardware path; disabled unless KF_NESTED=1 on the large payload.
# nested_gate.sh <tag> <prov_dir> <full revision>
exec python3 "$(dirname "${BASH_SOURCE[0]}")/nested_gate.py" "$@"
