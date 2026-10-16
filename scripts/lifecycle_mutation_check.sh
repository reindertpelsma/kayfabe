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
P=crates/kf-qemu/src/userpump.rs

TESTS=(--lib --test user_lifecycle_model --test user_lifecycle_props --test user_lifecycle_stress --test user_turn_unit)

for f in $T $L $S $P; do cp "$f" "$OUT/orig/$(basename "$f")"; done
restore() { for f in $T $L $S $P; do cp "$OUT/orig/$(basename "$f")" "$f"; done; }
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
  core_epoch_before_put) # the registry epoch is loaded FIRST (before GP_PUT and the entry), as its own step
    m $T '                self.phase = if self.segment_pending {
                    Phase::Segment
                } else {
                    Phase::Put
                };' '                self.phase = if self.segment_pending {
                    Phase::Segment
                } else {
                    Phase::Epoch
                };'
    m $T '                self.epoch = io.registry_epoch();
                self.phase = Phase::Plan;' '                self.epoch = io.registry_epoch();
                self.phase = Phase::Put;'
    m $T '                self.phase = Phase::Epoch;
                Ok(Step::More)' '                self.phase = Phase::Plan;
                Ok(Step::More)' ;;
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
    m $L '                if !self.markers_off() && self.here == Some(relayed) {
                    true' '                if false && self.here == Some(relayed) {
                    true' ;;
  scan_mask_reset) # D4: the sub-device mask state returns to its birth value at a fresh resume
    m $S '            subdevice: self.subdevice.clone(),
            tally: self.tally,' '            subdevice: self.subdevice.as_ref().map(|s| Subdevice {
                id: s.id,
                active: true,
                stored: None,
            }),
            tally: self.tally,' ;;
  # ---- PHASE 1c (second review): the exact mutants of the review and the new behaviour
  life_no_window_reanchor) # review 2, mutant A: the Window -> Pointer exit no longer re-anchors
    m $L '                    self.counts.to_pointer_from_window += 1;
                    self.anchor(o);' '                    self.counts.to_pointer_from_window += 1;' ;;
  life_anchor_ignores_full_ring) # the first anchor / the exit anchor takes a GP_PUT read with a FULL guest ring
    m $L '        if o.put_full {
            self.counts.anchor_refused_ring_full += 1;
        } else {
            self.safe_end = o.put_abs;
        }' '        self.safe_end = o.put_abs;' ;;
  life_pointer_anchor_ignores_full_ring) # a clean Pointer turn takes a full ring GP_PUT as the anchor
    m $L '                    if !o.put_full {
                        self.safe_end = self.safe_end.max(o.put_abs);' '                    if true {
                        self.safe_end = self.safe_end.max(o.put_abs);' ;;
  life_stale_start_uncounted) # a Window above the newest anchor is not reported
    m $L '                        self.counts.anchor_stale_start += u64::from(stale);
' '' ;;
  life_latch_local_ignored) # a channel that saw a packet open at GP_PUT still places Pointer markers
    m $L '            || self.latched
' '' ;;
  life_latch_shared_ignored) # another channel'"'"'s latch is not honoured
    m $L '            || self.shared.as_ref().is_some_and(|l| l.is_set())
' '' ;;
  life_open_boundary_not_latched) # an open GP_PUT boundary in Scan / Window latches nothing
    m $L '                if open && matches!(self.mode, Mode::Scan | Mode::Window) {' '                if false && open && matches!(self.mode, Mode::Scan | Mode::Window) {' ;;
  life_latch_counted_twice) # every open boundary counts a new latch
    m $L '        let first_here = !self.latched;' '        let first_here = true;' ;;
  life_open_notices_unbounded) # PUT_BOUNDARY_OPEN is reported for every boundary outside a Window too
    m $L 'if self.open_notices <= OPEN_NOTICES || self.mode == Mode::Window {' 'if true {' ;;
  life_stale_boundary) # review 2, N4: a source switch keeps the old ring'"'"'s GP_PUT boundaries
    m $L '        self.next_boundary = None;
        self.here = None;
        self.counted = None;
' '' ;;
  life_mode_no_pointer_markers_ignored) # no_pointer_markers still places Pointer markers
    m $L '        if !self.markers_off() && self.here == Some(relayed) {' '        if self.here == Some(relayed) {' ;;
  life_mode_no_anchored_ignored) # no_anchored_windows still starts at the anchor
    m $L '                    let unanchored = self.kind == LifecycleMode::NoAnchoredWindows;' '                    let unanchored = false;' ;;
  life_mode_always_scan_leaves_scan) # always_scan plans Pointer mode like the default
    m $L '        if self.kind == LifecycleMode::AlwaysScan {
            return scan_plan(false, false);
        }' '        if false {
            return scan_plan(false, false);
        }' ;;
  core_switch_no_post_scan) # review 2, mutant W inside the relay: the switch drops the lifecycle'"'"'s post-switch scan
    m $T '        self.written_get = 0;
        self.life.source_switched();' '        self.written_get = 0;' ;;
  core_switch_keeps_cursor) # the switch does not reset the cursor
    m $T '        self.generation = generation;
        self.cursor = 0;' '        self.generation = generation;' ;;
  core_switch_keeps_pending_get) # the switch keeps a write-back owed to the OLD ring
    m $T '        self.grace.clear();
        self.guest_get = None;
        self.written_get = 0;
        self.life.source_switched();' '        self.grace.clear();
        self.written_get = 0;
        self.life.source_switched();' ;;
  core_new_ring_get_uncounted) # a nonzero GP_GET in the new USERD is not counted
    m $T '        self.life.switch_cursor_mismatch();
        self.written_get = get;
        true' '        self.written_get = get;
        true' ;;
  core_ring_never_full) # the ring-full test is never true
    m $T '        self.put_full = put.wrapping_add(1) & (entries - 1) == self.written_get;' '        self.put_full = false;' ;;
  core_write_back_unrecorded) # the GP_GET written back is not remembered (the ring looks full / empty wrongly)
    m $T '                            self.written_get = get;
' '' ;;
  scan_tally_first_only) # P1: only the first 5080 SET_OBJECT is counted
    m $S '            if software(data) {
                self.tally.software_binds += 1;' '            if software(data) && self.tally.software_binds == 0 {
                self.tally.software_binds += 1;' ;;
  scan_tally_no_rebind) # P1: a rebind of the software subchannel is not counted
    m $S '                self.tally.rebinds += 1;
' '' ;;
  *) echo "unknown source mutant $1" >&2; exit 2 ;;
  esac
}

# PUMP mutants: exact-text patches of crates/kf-qemu/src/userpump.rs, run against `cargo test -p kf-qemu --lib`
pump_mutant() { # name
  case "$1" in
  pump_w_no_switch) # review 2, mutant W: migrate does not switch the relay (no post-switch scan, no cursor reset)
    m $P '                    self.core.switch_source(generation);
' '' ;;
  pump_w_no_mismatch) # migrate never reports a nonzero GP_GET of the new USERD
    m $P '                && self.core.new_ring_get(u32::from_le_bytes(b))' '                && u32::from_le_bytes(b) != 0' ;;
  pump_r_object_is_the_client) # review 2, mutant R (variant): the software object'"'"'s key names the client as the object
    m $P '        client,
        object: sw.guest,' '        client,
        object: client,' ;;
  pump_r_channel_handle) # review 2, mutant R: registry_live asks about the CHANNEL'"'"'s handle as the client
    m $P '            self.r.key.0,
            self.sw.as_ref(),' '            self.r.key.1,
            self.sw.as_ref(),' ;;
  pump_r_no_registry) # registry_live never consults the registry
    m $P '        (Some(reg), Some(sw)) => reg.live_count(software_key(client, sw)),' '        (Some(_), Some(_)) => 0,' ;;
  pump_no_tally_sync) # the SET_OBJECT tally is never handed to the counters
    m $P '        self.core.sync_tally();
' '' ;;
  pump_no_notices) # the lifecycle notices are never printed
    m $P '        self.report_lifecycle_notices();
        self.core.sync_tally();' '        self.core.sync_tally();' ;;
  pump_no_latch) # the VM-wide latch is not handed to the channels
    m $P '                    .with_latch(me.relay_stats.life.latch.clone()),' '                    ,' ;;
  pump_mode_ignored) # KF3_LIFECYCLE_MODE is read but not applied
    m $P '.with_mode(lifecycle_mode())' '.with_mode(kf_chan::user_lifecycle::LifecycleMode::Default)' ;;
  *) echo "unknown pump mutant $1" >&2; exit 2 ;;
  esac
}
run_pump_tests() { # name
  local name=$1
  [ "${DRYRUN:-0}" = 1 ] && { echo "$name: patch applies"; return; }
  cargo test -j2 --release --no-fail-fast -p kf-qemu --lib >"$OUT/$name.log" 2>&1
  local rc=$?
  grep -E "^test .* FAILED$" "$OUT/$name.log" | sed 's/^test //; s/ \.\.\. FAILED$//' | sort -u >"$OUT/$name.failing"
  local cerr=""
  grep -qE "^error(\[|: could not compile)" "$OUT/$name.log" && cerr=" COMPILE-ERROR"
  printf '%s\trc=%s\tfailing=%s%s\n' "$name" "$rc" "$(wc -l <"$OUT/$name.failing")" "$cerr" | tee -a "$OUT/summary.tsv"
  sed 's/^/    /' "$OUT/$name.failing"
}

run_tests() { # name, KEY=VALUE or ""
  local name=$1 envset=$2
  [ "${DRYRUN:-0}" = 1 ] && { echo "$name: patch applies"; return; }
  if [ -n "$envset" ]; then
    env "$envset" cargo test -j2 --release --no-fail-fast -p kf-chan --features lifecycle-mutants "${TESTS[@]}" >"$OUT/$name.log" 2>&1
  else
    cargo test -j2 --release --no-fail-fast -p kf-chan --features lifecycle-mutants "${TESTS[@]}" >"$OUT/$name.log" 2>&1
  fi
  local rc=$?
  grep -E "^test .* FAILED$" "$OUT/$name.log" | sed 's/^test //; s/ \.\.\. FAILED$//' | sort -u >"$OUT/$name.failing"
  local cerr=""
  grep -qE "^error(\[|: could not compile)" "$OUT/$name.log" && cerr=" COMPILE-ERROR"
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
for sm in core_ignore_seed core_no_gate_reset core_epoch_before_put core_double_fetch core_drop_flip_entry core_skip_validation core_bound_without_datum core_reads_beyond_put core_pointer_gone_unchecked core_carried_gate_removed core_no_fresh_resume life_stale_anchor life_no_post_switch life_dead_scans life_forget_len0_sync life_no_boundary_tracking scan_mask_reset life_no_window_reanchor life_anchor_ignores_full_ring life_pointer_anchor_ignores_full_ring life_stale_start_uncounted life_latch_local_ignored life_latch_shared_ignored life_open_boundary_not_latched life_latch_counted_twice life_open_notices_unbounded life_stale_boundary life_mode_no_pointer_markers_ignored life_mode_no_anchored_ignored life_mode_always_scan_leaves_scan core_switch_no_post_scan core_switch_keeps_cursor core_switch_keeps_pending_get core_new_ring_get_uncounted core_ring_never_full core_write_back_unrecorded scan_tally_first_only scan_tally_no_rebind; do
  want "$sm" || continue
  skip "$sm" && continue
  restore
  source_mutant "$sm"
  run_tests "$sm" ""
  restore
done
echo "== PUMP mutants (kf-qemu --lib)"
for pm in pump_w_no_switch pump_w_no_mismatch pump_r_object_is_the_client pump_r_channel_handle pump_r_no_registry pump_no_tally_sync pump_no_notices pump_no_latch pump_mode_ignored; do
  want "$pm" || continue
  skip "$pm" && continue
  restore
  pump_mutant "$pm"
  run_pump_tests "$pm"
  restore
done
echo "== done; results in $OUT (summary.tsv, <name>.failing, <name>.log)"
