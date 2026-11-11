#!/bin/bash
# DNS fallback for the Vast boxes and the GitHub runners (2026-11-11).
# MEASURED 2026-11-11 (GitHub ubuntu-26.04 runner): the default resolver timed out for archive.ubuntu.com,
# security.ubuntu.com and cloud-images.ubuntu.com while 1.1.1.1, 8.8.8.8 and 9.9.9.9 answered.
# If archive.ubuntu.com does not resolve (bounded, 5 s), point the resolver at 1.1.1.1 and 9.9.9.9
# (resolvectl on the default-route interface when systemd-resolved answers, else /etc/resolv.conf),
# re-check, and print exactly one line: KF_DNS_FALLBACK used=0 | used=1 ok=<0|1>.
# Never fails the caller (always exit 0), holds no secrets, and does nothing when resolution works.
# Test overrides (the box never sets them): KF_DNS_RESOLV_CONF, KF_DNS_PROBE_HOST.
probe_host=${KF_DNS_PROBE_HOST:-archive.ubuntu.com}
resolv_conf=${KF_DNS_RESOLV_CONF:-/etc/resolv.conf}

resolves() {
    if command -v timeout >/dev/null 2>&1; then
        timeout 5 getent hosts "$probe_host" >/dev/null 2>&1
    else
        getent hosts "$probe_host" >/dev/null 2>&1
    fi
}

if resolves; then
    echo "KF_DNS_FALLBACK used=0"
    exit 0
fi

applied=0
if command -v resolvectl >/dev/null 2>&1 && resolvectl status >/dev/null 2>&1; then
    iface=$(ip -o route show default 2>/dev/null | awk '{for (i = 1; i < NF; i++) if ($i == "dev") { print $(i + 1); exit }}')
    if [ -n "$iface" ] && resolvectl dns "$iface" 1.1.1.1 9.9.9.9 >/dev/null 2>&1; then
        resolvectl flush-caches >/dev/null 2>&1
        applied=1
    fi
fi
if [ "$applied" = 0 ]; then
    # Keep the first original once; rewriting the same two lines is idempotent.
    if [ ! -e "$resolv_conf.kf-orig" ] && [ -e "$resolv_conf" ]; then
        cp -p "$resolv_conf" "$resolv_conf.kf-orig" 2>/dev/null
    fi
    printf 'nameserver 1.1.1.1\nnameserver 9.9.9.9\n' >"$resolv_conf" 2>/dev/null
    chmod 644 "$resolv_conf" 2>/dev/null
fi

if resolves; then
    echo "KF_DNS_FALLBACK used=1 ok=1"
else
    echo "KF_DNS_FALLBACK used=1 ok=0"
fi
exit 0
