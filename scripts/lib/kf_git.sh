# Sourced helper: read-only git on a checkout this user does not own (rootless run on a root-owned tree).
#
#   . "$(dirname "${BASH_SOURCE[0]}")/../lib/kf_git.sh"
#   rev=$(kf_git "$KF3_REPO" rev-parse --short=8 HEAD)
#
# git refuses a checkout owned by another uid ("detected dubious ownership"); run_fast_guest.sh then saw
# `|| echo unknown` as the revision, found no kf3 binary and failed the guest arm with boot_rc=2. This trusts
# exactly one directory, for exactly one invocation: `git -c safe.directory=<that absolute path> -C <that path>`.
# Never `*`, never a parent, never written to any config file. Use it for read-only queries (rev-parse, status,
# log) only. The same rule in Python: scripts/lib/kf_git.py.
kf_git() {
    local d=$1 abs
    shift
    abs=$(cd "$d" 2>/dev/null && pwd -P) || { echo "kf_git: no such directory: $d" >&2; return 2; }
    case $abs in
        /*[*?[]* | *$'\n'*) echo "kf_git: refusing a pattern or multi-line path: $abs" >&2; return 2 ;;
        /) echo "kf_git: refusing /" >&2; return 2 ;;
    esac
    git -c "safe.directory=$abs" -C "$abs" "$@"
}
