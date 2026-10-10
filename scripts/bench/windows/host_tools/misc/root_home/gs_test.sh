#!/bin/bash
# gs_test.sh <label> — bring up nvkvm-steamos and report on the gamescope session.
# Leaves the stack running. Never prunes volumes: the guest is 158 GB of install.
set -u
LABEL="${1:?need a label}"
BASE=/root/gs-test
OUT="$BASE/$LABEL"
mkdir -p "$OUT"
exec >"$OUT/run.log" 2>&1
say() { echo "[$(date +%H:%M:%S)] $*"; }

cd "$BASE/repo" || { say "FATAL: no repo at $BASE/repo"; exit 1; }
say "testing $(git log --oneline -1)  [$(git rev-parse --abbrev-ref HEAD)]"

say "BAR1 before:"; nvidia-smi -q | grep -A3 "BAR1 Memory Usage" | grep -E "Used|Free"

say "stopping any running stack"
docker compose down --remove-orphans >/dev/null 2>&1

say "build"
docker compose build >"$OUT/build.log" 2>&1 || { say "BUILD FAILED"; tail -30 "$OUT/build.log"; exit 1; }
say "build ok"

say "up (wayland: no NVKVM_WAYLAND_SOCKET override)"
docker compose up -d >"$OUT/up.log" 2>&1 || { say "UP FAILED"; tail -30 "$OUT/up.log"; exit 1; }
docker compose ps --format "  {{.Name}} {{.Status}}"

say "waiting for the guest to answer ssh on 15022 (max 900s)"
t0=$SECONDS; ok=0
while [ $((SECONDS-t0)) -lt 900 ]; do
    if ./steamos-ssh true >/dev/null 2>&1; then ok=1; break; fi
    sleep 10
done
say "guest ssh: $([ $ok = 1 ] && echo "up after $((SECONDS-t0))s" || echo "NEVER CAME UP")"

# SSH answering does NOT mean the session has started -- nvkvm-boot runs BEFORE
# display-manager.  Sampling here is what produced a bogus "inactive" last run.
# Wait for the session to settle, then sample.
if [ $ok = 1 ]; then
    say "waiting for gamescope-session to go active (max 300s)"
    t1=$SECONDS; ready=0
    while [ $((SECONDS-t1)) -lt 300 ]; do
        st=$(./steamos-ssh 'systemctl --user --machine=deck@ is-active gamescope-session.service 2>/dev/null' 2>/dev/null | tr -d "\r\n ")
        [ "$st" = active ] && { ready=1; break; }
        sleep 10
    done
    say "gamescope-session: $([ $ready = 1 ] && echo "ACTIVE after $((SECONDS-t1))s" || echo "NEVER WENT ACTIVE in 300s")"
fi

if [ $ok = 1 ]; then
    say "--- the readiness deadline actually in the guest image ---"
    ./steamos-ssh 'grep -n "read.*-t" /usr/lib/steamos/gamescope-session 2>/dev/null | head -5; \
        ls -la /usr/lib/steamos/gamescope-session.nvkvm-orig 2>/dev/null || echo "(no .nvkvm-orig backup)"' 2>&1

    say "--- gamescope session state ---"
    ./steamos-ssh 'systemctl --user --machine=deck@ is-active gamescope-session.service 2>/dev/null; \
        systemctl --user --machine=deck@ status gamescope-session.service --no-pager 2>&1 | head -12' 2>&1

    say "--- did anything reach the display? ---"
    ./steamos-ssh 'for c in /sys/class/drm/card*/card*-*/enabled; do echo "  $c = $(cat $c 2>/dev/null)"; done; \
        echo "  steam procs: $(pgrep -c steam 2>/dev/null || echo 0)"; \
        echo "  gamescope procs: $(pgrep -c gamescope 2>/dev/null || echo 0)"' 2>&1
fi

say "--- host broker: did it present frames? ---"
docker compose logs broker 2>&1 | tail -25

say "--- convergence: what did the gamescope fixup say? ---"
docker compose logs vmm 2>&1 | grep -i "gamescope\|readiness\|deadline" | tail -15
echo "(end of gamescope lines)"

say "BAR1 after:"; nvidia-smi -q | grep -A3 "BAR1 Memory Usage" | grep -E "Used|Free"
say "DONE"
