#!/usr/bin/env bash
# ★★★ THE ORPHAN GATE — every public verb with NO caller outside its own crate.
#
#   usage: scripts/orphan_gate.sh [<path-glob> ...]      # default: every crate
#          ORPHAN_GATE_LIMIT=20 scripts/orphan_gate.sh crates/kayfabe-isolate
#          ORPHAN_GATE_AXES=default scripts/orphan_gate.sh   # skip the feature axes (FAST, UNSOUND)
#
# ## ⊘⊘ WHY THIS IS NOT A GREP, AND THE MEASUREMENT THAT SETTLES IT
#
# `[measured 2026-08-11]` a name-search returns **the same answer for a working verb and an
# orphaned one**: `MapGuestRam` greps as "zero callers" and runs **8× per boot**. Text search
# was the instrument that failed every time it was load-bearing this campaign — seven times.
#
# ⇒ **Enumeration by text is fine; the VERDICT is the compiler's.** For each candidate the gate
# rewrites `pub fn NAME` to `pub(crate) fn NAME`, runs `cargo check`, and restores.
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
# ## ⊘ IT DOES NOT FAIL THE BUILD ON ORPHANS, AND THAT IS A DECISION
#
# Exit 0 whether or not it finds orphans. The first list over a tree this size is large, it needs
# human triage, and **a gate that goes red on day one gets disabled on day two.** What it
# enforces is a question for after the first triage, not before it.
# ★ **But it DOES exit non-zero when the INSTRUMENT fails** (exit 4) — see §D1. Finding nothing
# because there was nothing, and finding nothing because you did not look, must not share a code.
#
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
#
# ══════════════════════════════════════════════════════════════════════════════════════════════
# ## ⊘⊘ THE FOUR DEFECTS FOUND BY THE w256 FULL SWEEP — FIXED HERE `[2026-08-11, w258]`
#
# The sweep (`traces/gates/orphan_sweep_w256/`, commit d55187a) ran this gate UNMODIFIED over
# 1725 candidates and found four defects **in the gate itself**. Every number below was
# re-measured at w258 before the fix was written; all four reproduced exactly.
#
# ### D1. VACUOUS ADJUDICATION — the worst, because it reads as healthy
#
# `crates/kayfabe-abi/gen/` detaches itself from the workspace (an empty `[workspace]` table at
# `gen/Cargo.toml:28`), **deliberately** — a broken offline ABI generator must never break a
# customer's `cargo build`. Consequence: `cargo metadata` lists 24 packages and `kayfabe-abi-gen`
# is not among them, so `cargo check --workspace` **never compiles it**.
# `[re-measured w258]` appending `this is not rust @@@` to `gen/src/emit.rs`:
#     cargo check --workspace                                  → RC=0    ⊘ garbage compiles
#     cargo check --manifest-path crates/kayfabe-abi/gen/...    → RC=101  ✓ garbage caught
# ⇒ all **14** candidates under `gen/` were adjudicated by a compilation that never read them,
# and in triage they landed in **INTERNAL_CALLER — the bucket that reads as healthy**
# (`grep kayfabe-abi/gen traces/gates/orphan_sweep_w256/triage_all.tsv` → 14/14 INTERNAL_CALLER).
#
# ★ THE FIX IS NOT "attach gen/ to the workspace" — that would destroy the property the
# detachment exists to provide. The fix is that **the gate must adjudicate each candidate with a
# compilation that actually reads it**, and must be able to say so:
#   * every candidate is mapped to its OWNING MANIFEST (nearest ancestor `Cargo.toml`);
#   * a manifest that is a workspace member adjudicates via `--workspace`;
#   * a DETACHED manifest adjudicates via its own `--manifest-path`;
#   * ★★ and before any scope is trusted, it is **PROVEN to be live by garbage injection** —
#     the same test that exposed this defect, now run automatically on every invocation (§SELF-TEST).
#   * a scope whose garbage is NOT caught is marked **VACUOUS**; its candidates are reported
#     `UNADJUDICATED`, **never ORPHAN**, and the gate exits 4.
# ⊘ An empty adjudication must announce itself. This tree has paid for the opposite reading
# repeatedly — `dlen=0` rows, zero-byte job outputs, absent log lines.
#
# ### D2. THE INT/TERM TRAP RESTORED BUT DID NOT EXIT
#
# `trap restore_all EXIT INT TERM` ran the handler and **resumed the loop**, re-mutating
# immediately; bash also defers the trap until the foreground `cargo check` returns. Measured:
# three SIGTERMs left the gate running and the tree dirty; only SIGKILL stopped it, leaving a
# `.orphan_gate_bak` behind. ⇒ the INT/TERM handler now **exits 130**.
#
# ### D3. NON-DEFAULT FEATURES WERE OUTSIDE THE QUANTIFIER
#
# Three crates carry non-default features (`cargo metadata`, w258):
#     kayfabe-device/test-lock-probe   kayfabe-linux-raw/force-host-page-size
#     kayfabe-qemu-raw/host-isolates
# A caller that exists **only** under one of them is invisible to a default-feature check, so the
# verb reads as an orphan. This is the `--all-targets`-quantifies-over-TARGETS-not-FEATURES trap
# one axis over. ⇒ adjudication is now the CONJUNCTION over all axes: a verb is an orphan only if
# `pub(crate)` compiles under **every** axis. ★ Short-circuited on the first FAILURE, because one
# failure already proves the verb is live — so live verbs stay cheap and only orphan candidates
# pay the full multiplier.
#
# ### D4. THE ENUMERATION REGEX MISSED 130 PUBLIC VERBS — ⊘ BUT ITS OBVIOUS FIX IS WRONG
#
# `^[[:space:]]*pub fn ` misses `pub const fn` (**128**) and `pub extern` (**2**, plus 18
# `pub unsafe extern`). Both counts reproduced at w258.
# ⊘⊘ **CORRECTED — adding `extern` to the regex as stated would have INTRODUCED a defect of
# exactly the kind this gate exists to prevent.** `[measured w258]` all **20** `extern` verbs in
# the tree live in `crates/kayfabe-qemu-raw/src/shim_unsafe.rs` and **all 20 carry
# `#[unsafe(no_mangle)]`** — they are the C ABI entry points QEMU's shim calls. Their callers are
# in **another language**, so the compiler is not their adjudicator. Demonstrated on
# `kayfabe_shim_abi_version` (the shim's very first handshake call, `shim_unsafe.rs:725`):
#     pub(crate) extern "C" fn kayfabe_shim_abi_version   → cargo check RC=0 on both axes
# ⇒ the gate would have reported 20 **live** C entry points as orphans. That is the `MapGuestRam`
# shape again: a caller text and now even *rustc* cannot see.
# ⇒ FIX: enumerate `pub const fn` (a real, genuinely-missed Rust verb) **and** enumerate `extern`
# verbs, but classify anything carrying `#[no_mangle]` / `#[unsafe(no_mangle)]` / `#[export_name]`
# as **FFI_EXPORT** — reported in its own bucket, **never as ORPHAN**. A symbol exported to C is
# out of the compiler's jurisdiction, and the gate now says so instead of guessing.
# ══════════════════════════════════════════════════════════════════════════════════════════════
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO" || exit 2
LIMIT=${ORPHAN_GATE_LIMIT:-0}
AXES_MODE=${ORPHAN_GATE_AXES:-all}

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
# ⊘ D2: EXIT must be restore-only (an `exit` inside the EXIT trap re-enters it); INT/TERM must
# restore AND LEAVE. Without the explicit exit the loop resumes and re-mutates the tree.
trap restore_all EXIT
trap 'restore_all; exit 130' INT TERM

declare -a ROOTS=("$@")
[ ${#ROOTS[@]} -eq 0 ] && ROOTS=(crates)

# ---- 0a. THE ADJUDICATION AXES (D3) --------------------------------------------------------
# A verb is an orphan only if `pub(crate)` compiles under EVERY axis. A caller reachable only
# under a non-default feature is a caller.
declare -a AXES=("")
if [ "$AXES_MODE" = "all" ]; then
  AXES=(
    ""
    "--features kayfabe-device/test-lock-probe"
    "--features kayfabe-linux-raw/force-host-page-size"
    "--features kayfabe-qemu-raw/host-isolates"
  )
else
  echo "⚠ ORPHAN_GATE_AXES=$AXES_MODE — default features ONLY. Verdicts are UNSOUND (D3)."
fi

# `check_scope <scope> [extra cargo args...]` — returns 0 iff the tree compiles under ALL axes.
# Short-circuits on the first failure: one failure already proves the verb is live.
check_scope() {
  local scope="$1"; shift
  local -a base
  if [ "$scope" = "workspace" ]; then base=(cargo check --workspace --quiet)
  else base=(cargo check --manifest-path "$scope" --quiet); fi
  local ax
  for ax in "${AXES[@]}"; do
    # ⊘ A detached crate has no workspace features; only the default axis applies to it.
    if [ "$scope" != "workspace" ] && [ -n "$ax" ]; then continue; fi
    # shellcheck disable=SC2086
    "${base[@]}" $ax >/dev/null 2>&1 || return 1
  done
  return 0
}

# ---- 0b. the baseline must be green, or every verdict below is meaningless -----------------
# ★ A tree that does not compile makes "it still compiles" vacuously false for every candidate,
# and the gate would report EVERY verb as wired — a silent all-clear. Checked, not assumed.
echo "== baseline: cargo check (all ${#AXES[@]} axis/axes)"
if ! check_scope workspace; then
  echo "★ FAIL: the tree does not compile before any mutation. Every verdict would be garbage."
  exit 3
fi
echo "   baseline OK"

# ---- 0c. MAP EVERY CANDIDATE FILE TO THE COMPILATION THAT ACTUALLY READS IT (D1) ------------
# Workspace member manifests, straight from cargo — never inferred from the directory layout.
mapfile -t WS_MANIFESTS < <(
  cargo metadata --no-deps --format-version 1 2>/dev/null |
    python3 -c 'import json,sys;[print(p["manifest_path"]) for p in json.load(sys.stdin)["packages"]]'
)
if [ ${#WS_MANIFESTS[@]} -eq 0 ]; then
  echo "★ FAIL: cargo metadata listed no packages; scope mapping impossible."
  exit 3
fi

# owning_scope <file> -> "workspace" | <abs path to a detached Cargo.toml>
owning_scope() {
  local d; d="$(cd "$(dirname "$1")" && pwd)"
  while [ "$d" != "/" ]; do
    if [ -f "$d/Cargo.toml" ]; then
      local m="$d/Cargo.toml" w
      for w in "${WS_MANIFESTS[@]}"; do
        [ "$w" = "$m" ] && { echo workspace; return; }
      done
      echo "$m"; return
    fi
    d="$(dirname "$d")"
  done
  echo UNOWNED
}

# ---- 1. ENUMERATE candidates (text is fine here — the verdict is not text's) ---------------
# Every inherent `pub` verb: `pub fn`, `pub const fn`, `pub unsafe fn`, `pub async fn`,
# `pub extern "C" fn`, `pub unsafe extern "C" fn` (D4). Skipping:
#   - `pub(` — already scoped
#   - files under tests/ or benches/
#   - anything inside a `trait` block (visibility is the trait's; see the header)
# ★ D4: a `#[no_mangle]`/`#[export_name]` attribute in the preceding attribute run marks the verb
# FFI — its caller is in C and rustc cannot adjudicate it.
mapfile -t CANDS < <(
  for root in "${ROOTS[@]}"; do
    find "$root" -name '*.rs' -not -path '*/tests/*' -not -path '*/benches/*' 2>/dev/null
  done | sort -u | while read -r f; do
    awk -v F="$f" '
      /^[[:space:]]*(pub )?trait [A-Za-z]/ { intrait=1; depth=0 }
      intrait { depth += gsub(/{/,"{"); depth -= gsub(/}/,"}"); if (depth<=0 && NR>1) intrait=0; next }
      /^[[:space:]]*#\[/ {
        if ($0 ~ /no_mangle/ || $0 ~ /export_name/) ffi=1
        next
      }
      /^[[:space:]]*pub[[:space:]]/ {
        head=$0; sub(/[(<].*$/,"",head)
        if (head ~ /[[:space:]]fn[[:space:]]+[a-z_][A-Za-z0-9_]*[[:space:]]*$/) {
          name=head; sub(/^.*[[:space:]]fn[[:space:]]+/,"",name); gsub(/[[:space:]]/,"",name)
          print F "\t" NR "\t" name "\t" (ffi ? "FFI" : "RS")
        }
        ffi=0; next
      }
      /^[[:space:]]*$/ { ffi=0 }
    ' "$f"
  done
)

TOTAL=${#CANDS[@]}
echo "== candidates enumerated: $TOTAL  (inherent \`pub\` verbs; trait methods are out of scope)"
[ "$LIMIT" -gt 0 ] && echo "   ORPHAN_GATE_LIMIT=$LIMIT — checking the first $LIMIT"

# ---- 1b. ★★ SELF-TEST: PROVE EVERY SCOPE IS LIVE BY GARBAGE INJECTION (D1) ------------------
# This is the exact test that exposed D1, promoted from a one-off into the instrument. For each
# distinct scope actually present among the candidates, append garbage to one of its files and
# require the adjudication command to FAIL. A scope that swallows garbage cannot adjudicate
# anything, and every verdict it would produce is vacuous.
declare -A SCOPE_OF_FILE=() SCOPE_LIVE=() SCOPE_WITNESS=()
for row in "${CANDS[@]}"; do
  f=${row%%$'\t'*}
  [ -n "${SCOPE_OF_FILE[$f]:-}" ] && continue
  s="$(owning_scope "$f")"
  SCOPE_OF_FILE[$f]="$s"
  [ -z "${SCOPE_WITNESS[$s]:-}" ] && SCOPE_WITNESS[$s]="$f"
done

echo "== self-test: garbage injection per scope (${#SCOPE_WITNESS[@]} scope/s)"
VACUOUS=0
for s in "${!SCOPE_WITNESS[@]}"; do
  w="${SCOPE_WITNESS[$s]}"
  if [ "$s" = "UNOWNED" ]; then
    echo "   ⊘ VACUOUS  scope=UNOWNED (no ancestor Cargo.toml)  witness=$w"
    SCOPE_LIVE[$s]=0; VACUOUS=1; continue
  fi
  cp "$w" "$w.orphan_gate_bak" || { echo "   ⊘ VACUOUS  scope=$s (cannot back up $w)"; SCOPE_LIVE[$s]=0; VACUOUS=1; continue; }
  printf '\nthis is not rust @@@\n' >> "$w"
  if check_scope "$s"; then
    # The compilation returned 0 with syntactic garbage in the file ⇒ it never read it.
    echo "   ⊘⊘ VACUOUS  scope=$s  witness=$w — GARBAGE COMPILED. Verdicts here would be empty."
    SCOPE_LIVE[$s]=0; VACUOUS=1
  else
    echo "   ✓ live     scope=$s  witness=$w"
    SCOPE_LIVE[$s]=1
  fi
  mv -f "$w.orphan_gate_bak" "$w"; touch "$w"   # ⊘ mtime trap
done

# ---- 2. ADJUDICATE each one with the compiler ----------------------------------------------
orphans=0
checked=0
skipped_ffi=0
unadjudicated=0
declare -a FOUND=() FFI=() UNADJ=()
for row in "${CANDS[@]}"; do
  [ "$LIMIT" -gt 0 ] && [ "$checked" -ge "$LIMIT" ] && break
  f=${row%%$'\t'*}; rest=${row#*$'\t'}; ln=${rest%%$'\t'*}
  rest2=${rest#*$'\t'}; name=${rest2%%$'\t'*}; kind=${rest2##*$'\t'}
  scope="${SCOPE_OF_FILE[$f]}"

  # ⊘ D4: a symbol exported to C is out of rustc's jurisdiction. Reported, never adjudicated.
  if [ "$kind" = "FFI" ]; then
    FFI+=("$f:$ln  $name")
    skipped_ffi=$(( skipped_ffi + 1 ))
    continue
  fi
  # ⊘ D1: a scope that swallows garbage answers every question with silence.
  if [ "${SCOPE_LIVE[$scope]:-0}" -ne 1 ]; then
    UNADJ+=("$f:$ln  $name  (scope=$scope)")
    unadjudicated=$(( unadjudicated + 1 ))
    continue
  fi

  cp "$f" "$f.orphan_gate_bak" || continue
  # ⊘ Line-addressed, so a name that appears twice in one file cannot mutate the wrong one.
  # ⊘ D4: replace the LEADING `pub `, so `pub const fn` / `pub unsafe extern "C" fn` scope too.
  sed -i "${ln}s/^\([[:space:]]*\)pub /\1pub(crate) /" "$f"
  if check_scope "$scope"; then
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

# ---- 3. REPORT. No failure on orphans, by design (see the header) --------------------------
echo
echo "== orphan gate: $orphans of $checked adjudicated have no caller outside their own crate"
if [ "$orphans" -gt 0 ]; then
  printf '   %s\n' "${FOUND[@]}"
  echo
  echo "⊘ This is a REPORT, not a verdict on any one of them: a verb may be a deliberate seam"
  echo "  awaiting its consumer, or a capability proven and never wired. Only a human knows"
  echo "  which, and that triage is what decides what this gate eventually enforces."
fi
if [ "$skipped_ffi" -gt 0 ]; then
  echo
  echo "== FFI_EXPORT: $skipped_ffi verb/s exported to C — NOT adjudicable by rustc (D4)"
  printf '   %s\n' "${FFI[@]}"
  echo "⊘ Their callers are in another language. Absence of a Rust caller is not evidence here."
fi
if [ "$unadjudicated" -gt 0 ]; then
  echo
  echo "★ UNADJUDICATED: $unadjudicated candidate/s in a VACUOUS scope — NOT orphans, NOT clean (D1)"
  printf '   %s\n' "${UNADJ[@]}"
fi
# ⊘ Orphans → 0 (see the header: a day-one red gate is a gate nobody runs on day two).
# ★ An INSTRUMENT failure → 4. "Found nothing" and "could not look" must not share an exit code.
if [ "$VACUOUS" -ne 0 ]; then
  echo
  echo "★ EXIT 4: at least one scope could not adjudicate. The numbers above are INCOMPLETE."
  exit 4
fi
exit 0
