# shellcheck shell=sh
# Installed as /etc/profile.d/kf-ci-hold.sh by vast_onstart.sh.
# An interactive SSH login takes the box into MANUAL mode (POST /hold via kf-box-hold). The call
# runs detached with all output discarded, so it never blocks or disturbs the login, and any
# failure is ignored. It runs at most once per login (children inherit KF_CI_HOLD_DONE).
case $- in
*i*)
    if [ -n "${SSH_CONNECTION:-}" ] && [ -z "${KF_CI_HOLD_DONE:-}" ]; then
        KF_CI_HOLD_DONE=1
        export KF_CI_HOLD_DONE
        if [ -x "${KF_BOX_HOLD_BIN:-/usr/local/bin/kf-box-hold}" ]; then
            ("${KF_BOX_HOLD_BIN:-/usr/local/bin/kf-box-hold}" </dev/null >/dev/null 2>&1 &) \
                >/dev/null 2>&1 || true
        fi
    fi
    ;;
esac
:
