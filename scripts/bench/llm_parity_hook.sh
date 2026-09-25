#!/usr/bin/env bash
# POST_CAPTURE_HOOK wrapper (boot_capture.sh calls a hook as `<hook> <tag>`) for the LLM parity
# matrix in the fat guest on kf3. Typical boot (one boot = the whole guest lane, one ledger):
#   KF_DEVICE=kf3 NVKVM_RAM_MB=8192 GQ_TIMEOUT=600 POST_CAPTURE_HOOK=$PWD/scripts/bench/llm_parity_hook.sh \
#     bash scripts/bench/boot_capture.sh llm_<tag>
exec "$(cd "$(dirname "$0")" && pwd)/llm_parity.sh" hook "$@"
