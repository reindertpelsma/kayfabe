#!/usr/bin/env bash
# SECOND question, asked of the SAME compilation the orphan gate already runs.
#   `pub fn` -> `pub(crate) fn`; the gate calls it an ORPHAN if the workspace still compiles.
#   rustc `dead_code` then fires IFF no caller is left in the crate at all.
# ⚠ MEASURED: rustc says "function", "method", "associated function", and COLLAPSES
#   several into "associated items `a` and `b` are never used". Matching only "function"
#   silently classified EVERY inherent method as live. Match on the NAME, not the noun.
set -uo pipefail
export PATH=$HOME/.cargo/bin:$PATH
REPO=$1; LIST=$2
cd "$REPO" || exit 2
restore_all() {
  find . -name "*.tri_bak" -not -path "./target/*" 2>/dev/null | while read -r b; do
    mv -f "$b" "${b%.tri_bak}"; touch "${b%.tri_bak}"
  done
}
trap "restore_all; exit 130" INT TERM
trap restore_all EXIT
named_dead() { # $1 = compiler output, $2 = symbol name
  printf "%s" "$1" | grep "never used" | grep -qF "\`$2\`"
}
echo "PROBE_START $(date -Is) rev=$(git rev-parse --short HEAD)"
base=$(cargo check --workspace 2>&1) || { echo "FAIL baseline plain"; exit 3; }
cargo check --workspace --all-targets --quiet >/dev/null 2>&1 || { echo "FAIL baseline all-targets"; exit 3; }
echo "baseline OK; pre-existing never-used lines: $(printf "%s" "$base" | grep -c "never used")"
printf "%s" "$base" | grep "never used" | sed "s/^/BASELINE_WARN /"
while read -r f ln name; do
  [ -z "${name:-}" ] && continue
  cp "$f" "$f.tri_bak" || continue
  sed -i "${ln}s/pub fn /pub(crate) fn /" "$f"
  out=$(cargo check --workspace 2>&1); rc=$?
  if [ $rc -ne 0 ]; then
    verdict=NOT_ORPHAN
  elif ! named_dead "$out" "$name"; then
    verdict=INTERNAL_CALLER
  else
    out2=$(cargo check --workspace --all-targets 2>&1); rc2=$?
    if [ $rc2 -ne 0 ]; then verdict=EXTERNAL_TEST_CALLER
    elif named_dead "$out2" "$name"; then verdict=NO_CALLER_ANYWHERE
    else verdict=UNIT_TEST_CALLER
    fi
  fi
  echo "RESULT|$f|$ln|$name|$verdict"
  mv -f "$f.tri_bak" "$f"; touch "$f"
done < "$LIST"
echo "PROBE_EXIT_STATUS=0"
echo "PROBE_DONE $(date -Is)"
