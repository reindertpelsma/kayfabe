#!/bin/sh
# Shared curl-stdin Access transport. No credentials in argv or diagnostics.
kf_access_validate() {
    [ -n "${KF_ACCESS_ID:-}${KF_ACCESS_SECRET:-}" ] || return 0
    case ${KF_ACCESS_ID:-} in ''|*[!A-Za-z0-9_.-]*) return 1;; esac
    case ${KF_ACCESS_SECRET:-} in ''|*[!A-Za-z0-9_.-]*) return 1;; esac
    [ "${#KF_ACCESS_ID}" -le 256 ] && [ "${#KF_ACCESS_SECRET}" -ge 32 ] &&
        [ "${#KF_ACCESS_SECRET}" -le 256 ]
}
kf_access_headers() {
    [ -n "${KF_ACCESS_ID:-}" ] || return 0
    printf 'header = "CF-Access-Client-Id: %s"\n' "$KF_ACCESS_ID"
    printf 'header = "CF-Access-Client-Secret: %s"\n' "$KF_ACCESS_SECRET"
}
