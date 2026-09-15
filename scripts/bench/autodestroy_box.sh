#!/usr/bin/env bash
# autodestroy_box.sh <instance-id> <seconds> [logfile]
#
# ★★★ THE MONEY SAFETY NET, AND WHY IT IS A DETACHED FILE.
#
# Every in-process net -- a bash trap, a Rust `Drop`, an agent's "I will destroy it when I
# finish" -- SHARES ITS FATE WITH THE PROCESS IT PROTECTS. A usage limit, a SIGKILL, an OOM,
# a dropped ssh or a closed session takes the net down with the swimmer, and the box bills on.
# So this is armed SEPARATELY, detached with setsid, and outlives whatever armed it.
#
# ⊘ IT DESTROYS EXACTLY ONE NAMED ID AND NEVER ENUMERATES. The vast account is SHARED with
# other Claude sessions running budgeted sweeps; a net that destroys "everything listed" is a
# worse failure than the one it prevents. See memory: teardown_is_scoped_to_ids_you_created.
#
# ⚠ `vastai destroy instance <id>` PROMPTS and prints `Aborted.` -- a bare call leaves the box
# running. `echo y |` is not decoration.
set -u
ID="${1:?instance id}"; SECS="${2:?seconds}"; LOG="${3:-/workspace/autodestroy-$ID.log}"
{
  echo "=== ARMED $(date -u +%FT%TZ) for instance $ID, firing in ${SECS}s ==="
  sleep "$SECS"
  echo "=== FIRING $(date -u +%FT%TZ) ==="
  if ! timeout 120 vastai show instances 2>/dev/null | grep -qE "^\s*[0-9]+\s+$ID\b"; then
    echo "instance $ID is already gone -- nothing to do"; echo "=== DONE (no-op) ==="; exit 0
  fi
  echo y | timeout 180 vastai destroy instance "$ID" 2>&1
  sleep 5
  echo "--- VERIFY (verbatim) ---"
  timeout 120 vastai show instances 2>&1
  echo "=== DONE $(date -u +%FT%TZ) ==="
} >>"$LOG" 2>&1
