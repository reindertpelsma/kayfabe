#!/usr/bin/env bash
# POST_CAPTURE_HOOK glue for the llm_parity stage (boot_capture.sh calls a hook as `<hook> <tag>` with the guest up).
# All logic is in llm_parity.py (guest-hook); it reads LLM_PARITY_HOOK_CFG, set by the stage.
set -euo pipefail
exec python3 "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/llm_parity.py" guest-hook "$@"
