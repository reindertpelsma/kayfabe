#!/usr/bin/env bash
# Shared bench wrapper; all timing/retry decisions live in readiness.py.
READINESS_HELPER=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/readiness.py
readiness_stage() {
    READINESS_STAGE=${1:?stage receipt}; local nominal=${2:?} ceiling=${3:?}
    python3 "$READINESS_HELPER" stage --out "$READINESS_STAGE" \
        --profile "${WINVM_READINESS_PROFILE:?}" --lease "${WINVM_READINESS_LEASE:?}" \
        --nominal "$nominal" --ceiling "$ceiling"
}
readiness_wait() {
    local name=${1:?}; shift
    [[ $name =~ ^[a-z0-9_]+$ ]] || return 2
    python3 "$READINESS_HELPER" wait --profile "$WINVM_READINESS_PROFILE" \
        --stage "$READINESS_STAGE" --nominal 450 --ceiling 2700 --marker KF_SSH_UP \
        --receipt "$(dirname "$READINESS_STAGE")/${name}_readiness.json" -- "$@"
}
readiness_step() {
    local mode=${1:?} name=${2:?} nominal=${3:?} ceiling=${4:?}; shift 4
    [[ $name =~ ^[a-z0-9_]+$ ]] || return 2
    python3 "$READINESS_HELPER" "$mode" --profile "$WINVM_READINESS_PROFILE" \
        --stage "$READINESS_STAGE" --nominal "$nominal" --ceiling "$ceiling" \
        --receipt "$(dirname "$READINESS_STAGE")/${name}_readiness.json" -- "$@"
}
