# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# kfroot.sh -- sourced by the CI box scripts (windows_gate.sh, windows_apps.sh, win_vm.sh). Glue only: the logic is
# scripts/ci/kfroot.py (CLAUDE.md, "Shell is glue, logic is Python").
# STATUS: LIVE as code, 2026-11-15; NOT RUN ON A BOX (docs/design/V3_KFROOT_ROOTLESS.md).
#
#   kfroot CMD [ARGS...]   the ONE way a CI script runs a command as root. Every root step is a greppable `kfroot <cmd>` line
#                          and carries a comment that says why it needs root. As root already (the legacy box harness) the
#                          command runs directly. In a NoNewPrivs tree (the dropped kayfabe tree) sudo cannot elevate: the
#                          step is refused (rc 126) and the caller reports which step needed root.
#   kf_dmesg               the kernel log. dmesg needs CAP_SYSLOG while kernel.dmesg_restrict=1; the box bootstrap sets it
#                          to 0, otherwise this goes through kfroot.
#   kf_rootless_assert LABEL EVDIR [ARGS...]
#                          kfroot.py assert-tree in mode ${KF_ROOTLESS:-record} (require | record | off): prints the
#                          KFROOT_ASSERT lines, writes EVDIR/rootless_LABEL.json, returns 1 when mode=require and violated.
# (KFROOT_EUID, KFROOT_SELF_STATUS and KFROOT_DMESG_RESTRICT are test knobs: a fake uid and fake /proc files)
kfroot() {
    if [ "${KFROOT_EUID:-$(id -u)}" = 0 ]; then
        "$@"
        return
    fi
    if grep -q '^NoNewPrivs:[[:space:]]*1' "${KFROOT_SELF_STATUS:-/proc/self/status}" 2> /dev/null; then
        echo "KFROOT_REFUSED NoNewPrivs=1: 'kfroot $*' cannot run inside the dropped kayfabe tree (do it before the drop)" >&2
        return 126
    fi
    command kfroot "$@"
}
kf_dmesg() {
    if [ "${KFROOT_EUID:-$(id -u)}" = 0 ] || [ "$(cat "${KFROOT_DMESG_RESTRICT:-/proc/sys/kernel/dmesg_restrict}" 2> /dev/null || echo 1)" = 0 ]; then
        dmesg
    else
        # root: kernel.dmesg_restrict=1 needs CAP_SYSLOG to read the log
        kfroot dmesg
    fi
}
_kf_lib=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
KF_KFROOT_PY=${KF_KFROOT_PY:-$_kf_lib/../ci/kfroot.py}
kf_rootless_assert() { # LABEL EVDIR [kfroot.py assert-tree args...]
    local label=$1 ev=$2 mode=${KF_ROOTLESS:-record} rc=0
    shift 2
    case "$mode" in
        off) echo "KFROOT_ASSERT mode=off ok=skipped label=$label"; return 0 ;;
        require | record) ;;
        *) echo "KFROOT_ASSERT mode=$mode ok=0 label=$label (KF_ROOTLESS must be require, record or off)"; return 1 ;;
    esac
    python3 -I "$KF_KFROOT_PY" assert-tree --mode "$mode" --out "$ev/rootless_$label.json" "$@" || rc=$?
    if [ "$rc" != 0 ] && [ "$mode" = record ]; then
        echo "KFROOT_ASSERT mode=record ok=0 label=$label error=rc$rc"
        rc=0
    fi
    return "$rc"
}

# --- for a script that has `say`, an evidence directory $EV and wants the assertion in its verdict (windows_gate.sh,
# --- windows_apps.sh). ROOTLESS_STATE is ok | violated (sticky) | off | unchecked.
ROOTLESS_STATE=unchecked
ROOTLESS_FINAL_RC=0
# rootless_check LABEL [kfroot.py assert-tree args]: the KFROOT_ASSERT lines go to the log. Returns non-zero only when
# KF_ROOTLESS=require and the assertion failed.
rootless_check() {
    local label=$1 out rc=0 l
    shift
    out=$(kf_rootless_assert "$label" "$EV" "$@") || rc=$?
    while IFS= read -r l; do [ -z "$l" ] || say "$l"; done <<< "$out"
    if [ "${KF_ROOTLESS:-record}" = off ]; then
        [ "$ROOTLESS_STATE" != unchecked ] || ROOTLESS_STATE=off
    elif [ "$rc" != 0 ] || ! grep -q ' ok=1 ' <<< "$out"; then
        ROOTLESS_STATE=violated
    elif [ "$ROOTLESS_STATE" != violated ]; then
        ROOTLESS_STATE=ok
    fi
    return "$rc"
}
# rootless_final: the sudo log of the whole run window (complete once the guest is torn down): no entry other than the kfroot
# wrapper. KF_SUDO_LOG_MARK is the log offset `kfroot.py run` recorded before the drop. Sets ROOTLESS_FINAL_RC.
rootless_final() {
    ROOTLESS_FINAL_RC=0
    if [ -n "${KF_SUDO_LOG_MARK:-}" ]; then
        rootless_check final --sudo-since "$KF_SUDO_LOG_MARK" || ROOTLESS_FINAL_RC=1
    elif [ "${KF_ROOTLESS:-record}" = require ]; then
        say "KFROOT_ASSERT mode=require ok=0 label=final why=no-sudo-log-mark (start the run with kfroot.py run)"
        ROOTLESS_STATE=violated
        ROOTLESS_FINAL_RC=1
    else
        say "KFROOT_ASSERT mode=${KF_ROOTLESS:-record} ok=unchecked label=final why=no-sudo-log-mark"
    fi
}
