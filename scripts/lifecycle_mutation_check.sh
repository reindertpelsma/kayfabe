#!/usr/bin/env bash
# Mutation check of the Windows USER-ring lifecycle (docs/design/V3_USER_RING_LIFECYCLE_20261016.md section 8).
#
# Two kinds of deliberate defect, each followed by a re-run of the whole GPU-free lifecycle suite; the tests that
# FAIL are listed. A mutant no test catches is a missing test; a mutant caught by ONE test is listed as thin.
#   * machine mutants: `kf_chan::user_lifecycle::Mutant`, selected with KF_LIFECYCLE_MUTANT (feature
#     `lifecycle-mutants`, never enabled by another crate);
#   * SOURCE mutants (since PHASE 1b): an exact-text patch of the PRODUCTION source (`user_turn.rs`,
#     `user_lifecycle.rs`, `user_scan.rs`) that the same script reverts afterwards. The per-entry turn is the code the
#     pump runs, so a source mutant is a mutation of the real relay.
# The unmutated run must be green first.
#
# usage: scripts/lifecycle_mutation_check.sh [outdir]      (CARGO_TARGET_DIR keeps the build elsewhere;
#        ONLY="name1 name2" runs just those; RESUME=1 skips mutants with a result file)
# GPU-free; never touches hardware.
set -u
cd "$(dirname "$0")/.."
OUT=${1:-/tmp/lifecycle_mutation}
mkdir -p "$OUT/orig"
export KF3_STRESS_SECS=${KF3_STRESS_SECS:-1}
T=crates/kf-chan/src/user_turn.rs
L=crates/kf-chan/src/user_lifecycle.rs
S=crates/kf-chan/src/user_scan.rs

TESTS=(--lib --test user_lifecycle_model --test user_lifecycle_props --test user_lifecycle_stress)

for f in $T $L $S; do cp "$f" "$OUT/orig/$(basename "$f")"; done
restore() { for f in $T $L $S; do cp "$OUT/orig/$(basename "$f")" "$f"; done; }
trap restore EXIT

# m FILE OLD NEW : replace the exact text OLD by NEW in FILE (must occur exactly once)
m() {
  OLD="$2" NEW="$3" perl -0pi -e 'BEGIN{$o=$ENV{OLD};$n=$ENV{NEW};} $c=()=/\Q$o\E/g; die "mutation text not found exactly once ($c): $o\n" if $c!=1; s/\Q$o\E/$n/;' "$1" || { echo "MUTATION DID NOT APPLY" >&2; exit 2; }
}

source_mutant() { # name
  case "$1" in
  core_ignore_seed) # T1: the seeded gate is ignored at a Fresh window start
    m $T '        let gate_entry = if self.seed {
            self.raw | GP_SYNC_WAIT
        } else {
            self.raw
        };' '        let gate_entry = self.raw;' ;;
  core_no_gate_reset) # a stale gate proof one ring lap later
    m $T '                    self.sync = SyncGate::default();
' '' ;;
  core_epoch_before_put) # the registry epoch is loaded BEFORE GP_PUT and the entry
    m $T '        let Some(put) = io.gp_put()? else {' '        self.epoch = io.registry_epoch();
        let Some(put) = io.gp_put()? else {'
    m $T '                self.epoch = io.registry_epoch();
                self.phase = Phase::Plan;' '                let _ = io.registry_epoch();
                self.phase = Phase::Plan;' ;;
  core_double_fetch) # a second load of the entry is what gets stored
    m $T '        io.publish_entry(
            kind,
            &mut self.fetch,
            self.generation,
            self.next_get,
            self.raw,
        )?;' '        let lo = io.load_dword(self.cursor, false)?;
        let hi = io.load_dword(self.cursor, true)?;
        io.publish_entry(
            kind,
            &mut self.fetch,
            self.generation,
            self.next_get,
            (u64::from(hi) << 32) | u64::from(lo),
        )?;' ;;
  core_drop_flip_entry) # the first entry of a Fresh window is skipped
    m $T '                    self.life.restarted();
' '                    self.life.restarted();
                    self.cursor = self.next_get;
                    self.relayed += 1;
                    self.again = true;
                    return self.idle_step(false);
' ;;
  core_skip_validation) # an entry inspect_entry refused is forwarded anyway
    m $T 'let desc = match inspect_entry(self.raw, io.max_entry_bytes()) {' 'let desc = match inspect_entry(self.raw, io.max_entry_bytes())
            .or(Ok::<Source, crate::user_stream::Refusal>(Source::Nop))
        {' ;;
  core_bound_without_datum) # a SET_OBJECT header (datum unscanned) counts as the binding
    m $T '        let sw_mask = self.scan.software_mask(|v| io.sw_matches(v));' '        let mut sw_mask = self.scan.software_mask(|v| io.sw_matches(v));
        if let Some((h, _)) = self.scan.carried() {
            if h.method == 0 && h.subchannel >= 5 {
                sw_mask |= 1 << h.subchannel;
            }
        }' ;;
  core_reads_beyond_put) # I8: GP_PUT == cursor is treated as work: reads one entry at/beyond GP_PUT
    m $T '        if put == self.cursor {' '        if false && put == self.cursor {' ;;
  core_pointer_gone_unchecked) # D6: Pointer mode never notices a freed software object
    m $T 'if self.life.mode() == Mode::Pointer && view.gone {' 'if false && view.gone {' ;;
  core_carried_gate_removed) # D1 family: a SYNC_WAIT behind an open packet is gated (marker inside the packet)
    m $T 'if gate_entry >> 63 & 1 == 1 && self.scan.carried().is_some() {' 'if false {' ;;
  core_no_fresh_resume) # D2 family: the parser is not restarted at a Fresh window (stale open packet)
    m $T '                    self.scan = self.scan.fresh_resume();
' '' ;;
  life_stale_anchor) # D2: a turn that SHOWED the registration moves the anchor (its GP_PUT is not pre-reply)
    m $L '            Mode::Pointer => {
                if self.live == 0 {' '            Mode::Pointer => {
                self.safe_end = self.safe_end.max(o.put_abs);
                if self.live == 0 {' ;;
  life_no_post_switch) # D5: the Window may end right after a source switch
    m $L ' && self.post_switch == 0 {' ' {' ;;
  life_dead_scans) # D6: a torn-down machine returns a scan plan
    m $L '            return Plan::Stop;' '            return scan_plan(false, false);' ;;
  life_forget_len0_sync) # T2: Pointer mode does not record a LENGTH-0 SYNC_WAIT
    m $L '                if raw & GP_SYNC_WAIT != 0 {' '                if raw & GP_SYNC_WAIT != 0 && (raw >> 42) & 0x1f_ffff != 0 {' ;;
  life_no_boundary_tracking) # D3: a Pointer-mode marker is never allowed (only the base spin / backstop wake)
    m $L '                if self.here == Some(relayed) {
                    true' '                if false && self.here == Some(relayed) {
                    true' ;;
  scan_mask_reset) # D4: the sub-device mask state returns to its birth value at a fresh resume
    m $S '            subdevice: self.subdevice.clone(),
        }
    }

    /// Whether the metadata is between' '            subdevice: self.subdevice.as_ref().map(|s| Subdevice {
                id: s.id,
                active: true,
                stored: None,
            }),
        }
    }

    /// Whether the metadata is between' ;;
  *) echo "unknown source mutant $1" >&2; exit 2 ;;
  esac
}

run_tests() { # name, KEY=VALUE or ""
  local name=$1 envset=$2
  if [ -n "$envset" ]; then
    env "$envset" cargo test -j2 --release --no-fail-fast -p kf-chan --features lifecycle-mutants "${TESTS[@]}" >"$OUT/$name.log" 2>&1
  else
    cargo test -j2 --release --no-fail-fast -p kf-chan --features lifecycle-mutants "${TESTS[@]}" >"$OUT/$name.log" 2>&1
  fi
  local rc=$?
  grep -E "^test .* FAILED$" "$OUT/$name.log" | sed 's/^test //; s/ \.\.\. FAILED$//' | sort -u >"$OUT/$name.failing"
  local cerr=""
  grep -q "^error" "$OUT/$name.log" && cerr=" COMPILE-ERROR"
  printf '%s\trc=%s\tfailing=%s%s\n' "$name" "$rc" "$(wc -l <"$OUT/$name.failing")" "$cerr" | tee -a "$OUT/summary.tsv"
  sed 's/^/    /' "$OUT/$name.failing"
}

want() { [ -z "${ONLY:-}" ] || [[ " $ONLY " == *" $1 "* ]]; }
skip() { [ "${RESUME:-0}" = 1 ] && [ -f "$OUT/$1.failing" ]; }

if want baseline && ! skip baseline; then echo "== baseline (no mutant): must be green"; run_tests baseline ""; fi

echo "== machine mutants (KF_LIFECYCLE_MUTANT)"
for mm in pointer_scans no_seed forget_sync pointer_to_scan revive_after_teardown exit_ignores_live mask_frozen anchor_ignored pointer_markers_anywhere pointer_no_markers; do
  want "machine_$mm" && ! skip "machine_$mm" && run_tests "machine_$mm" "KF_LIFECYCLE_MUTANT=$mm"
done

echo "== source mutants (exact-text patch of the production source, reverted after each run)"
for sm in core_ignore_seed core_no_gate_reset core_epoch_before_put core_double_fetch core_drop_flip_entry core_skip_validation core_bound_without_datum core_reads_beyond_put core_pointer_gone_unchecked core_carried_gate_removed core_no_fresh_resume life_stale_anchor life_no_post_switch life_dead_scans life_forget_len0_sync life_no_boundary_tracking scan_mask_reset; do
  want "$sm" || continue
  skip "$sm" && continue
  restore
  source_mutant "$sm"
  run_tests "$sm" ""
  restore
done
echo "== done; results in $OUT (summary.tsv, <name>.failing, <name>.log)"
