#!/usr/bin/env bash
# Controller must obtain owner authorization first. No VM; owned unprivileged RM client only.
set -euo pipefail
if [[ $# != 6 || $1 != --owner-authorized ]]; then
    echo 'usage: --owner-authorized BINARY N sparse|normal FRESH_DIR DESKTOP_WATCHDOG' >&2
    exit 2
fi
shift
exec flock -o -n /tmp/kayfabe-fastguest.lock \
    timeout --signal=TERM --kill-after=10s 300s "$1" --run-native "$2" "$3" "$4" "$5"
