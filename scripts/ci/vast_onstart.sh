#!/bin/bash
# Entry point of every Vast CI box: the Worker's tiny onstart clones the repo, checks out the
# exact SHA and execs this script. Everything per-run arrives as KF_* environment variables
# (contract: scripts/ci/README.md). This script
#   installs curl/python3/git when missing, validates the contract (structured failure post on
#   violation), keeps the environment in a root-only file, installs kf-box-kill and the SSH
#   hold hook, starts the reporter, runs the payload under the soft deadline with the run
#   capability removed from its environment, and hands the result to the reporter.
# KF_CI_* variables below are overrides for tests; the Worker never sets them.
set -u
umask 077
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo=$(cd "$here/../.." && pwd)
workspace=${KF_CI_WORKSPACE:-/workspace}
run_dir=${KF_CI_RUN_DIR:-/run/kf-ci}
bin_dir=${KF_CI_BIN_DIR:-/usr/local/bin}
profile_dir=${KF_CI_PROFILE_DIR:-/etc/profile.d}
payload=scripts/ci/vast_payload.sh
log=$workspace/kf-ci-output.log
result=$workspace/kf-result.json
read -r -a reporter_args <<<"${KF_CI_REPORTER_ARGS:-}"
report() { python3 "$here/vast_report.py" "$@" ${reporter_args[@]+"${reporter_args[@]}"}; }

missing_tools() {
    local tool
    for tool in curl python3 git; do
        command -v "$tool" >/dev/null 2>&1 || echo "$tool"
    done
}
if [ -n "$(missing_tools)" ]; then
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq && apt-get install -y -qq curl python3 git ca-certificates
fi
if [ -n "$(missing_tools)" ]; then
    echo "vast_onstart: cannot continue, missing and not installable: $(missing_tools)" >&2
    exit 2
fi

install -d -m 700 "$workspace" || exit 2
report check-env --write "$run_dir/env" || exit 2
set -a
# shellcheck disable=SC1091
. "$run_dir/env"
set +a

mkdir -p "$bin_dir" "$profile_dir"
install -m 700 "$here/kf-box-kill" "$bin_dir/kf-box-kill"
install -m 700 "$here/kf-box-hold" "$bin_dir/kf-box-hold"
install -m 700 "$here/kf-box-blob-url" "$bin_dir/kf-box-blob-url"
install -m 644 "$here/kf-ci-hold.sh" "$profile_dir/kf-ci-hold.sh"

rm -f "$result" "$run_dir/done" "$run_dir/done.tmp"
: >"$log"
# An internal pipe carries the first accepted report event to dependency consumers.
# Keep ordinary/offline payloads runnable; only private blob setup waits on this event.
ready_pipe=$run_dir/ready.$$
mkfifo -m 600 "$ready_pipe" || exit 2
exec {ready_fd}<>"$ready_pipe" || exit 2
rm -f "$ready_pipe"
report daemon --log "$log" --done "$run_dir/done" --result "$result" --ready-fd "$ready_fd" &
reporter=$!

rc=0
timed_out=0
reason=
now=$(date +%s)
remaining=$((KF_SOFT_DEADLINE - now))
if [ ! -f "$repo/$payload" ]; then
    echo "vast_onstart: $payload is absent from this checkout; nothing to run" >>"$log"
    rc=127
    reason=payload_missing
elif [ "$remaining" -le 0 ]; then
    echo "vast_onstart: the soft deadline has already passed; the payload is not started" >>"$log"
    rc=124
    timed_out=1
else
    # The payload gets no run capability: broker URL, run id, token and runner nonce are removed.
    (
        cd "$repo" || exit 125
        exec timeout --signal=TERM --kill-after=30s "${remaining}s" \
            env -u KF_RUN_TOKEN -u KF_BROKER_URL -u KF_RUN_ID -u KF_RUNNER_NONCE \
            "KF_SOFT_DEADLINE_MS=$((KF_SOFT_DEADLINE * 1000))" \
            "KF_BROKER_READY_FD=$ready_fd" \
            bash "$payload" "$KF_SHA" "$KF_NONCE"
    ) >>"$log" 2>&1 </dev/null
    rc=$?
    # 124/137 from timeout(1) count only if the deadline really passed (a payload may exit so).
    if [ "$rc" -eq 124 ] || [ "$rc" -eq 137 ]; then
        [ "$(date +%s)" -ge $((KF_SOFT_DEADLINE - 2)) ] && timed_out=1
    fi
fi

exec {ready_fd}>&-

{
    printf 'rc=%d\ntimeout=%d\n' "$rc" "$timed_out"
    [ -z "$reason" ] || printf 'reason=%s\n' "$reason"
} >"$run_dir/done.tmp"
mv "$run_dir/done.tmp" "$run_dir/done"

# The reporter delivers the remaining log and the result, then exits; bound the wait anyway.
waited=0
while kill -0 "$reporter" 2>/dev/null && [ "$waited" -lt 4500 ]; do
    sleep 0.2
    waited=$((waited + 1))
done
kill "$reporter" 2>/dev/null
wait "$reporter"
reporter_rc=$?
if [ "$rc" -ne 0 ]; then
    exit "$rc"
fi
exit "$reporter_rc"
