#!/usr/bin/env bash
# ★★★ THE ORPHAN GATE — every public verb with NO caller outside its own crate.
#
#   usage: scripts/orphan_gate.sh [<path-glob> ...]      # default: every crate
#          ORPHAN_GATE_LIMIT=20 scripts/orphan_gate.sh crates/kayfabe-isolate
#
# ## ⊘⊘ WHY THIS IS NOT A GREP, AND THE MEASUREMENT THAT SETTLES IT
#
# `[measured 2026-08-11]` a name-search returns **the same answer for a working verb and an
# orphaned one**: `MapGuestRam` greps as "zero callers" and runs **8× per boot**. Text search
# was the instrument that failed every time it was load-bearing this campaign — seven times.
#
# ⇒ **Enumeration by text is fine; the VERDICT is the compiler's.** For each candidate the gate
# rewrites `pub fn NAME` to `pub(crate) fn NAME`, runs `cargo check --workspace`, and restores.
# **If it still compiles, nothing outside the crate calls it.** `MapGuestRam` cannot pass that:
# removing its visibility does not compile.
#
# ## ⊘ `--all-targets` is DELIBERATELY OMITTED
#
# The question is *"no caller outside its own crate, excluding tests and harnesses"*. Integration
# tests are external crates, so `--all-targets` would count a test as a consumer and report a
# verb that only its own harness exercises as **wired** — which is the precise shape this gate
# exists to find (`ExportBacking`: proven in its harness, unreachable from the forwarding path).
#
# ## ⚠ WHAT IT CANNOT SEE, stated rather than implied
#
# **Trait methods.** A trait method inherits the trait's visibility and cannot be made
# `pub(crate)` individually, so it is skipped. A trait-method orphan is invisible here.
# ⊘ Also invisible: a verb reachable only from a `pub fn` that is itself an orphan — the gate
# reports the *outermost* orphan, and removing it is what exposes the next.
#
# ## ⊘ IT DOES NOT FAIL THE BUILD, AND THAT IS A DECISION
#
# Exit 0 whether or not it finds orphans. The first list over a tree this size is large, it needs
# human triage, and **a gate that goes red on day one gets disabled on day two.** What it
# enforces is a question for after the first triage, not before it.
# ## ⊘⊘⊘ THE MTIME TRAP — measured by this gate's OWN baseline check, on its first run
#
# Restoring a mutated file with `cp`/`mv` hands it the BACKUP's mtime, which is **older** than
# the fingerprint cargo recorded for the mutated build. `[measured 2026-08-11]` cargo then
# believes the crate is up to date and **serves the MUTATED compilation** — the tree reads
# `pub fn` on disk and fails to build with *"method `apply_deferring` is private"*. Every verdict
# after the first mutation would have been adjudicated against a stale build, and the gate would
# have reported confident nonsense.
# ⇒ **Every restore is followed by `touch`.** ★ And the thing that caught it was the baseline
# check below, written for an unrelated reason — a check earns its keep by failing for a reason
# its author did not have in mind.
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO" || exit 2
LIMIT=${ORPHAN_GATE_LIMIT:-0}

# ★★ RESTORE ON ANY EXIT. This gate MUTATES source to ask the compiler a question; a kill
# between the `sed` and the `mv` would otherwise leave a file edited and the next reader
# debugging a `pub(crate)` nobody wrote. ⊘ The trap is the difference between an instrument
# that is safe to interrupt and one that is not.
restore_all() {
  find . -name '*.orphan_gate_bak' -not -path './target/*' 2>/dev/null | while read -r b; do
    mv -f "$b" "${b%.orphan_gate_bak}"
    # ⊘ NOT optional — see THE MTIME TRAP in the header.
    touch "${b%.orphan_gate_bak}"
  done
}
trap restore_all EXIT INT TERM
declare -a ROOTS=("$@")
[ ${#ROOTS[@]} -eq 0 ] && ROOTS=(crates)

# ---- 0. the baseline must be green, or every verdict below is meaningless -----------------
# ★ A tree that does not compile makes "it still compiles" vacuously false for every candidate,
# and the gate would report EVERY verb as wired — a silent all-clear. Checked, not assumed.
echo "== baseline: cargo check --workspace"
if ! cargo check --workspace --quiet >/dev/null 2>&1; then
  echo "★ FAIL: the tree does not compile before any mutation. Every verdict would be garbage."
  exit 3
fi
echo "   baseline OK"

# ---- 1. ENUMERATE candidates (text is fine here — the verdict is not text's) ---------------
# `pub fn` at an indentation that puts it inside an `impl`/module, skipping:
#   - `pub(` — already scoped
#   - files under tests/ or benches/
#   - anything inside a `trait` block (visibility is the trait's; see the header)
mapfile -t CANDS < <(
  for root in "${ROOTS[@]}"; do
    find "$root" -name '*.rs' -not -path '*/tests/*' -not -path '*/benches/*' 2>/dev/null
  done | sort -u | while read -r f; do
    awk -v F="$f" '
      /^[[:space:]]*(pub )?trait [A-Za-z]/ { intrait=1; depth=0 }
      intrait { depth += gsub(/{/,"{"); depth -= gsub(/}/,"}"); if (depth<=0 && NR>1) intrait=0; next }
      /^[[:space:]]*pub fn [a-z_][A-Za-z0-9_]*/ {
        line=$0; sub(/^[[:space:]]*pub fn /,"",line); sub(/[(<].*$/,"",line);
        print F "\t" NR "\t" line
      }
    ' "$f"
  done
)

TOTAL=${#CANDS[@]}
echo "== candidates enumerated: $TOTAL  (inherent \`pub fn\` only; trait methods are out of scope)"
[ "$LIMIT" -gt 0 ] && echo "   ORPHAN_GATE_LIMIT=$LIMIT — checking the first $LIMIT"

# ---- 2. ADJUDICATE each one with the compiler ----------------------------------------------
orphans=0
checked=0
declare -a FOUND=()
for row in "${CANDS[@]}"; do
  [ "$LIMIT" -gt 0 ] && [ "$checked" -ge "$LIMIT" ] && break
  f=${row%%$'\t'*}; rest=${row#*$'\t'}; ln=${rest%%$'\t'*}; name=${rest#*$'\t'}
  cp "$f" "$f.orphan_gate_bak" || continue
  # ⊘ Line-addressed, so a name that appears twice in one file cannot mutate the wrong one.
  sed -i "${ln}s/pub fn /pub(crate) fn /" "$f"
  if cargo check --workspace --quiet >/dev/null 2>&1; then
    orphans=$(( orphans + 1 ))
    FOUND+=("$f:$ln  $name")
    echo "   ORPHAN  $f:$ln  $name"
  fi
  mv -f "$f.orphan_gate_bak" "$f"
  # ⊘ NOT optional — see THE MTIME TRAP in the header. Without this, cargo serves the MUTATED
  # build to the NEXT candidate and every verdict after the first is garbage.
  touch "$f"
  checked=$(( checked + 1 ))
done

# ---- 3. REPORT. No failure, by design (see the header) -------------------------------------
echo
echo "== orphan gate: $orphans of $checked checked have no caller outside their own crate"
if [ "$orphans" -gt 0 ]; then
  printf '   %s\n' "${FOUND[@]}"
  echo
  echo "⊘ This is a REPORT, not a verdict on any one of them: a verb may be a deliberate seam"
  echo "  awaiting its consumer, or a capability proven and never wired. Only a human knows"
  echo "  which, and that triage is what decides what this gate eventually enforces."
fi
# ⊘ Always 0 — see the header for why a day-one red gate is a gate nobody runs on day two.
exit 0
