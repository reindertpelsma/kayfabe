source /root/kayfabe/scripts/ci/vast_payload_hooks.sh
# A refused earlier arm may never submit another GPU operation.
if declare -F timeout >/dev/null; then
    eval "$(declare -f timeout | sed '1s/timeout/sw4_watched_timeout/')"
    timeout() {
        if [ -e "$KF_PAYLOAD_XID_FAILED" ]; then
            echo SW3_SKIPPED_AFTER_XID_STOP >&2
            return 7
        fi
        sw4_watched_timeout "$@"
    }
fi
