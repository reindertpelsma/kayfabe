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
  # ⊘⊘⊘ **THE PRESENCE CHECK IS DELETED, AND IT COST TWO BOXES A FULL NIGHT.**
  #
  # `[measured 2026-09-19]` it read
  #   `vastai show instances | grep -qE "^\s*[0-9]+\s+$ID\b"`
  # and **can never match**: `vastai` emits ANSI colour escapes, so every row begins
  # `\e[48;5;240m\e[97m  1  51402274 …` — an escape byte, not whitespace-then-digit. ⇒ the
  # check concluded *"already gone"* for a RUNNING instance, logged `DONE (no-op)`, and skipped
  # the destroy. Both nets fired on time, both reported success, and both boxes were still
  # billing when the owner looked.
  #
  # ⚠ It fails in the ONLY direction that matters. A guard that fails closed leaks an error; a
  # guard that fails OPEN leaks money and looks healthy doing it — the log reads `=== DONE ===`
  # either way. Same family as `a_check_that_reports_is_not_a_check_that_gates`.
  #
  # ★ `nvkvm-pv/scripts/sweep_autodestroy.sh` already had the right design and said so:
  # *"At the deadline: destroy every id in the registry, unconditionally. There is deliberately
  # NO disarm. A destroy of an already-destroyed instance is a no-op."* ⇒ the check bought
  # nothing and added the one failure mode a money net must not have. Destroy unconditionally;
  # the VERIFY below is what says whether it worked.
  # ⊘⊘⊘ **`yes |` AND `-y`, AND NEITHER IS DECORATION.** `[owner, 2026-09-19]` *"vastai destroy
  # asks about y confirm, read docs how to do auto destroy properly ... its tricky for CLI
  # command"*. `vastai destroy instance <id>` prompts `[y/N]`; with no tty a bare call can
  # print `Aborted.` and still exit 0, so the net reports success and the box bills on.
  # ⚠ This script carried only `echo y |`. `nvkvm-pv/scripts/sweep_autodestroy.sh:105` has had
  # both for months, and its comment says why: *"With no tty it reads ... success. Hence
  # `yes |` AND `-y`."* The CLI confirms it: `-y, --yes  Skip confirmation prompt`.
  yes | timeout 180 vastai destroy instance "$ID" -y 2>&1
  sleep 5
  echo "--- VERIFY (verbatim) ---"
  timeout 120 vastai show instances 2>&1
  # ★★★★★ **AND NOW ASSERT IT, WITH THE ESCAPES STRIPPED FIRST.**
  #
  # ⊘ The verbatim dump above is for a reader; it cannot fail. `[measured 2026-09-19]` a net
  # that only printed was a net that reported `=== DONE ===` over two running boxes. ⇒ strip
  # ANSI (`sed 's/\x1b\[[0-9;]*m//g'`) — the exact thing the deleted presence check did not do
  # — and then say, in one machine-readable line, whether the id survived.
  #
  # ⚠ This assertion may only ever make the log LOUDER. It must never gate the destroy above:
  # that is how the last one turned into a skip.
  if timeout 120 vastai show instances 2>/dev/null \
       | sed 's/\x1b\[[0-9;]*m//g' \
       | grep -qE "[[:space:]]$ID[[:space:]]"; then
    echo "AUTODESTROY_RESULT=STILL_RUNNING id=$ID ⊘⊘⊘ THE BOX IS STILL BILLING"
  else
    echo "AUTODESTROY_RESULT=GONE id=$ID"
  fi
  echo "=== DONE $(date -u +%FT%TZ) ==="
} >>"$LOG" 2>&1
