# Candidate 1 after the B5 fix — recovered hardware evidence

**STATUS: VERIFIED, 2026-10-04; recovered 2026-10-04.** The tested revision is
`0ac157b2fff15294073b95acbe7377a6133ae5aa` on `v3-cand-1`, including the B5 fix
`c1ca7945`. These runs completed on box **54049598** (vmb, RTX 3060 / GA106,
host 580.159.04) between 02:13 and 04:11 UTC. They predate this recovery session;
they are recovered results, not newly executed tests. The earlier handoff was
written before the jobs finished and its B5-pending statement is superseded.

## Result and provenance

| Check | Result at `0ac157b2` | Evidence |
|---|---|---|
| GitHub CI, including slow and firmware | All four jobs passed | [run 37170206222](https://github.com/reindertpelsma/kayfabe/actions/runs/37170206222) |
| Full kf-* tests, second unchanged attempt | 1,937 passed, zero failed | `jobs/cand1bmb2_tests.log`, `jobs/cand1bmb2.log` |
| Real-GPU gates | 9/9 | `jobs/cand1bmb2_gates.log` |
| Revision-specific QEMU and fresh fast guest | Both built successfully | `jobs/cand1bmb2_kf3.log`, `jobs/cand1bmb2_fg.log` |
| Bare-metal / thin-guest suites | 30/30 each, no crashes or omitted arms | `jobs/cand1bmb2_host.log`, `merge_bar/suite.out` |
| Channel births | 161 suite + 11 gate births, all USER | `jobs/cand1bmb2_births.log` |
| Apps | Host 71/71; guest 61/65 apps + 6/6 probes | `apps/comparison.txt`, `apps/{host,guest,guest_isolated}.res` |
| GOP B0 / B1 | Pixel-exact 1920x1080, 120/120 flips each; B1 no black handoff frame | `display/{b0c1b,b1c1b}/lane.log` |
| B5 unload and restore | **Four runs, 14/14 arms each**, no context-DMA refusal | `display/{b5c1b,b5r1b,b5r2b,b5r3b}/lane.log` |
| X11 display-SW A/B | Both lanes complete with the documented off/on behavior | `display/dsw_{a_off,b_on}_c1b/` |

The merge bar's own detached checkout was
`/workspace/bench/verify-worktrees/cand1bmb2.ldK7Pa`; it still had no tracked edits
when inspected during recovery. Its log records the full revision and `EXIT rc=0`.
Apps and display used `/workspace/bench/kf3-bins/0ac157b2/qemu-system-x86_64`,
SHA-256 prefix `4267680c50759413`; their driver logs record the same revision and
clean source tree. Their individual verdicts were checked: the orchestration
scripts' final exit alone is not a success criterion.

`source_manifest.json` records the box path, uncompressed size and SHA-256 of
each of the 592 recovered files. Every file was checked against that manifest
after transfer. Large text logs have a `.gz` suffix; `.zst` files are the app
harness's compressed QEMU logs. No executable, guest image, credential or TPM
state is included. `recipes/` contains the local session's orchestration as text.
`merge_bar/recovered_session/` also preserves the already-collected local record,
including the failed first attempt and the test diagnosis below.

## B5 and review

All four runs (`b5c1b`, `b5r1b`, `b5r2b`, `b5r3b`) show the restored console
continuing to update after X exits, a `PRESERVED scanout` line, and zero
`scanout REFUSED context DMA` lines. The later fbdev unload still becomes black,
as required. B1 and every B5 run report zero black frames at the initial handoff.
All eight display boots carry guest NVRM logs and zero guest Xids; the display
stage's host journald record has zero Xids and zero NVRM errors.

Recovery review checked the B5 delta in `crates/kf-qemu/src/display.rs`:
`LatchedDmas` retains only a resolved context DMA for that window's armed
`(client, handle, channel)`; it is invalidated on a new latch, window allocation
or free, and new instance memory. Failed resolution is not cached. Planning
still applies the surface bounds and refuses system memory, and composition
still bounds reads to the guest's store. Flip completion/GET waits for the
first copy, before the driver's idle-then-unbind sequence. The existing
`a_context_dma_unbound_before_the_preserving_free_keeps_the_console` regression
test covers the gap, the preserving free, re-latch refusal and new lifetime.
No blocking finding in that delta was identified. The earlier candidate merge
review is in `docs/handoff/2026-10-04/cand1_result.json`; its low findings were
fixed before this tested revision.

The X11-on log ends with `dispsw[twins=84 live=0 host_refused=0 ...]`, with all
other refusal counters zero. All 76 channel births on that arm are USER, with
`PRIVILEGED_CHANNEL=0`; the other seven display arms also have no privileged
birth. The default remains off, pending the separate security prerequisites.

## Existing failures and limitations

**The first merge-bar attempt failed**, at the same revision, in
`churning_births_and_frees_never_lose_a_store_or_ring_the_wrong_twin`:
`fast_late=574` (`jobs/cand1bmb_tests.log`). The unchanged second attempt passed.
This is not silently discarded. The recovered diagnosis identifies a test-data
collision: fake host tokens are `0xA000 + generation` and `0xB000 + generation`;
after 4,096 generations an A token reuses a freed B token's number. The fake
host's permanent freed-token set then reports a false late ring.

The session's isolated experiment recorded 16/30 failures with the original
numbers, all above 4,096 rounds, and 0/30 with wider-separated numbers, including
runs beyond 4,096 rounds (`merge_bar/recovered_session/flaky_churn_test/`).
Its wider-number variant was temporary, not part of this candidate. Repeat-run
notes also record failures on the older candidate and master's identical test.
The committed test still needs a collision-free identity scheme; this recovery
changes no tests or product code.

App-by-app comparison has **zero verdict differences** from both `2830988f`
and `8a682f1b`, including each failed app run alone. The same four unsupported
managed-memory apps fail, with eight host Xid 31 records across the batched and
isolated runs. `conjugateGradientUM` still prints a wrong answer with a success
status; the loud-UVM lane remains required. Host Xids are read from journald,
not the wrapping dmesg-ring delta. The previous candidate's PAT warning remains
in the recovered kernel logs; it is not resolved by the B5 fix. This run covers
GA106, not every GPU family, and does not cover the separate broker/max-fps,
P1/P2 or Windows branches.

The recovery branch merges master's two newer documentation commits and adds
only documentation/evidence to `0ac157b2`. Its product, build scripts and test
sources are byte-identical to that hardware-tested revision. The recovery
commit requires green CI before promotion under `OWNER_RULINGS` §R's
documentation-only rule. Check the recovery branch's latest Actions run for that
result; its head changes only when documentation or evidence is added.
