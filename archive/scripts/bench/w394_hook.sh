#!/usr/bin/env bash
# POST_CAPTURE_HOOK adapter for the w394 CUDA-apps suite.
# boot_capture.sh calls its hook as `$HOOK <TAG>`; the suite's first argument is the ARM.
# ⊘ Without this the suite would read the tag as the arm name and run neither.
exec "$(cd "$(dirname "$0")" && pwd)/w394_cuda_suite.sh" guest "${1:-w394}"
