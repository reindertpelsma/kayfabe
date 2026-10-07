#!/usr/bin/env bash
# POST_CAPTURE_HOOK: the graded gfx steps (gfx_hook.sh), then HOLD the guest for diagnosis
# (gfx/hold_hook.sh) — one boot for both the verdict and the follow-up questions.
HERE="$(cd "$(dirname "$0")" && pwd)"
"$HERE/../gfx_hook.sh" "$@"
"$HERE/hold_hook.sh" "$@"
