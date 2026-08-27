#!/usr/bin/env bash
# bench_wait.sh — RUNS ON THE BENCH HOST. Block until the Mode-2 guest is
# reachable over ssh, then print the forwarding-path markers.
#
# Run this as a SEPARATE command from bench_boot.sh.  Launching and polling in
# one ssh invocation has repeatedly gone wrong here.
#
# ★ The guest takes ~20-25 s to reach a login prompt and the serial log is
#   written lazily, so an early peek at /tmp/m0_serial.log shows it "stopping"
#   at 4-8 s mid kernel-init.  That is NOT a hang.  Wait the full timeout
#   before concluding anything.
#
# ★ If this reports GUEST DID NOT COME UP but the serial log ends at a
#   `nvkvm-guest login:` prompt, the guest booted fine and you have an ssh AUTH
#   failure, not a boot failure.  The harness scripts all run a BARE
#   `ssh -p 2223 ubuntu@localhost` with no -i, so root on the bench host must
#   have the guest key configured — see docs/BENCH_REBUILD_NOTES.md.
set -u
PORT="${SSH_PORT:-2223}"
TRIES="${TRIES:-120}"

for i in $(seq "$TRIES"); do
    if timeout 8 ssh -p "$PORT" -o BatchMode=yes -o ConnectTimeout=5 \
            ubuntu@localhost true 2>/dev/null; then
        echo "GUEST UP after ~$((i * 2))s"
        grep -aE "M5\.1|M6\.1|MEMTEST:|M5\.3" /tmp/m0_qemu.log || \
            echo "WARNING: no forwarding markers in /tmp/m0_qemu.log"
        exit 0
    fi
    sleep 2
done

echo "GUEST DID NOT COME UP after ~$((TRIES * 2))s"
echo "--- last 600 bytes of serial (does it end at a login prompt? then it is an SSH AUTH failure) ---"
tail -c 600 /tmp/m0_serial.log
exit 1
