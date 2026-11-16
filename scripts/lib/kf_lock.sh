# Sourced helper: open the shared GPU lock on a file descriptor without O_CREAT on an existing file.
#
#   . "$(dirname "${BASH_SOURCE[0]}")/../lib/kf_lock.sh"
#   kf_lock_open "${KF_LOCK:-/tmp/kayfabe-fastguest.lock}" [FD=9]; flock 9
#
# Why: `exec 9>FILE` is open(O_CREAT|O_TRUNC). With fs.protected_regular=1/2 (Ubuntu default) the kernel refuses
# that for a file in a sticky world-writable directory (/tmp) that another user created, even at mode 0666, and
# the later `flock 9` then dies with "Bad file descriptor". flock(2) needs no write access, so the lock is taken
# on a read-only descriptor of the one shared file. One path for every user keeps the mutual exclusion between
# users; the first user creates it mode 0666 (umask 0) so the next can open it. Logic and tests: scripts/lib/kf_lock.py.
kf_lock_open() {
    local f=$1 fd=${2:-9}
    if [ ! -e "$f" ]; then (umask 0; : >"$f") 2>/dev/null || true; fi
    if [ -L "$f" ] || { [ -e "$f" ] && [ ! -f "$f" ]; }; then
        echo "kf_lock_open: $f is not a regular file; refusing it" >&2; return 1
    fi
    eval "exec $fd<\"\$f\"" || { echo "kf_lock_open: cannot open $f" >&2; return 1; }
}
