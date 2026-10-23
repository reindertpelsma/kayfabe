#!/usr/bin/env bash
# Narrow instrumentation around existing raw-client / guest timeout calls.
# Loaded only by the payload's child shells; no suite/build implementation lives here.
if [ -n "${KF_PAYLOAD_ROOT:-}" ]; then
    apt-get() {
        if [ -n "${KF_APT_CACHE_ROOT:-}" ]; then
            command python3 -I "$KF_PAYLOAD_ROOT/scripts/ci/vast_setup.py" apt "$@"
        else
            command apt-get -o Acquire::Queue-Mode=host \
                -o Acquire::http::Pipeline-Depth=10 -o Acquire::https::Pipeline-Depth=10 "$@"
        fi
    }
    timeout() {
        local arg arm='' kind=other tag log rc timer fast=0 seen=0 qlog caller_pid
        local -a timer_args=() guest_args=()
        for arg in "$@"; do
            case "$arg" in
                */kayfabe-rm-ladder) kind=raw ;;
                */qemu-system-x86_64) kind=guest ;;
                --missing-page-fault|--defer-liveness)
                    [ "$kind" != raw ] || arm=$arg ;;
                --*)
                    [ "$kind" != raw ] || [ -n "$arm" ] || arm=$arg ;;
            esac
            [ "$arg" != kf-fastguest ] || fast=1
            if [[ "$arg" = */qemu-system-x86_64 ]]; then seen=1; fi
            if [ "$seen" = 1 ]; then guest_args+=("$arg"); else timer_args+=("$arg"); fi
        done
        if [ "$kind" = other ]; then
            command timeout "$@"
            return $?
        fi
        [ "$kind" = raw ] || arm=guest
        timer=$(type -P timeout) || return 7
        if [ "$kind" = guest ] && [ "$fast" = 1 ]; then
            caller_pid=$BASHPID
            qlog=$(readlink "/proc/$caller_pid/fd/1") || return 7
            command "$timer" "${timer_args[@]}" python3 -I \
                "$KF_PAYLOAD_ROOT/scripts/ci/vast_payload.py" \
                --guest-exec "$qlog" "${guest_args[@]}"
            return $?
        fi
        tag="ci-arm-${BASHPID}-${RANDOM}"
        log="${BENCH_DIR:-/workspace/bench}/xid_${tag}.log"
        command bash "$KF_PAYLOAD_ROOT/scripts/bench/host_xid_watch.sh" \
            "$tag" -- "$timer" "$@"
        rc=$?
        python3 -I "$KF_PAYLOAD_ROOT/scripts/ci/vast_payload.py" \
            --grade-xid "$arm" "$log" "$rc"
    }
fi
