#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
set -euo pipefail
S4_HOME=${S4_HOME:-/root/s4}
S4_PROV=${S4_PROV:-/root/prov}
s4_watcher_pid=

s4_step() { bash "$S4_HOME/$1.sh"; }
s4_watch() {
    python3 -u /root/kayfabe/scripts/bench/native_xid_guard.py watch \
        "$S4_PROV/native_xid_baseline.json" $$ "$S4_PROV/native_xid_fatal.json" \
        >"$S4_PROV/native_xid_watch.log" 2>&1 &
    s4_watcher_pid=$!
    kill -0 "$s4_watcher_pid"
}
s4_exit() {
    local rc=$?
    trap - EXIT
    set +e
    if [ -n "$s4_watcher_pid" ]; then
        kill -TERM "$s4_watcher_pid" 2>/dev/null
        wait "$s4_watcher_pid" 2>/dev/null
    fi
    echo "S4_EXIT rc=$rc $(date -u -Is)"
    exit "$rc"
}
s4_controller() {
    source "$S4_HOME/config.sh"
    mkdir -p "$S4_PROV"
    trap s4_exit EXIT
    trap 'exit 143' TERM
    echo "S4_START source=$S4_SOURCE $(date -u -Is)"
    s4_step provision
    s4_step native
    s4_watch
    s4_step install
    s4_step bind
    s4_step desktop
    s4_step final
    echo "S4_COMPLETE source=$S4_SOURCE $(date -u -Is)"
}
if [ "${BASH_SOURCE[0]}" = "$0" ]; then s4_controller "$@"; fi
