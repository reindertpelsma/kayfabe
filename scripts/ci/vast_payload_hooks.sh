#!/usr/bin/env bash
# Narrow instrumentation around existing raw-client / guest timeout calls.
# Loaded only by the payload's child shells; no suite/build implementation lives here.
if [ -n "${KF_PAYLOAD_ROOT:-}" ]; then
    timeout() {
        local arg arm='' kind=other tag log rc timer
        for arg in "$@"; do
            case "$arg" in
                */kayfabe-rm-ladder) kind=raw ;;
                */qemu-system-x86_64) kind=guest ;;
                --missing-page-fault) arm=$arg ;;
                --*) [ -n "$arm" ] || arm=$arg ;;
            esac
        done
        if [ "$kind" = other ]; then
            command timeout "$@"
            return $?
        fi
        [ "$kind" = raw ] || arm=guest
        timer=$(type -P timeout) || return 7
        tag="ci-arm-${BASHPID}-${RANDOM}"
        log="${BENCH_DIR:-/workspace/bench}/xid_${tag}.log"
        command bash "$KF_PAYLOAD_ROOT/scripts/bench/host_xid_watch.sh" \
            "$tag" -- "$timer" "$@"
        rc=$?
        python3 -I "$KF_PAYLOAD_ROOT/scripts/ci/vast_payload.py" \
            --grade-xid "$arm" "$log" "$rc"
    }
fi
