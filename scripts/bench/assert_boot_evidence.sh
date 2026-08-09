#!/usr/bin/env bash
# ★★★ THE GATE A `BOOTED` COMMIT MUST PASS: the boot's evidence is in the tree, is not
# empty, says what a boot says, and is **TRACKED BY GIT**.
#
#   usage: scripts/bench/assert_boot_evidence.sh [<tag>]
#
# With a tag it checks that boot specifically (and is the form to run before committing).
# With no tag it sweeps every file in `traces/guest_boots/` — which catches the *other* half:
# evidence that was copied in and then never committed.
#
# ## ★★★ Why this exists, and it is the THIRD sighting of one shape
#
# `[measured 2026-08-09]` three `BOOTED` commits — `08ef29b`, `18b75e8`, `a5ede26` — touched
# exactly ONE file each, the design doc. `s23`'s `_qemu.log` and the whole of `s24`/`s25`
# were never in the repository. The findings they carry are load-bearing (the wall is ONE
# channel; `NO-VASPACE-IN-NAMESPACE`; the `24/9/15` invariance that PROVES two instruments
# were observational) and nothing in the tree could re-verify any of them.
#
# Its two siblings, both from this project:
#
#   1. **The serial log is not where the driver's output is.** Six rung claims whose only
#      evidence was a session transcript. The serial log existed, was freshly timestamped,
#      was named after the boot — every signal said the evidence was there.
#   2. **`git commit -- <path>` does not add untracked files.** A `BOOTED` commit shipped
#      without its six logs and the commit SUCCEEDED.
#
# ⇒ ★ The pattern is not carelessness. It is that **the operation succeeds either way.**
# `git commit` exits 0 whether or not the evidence went with it; `boot_capture.sh` exits 0
# having stored the evidence on a rented box. Only an explicit assertion separates the two,
# and an assertion nobody runs is a paragraph. So this is a script with an exit status.
#
# ⊘ It deliberately does NOT check that the numbers are good. A red boot is evidence too.
# What it checks is that a claim beginning "BOOTED" can be checked by somebody else.
set -uo pipefail

REPO=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
BOOTS="$REPO/traces/guest_boots"
TAG=${1:-}
rc=0
fail() { printf '★ FAIL: %s\n' "$*"; rc=1; }

[ -d "$BOOTS" ] || { printf '★ FAIL: no %s\n' "$BOOTS"; exit 2; }

# ---- tracked-ness, over the WHOLE directory -----------------------------------------
# ⊘ `git ls-files --others` is the only question that matters here, and it is asked of the
# directory rather than of a tag: a file copied in by `boot_capture.sh` phase 6 and then
# left out of the commit is exactly the failure this gate exists for, and it has no tag.
untracked=$(cd "$REPO" && git ls-files --others --exclude-standard -- traces/guest_boots)
if [ -n "$untracked" ]; then
  fail "evidence in the tree but NOT TRACKED — \`git commit -- <path>\` will silently omit it:"
  printf '    %s\n' $untracked
fi

# ---- every committed file must actually contain something ---------------------------
# ★ An empty file's existence reads as capture; that is the bench-rebuild lesson stated one
# directory over. Checked over all of them, so an old zero-byte file cannot hide.
while IFS= read -r f; do
  [ -s "$REPO/$f" ] || fail "$f is EMPTY — its existence reads as a capture and is not one"
done < <(cd "$REPO" && git ls-files -- traces/guest_boots)

# ---- and, with a tag, that THIS boot's three files are all there and say so ----------
if [ -n "$TAG" ]; then
  q="run_${TAG}_qemu.log"; d="run_${TAG}_dmesg.log"; p="run_${TAG}_probe.log"
  for f in "$q" "$d" "$p"; do
    if [ ! -s "$BOOTS/$f" ]; then
      fail "$f is missing or empty in traces/guest_boots"
      continue
    fi
    if ! (cd "$REPO" && git ls-files --error-unmatch "traces/guest_boots/$f" >/dev/null 2>&1); then
      fail "$f is present but UNTRACKED — commit it before claiming BOOTED"
    fi
  done
  # ★ Content, not existence — the three strings that make each file the thing it claims to
  # be. ⊘ `RmInitAdapter` rather than `NVRM`, for `boot_capture.sh`'s measured reason: the
  # module-load banner is an `NVRM:` line, so an NVRM check passes on a capture in which the
  # adapter was never touched.
  grep -q 'nvkvm: doorbells:' "$BOOTS/$q" 2>/dev/null ||
    fail "$q carries no census line — QEMU never ran its exit notifier for this tag"
  grep -q 'RmInitAdapter\|rm_init_adapter' "$BOOTS/$d" 2>/dev/null ||
    fail "$d has no RmInitAdapter output — the adapter was never exercised"
  # ★★ THE REV STAMP, which is what makes the boot attributable at all. `boot_capture.sh`
  # writes it into the probe log from INSIDE the binary (never from BUILD_REV.txt, which
  # named a different revision once). A boot with no stamp cannot be cited against a commit.
  grep -q 'kayfabe-rev:[0-9a-f]' "$BOOTS/$p" 2>/dev/null ||
    fail "$p carries no \`kayfabe-rev:\` stamp — this boot cannot be attributed to a revision"
fi

if [ "$rc" -eq 0 ]; then
  n=$(cd "$REPO" && git ls-files -- traces/guest_boots | wc -l)
  printf 'ok — %s tracked, non-empty evidence files%s\n' "$n" \
    "${TAG:+, and run_${TAG}_{qemu,dmesg,probe}.log are complete and stamped}"
fi
exit "$rc"
