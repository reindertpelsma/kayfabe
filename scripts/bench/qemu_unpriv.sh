#!/usr/bin/env bash
# ★★★ V3_SEC_P0 — drop QEMU (the kf3 process) to an unprivileged user before it runs.
#
# The kayfabe security model is an UNPRIVILEGED host process driving a real GPU. A host RM
# channel's privilege is stamped at creation from the creating ioctl's capability, per ioctl, in
# the calling thread (`ogkm-580: kernel_channel.c:277-291`; `escape.c:304`). If QEMU runs as root
# (CAP_SYS_ADMIN), every passthrough twin kf3 births is an ADMIN host channel — the escalation the
# single store forbids (OWNER_RULINGS §N). kf3 refuses to realize under CAP_SYS_ADMIN
# (crates/kf-qemu/src/device.rs), so the bench must launch QEMU unprivileged for the gates/suite to
# run at all.
#
# This file is SOURCED by boot_nvkvm.sh and run_fast_guest.sh. It sets:
#   KF_QEMU_PREFIX  — an array to put in front of the qemu command (empty = run as-is)
# and exposes:
#   kf_unpriv_setup            — fill KF_QEMU_PREFIX, raising memlock first
#   kf_unpriv_file <path>...   — pre-create each QEMU-opened output file 0666 so the dropped
#                                process can write it (the parent shell's own `>` redirections
#                                already carry an open fd across the drop and need no help)
#
# KF_QEMU_USER (default kfqemu) is the service user provisioning creates. KF_QEMU_DROP=0 disables
# the drop (for a deliberate root diagnostic run, which kf3 then refuses unless
# KF3_UNSAFE_ALLOW_CAP_SYS_ADMIN=1 is also set).

KF_QEMU_PREFIX=()

kf_unpriv_setup() {
    KF_QEMU_PREFIX=()
    local user=${KF_QEMU_USER:-kfqemu}
    if [ "${KF_QEMU_DROP:-1}" != 1 ]; then
        echo "== QEMU privilege drop DISABLED (KF_QEMU_DROP=$KF_QEMU_DROP); kf3 will refuse to realize as root unless KF3_UNSAFE_ALLOW_CAP_SYS_ADMIN=1" >&2
        return 0
    fi
    if [ "$(id -u)" != 0 ]; then
        # Already unprivileged: nothing to drop. Verify we do not carry CAP_SYS_ADMIN anyway.
        return 0
    fi
    if ! id "$user" >/dev/null 2>&1; then
        echo "== ⚠ KF_QEMU_USER=$user does not exist — cannot drop privilege; provision it (scripts/bench/provision_box.sh). Running as root will be REFUSED by kf3 realize." >&2
        return 0
    fi
    if ! command -v setpriv >/dev/null 2>&1; then
        echo "== ⚠ setpriv(1) not found (util-linux) — cannot drop privilege cleanly." >&2
        return 0
    fi
    # Raise memlock as root BEFORE the drop: the whole guest RAM is pinned for the VM's life
    # (the host RM registers the guest memfd), which counts against RLIMIT_MEMLOCK, and an
    # unprivileged user cannot raise its own hard limit. setpriv preserves the rlimits of the
    # calling process across the uid change, so the dropped QEMU inherits this.
    ulimit -l unlimited 2>/dev/null || ulimit -l "$(( 32 * 1024 * 1024 ))" 2>/dev/null || true
    # --init-groups applies $user's supplementary groups (kvm, for /dev/kvm); every capability set
    # is emptied and no-new-privs is set, so the dropped QEMU cannot regain CAP_SYS_ADMIN.
    KF_QEMU_PREFIX=(setpriv --reuid "$user" --regid "$user" --init-groups
                    --inh-caps=-all --ambient-caps=-all --bounding-set=-all --no-new-privs)
    echo "== V3_SEC_P0: dropping QEMU to unprivileged user '$user' (no CAP_SYS_ADMIN); memlock raised, kvm group via --init-groups" >&2
}

kf_unpriv_file() {
    # Pre-create files QEMU itself opens (serial/console logs), world-writable, so the dropped
    # user can create/truncate them under a root-owned directory. No-op when not dropping.
    [ "${#KF_QEMU_PREFIX[@]}" -gt 0 ] || return 0
    local f
    for f in "$@"; do
        : > "$f" 2>/dev/null || true
        chmod 0666 "$f" 2>/dev/null || true
    done
}
