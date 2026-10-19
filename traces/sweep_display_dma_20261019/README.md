# Display DMA format from the public class

STATUS: LIVE, 2026-10-19. GPU-free validation complete; GB202 candidate desktop MEASURED FAIL.
Code45a4d91b5, separate unmerged branch codex/sweep-display-dma-20261019.
Accepted integration baseline b6accbca2/runtime24906bf33 contains candidate5f8f9d1a5f.
The optional CE95e23df77 branch is not merged or included.

Public CA7B/CA7D/CA7E headers specify METHOD_OFFSET and JUMP_OFFSET15:2,
addressing64KiB rings. The old decoder used method13:2/jump11:2 and both it
and the worker limited rings to4KiB. Valid Blackwell rings were clipped or
rejected; upper method bits could alias a lower method. The new format is
compiled from the exact public class table, not captured per-die data.
All20added rows at each of five ABI tags are compiler-derived. All prior rows
are unchanged. The bytes of all ten contributing headers per ABI were checked
against pinned official NVIDIA GitHub commits; hashes are in derivation.json.
No private interface, firmware decompile or native caps-page value is used.

Tests-first history: e3cc4570c has6runtime assertion failures/2legacy passes;
1b298ee41 extracts the original worker bound without changing its behavior;
bbec1d188 has4runtime worker-bound failures/4passes. Fix45a4d91b5 turns them
green. Current full releases: kf-disp284/0/5 ->303/0/5, kf-qemu407/0/0 ->416/0/0.
All existing tests and assertions are retained. Three old source-audit tests
initially saw a truncated prefix because the new test module was placed before
production functions; moving only that new module to the end restored them,
without changing their assertions or production pacing. Mutation counts are
8/3/2/2/2/4/2/2/2, runtime failures only. Two final AD104 compatibility tests
were added after the mutation run; all killing tests remain present.

Worker limits remain min(declared size, context-DMA limit, generated class
limit), word-aligned. Every format is bounded to at most64KiB; jump traversal
retains its len/4 budget and queue capacity stays4096. Core/window/window-immediate
select their own generated class; PIO cursor retains its prior defensive4KiB
bound. Legacy public decoder APIs retain their old format. No new unsafe code,
blocking, host verb, vCPU guest-memory read, clock,500ms heartbeat-period change,
pacing/tick/credit/GET-order/lifecycle/scrub/relay/channel-birth edit is made.
The only worker calculation change is its class-specific ring read bound.

AD104: Ada coreC77D/windowC67E/immediateC67B retain public method13:2,
jump11:2 and4KiB bounds. Exact580.65.06 method/jump corpora, including hostile
upper bits and cycles, match the prior public decoder byte for byte. This is
CPU compatibility evidence, not owner-host hardware parity. Existing exact
c56f family-admission assertions remain unchanged in the full QEMU suite.

Clippy:0primary diagnostics on changed lines; baseline/candidate each emit480
raw inherited warnings under identical all-targets -Dwarnings/cap-lintswarn.
An initial shared-target comparison reused baseline rmeta for the other
worktree (artifact fresh=true) and produced missing-API compile errors. It is
rejected; accepted clippy and final release use an exclusive candidate cache.
Rustfmt, bash syntax and diff whitespace checks pass. Inherited diagnostics
are not a blanket-clean claim.

Hardware baseline on owned GB20256788212 at24906bf33 is documented separately
on codex/sweep-integrate-20261018: native9+10/raw30/CUDA4PASS, guest595concurrency
41sPASS but31zero-heartbeat startup warnings, guest580Cinnamon/NVIDIA yet black
X11 SHM window/one flip timeout. A candidate must still prove functional output.
The Linux disk has post-baseline writes; a later PASS is functional evidence,
not a perfect same-snapshot causal comparison. No second capability-group run.

GB202 candidate run completed22:07:32..22:10:53UTC, owned56788212. Code45a4d91b5
QEMU SHA4e57223b50c474718c90b7939110a7e064ce74aad207a9a0b901235182b2590d.
Native gates9/9+10/12USER PASS. Linux580.159.04 still showed a one-color black
1920x1080 desktop, one flip timeout,12nvAssert,0guestXid, no host Xid increase.
PixelSHA788ae0147bdf979a6575938ca2d7d4403788588f7be2010f03776c968fd1ab49
is identical to baseline. Thus the codec fix did not fix B2 in this run. No
third B2 attempt this lease. Both hardware receipts and raw Linux evidence
are on the integration branch under phase2_blackwell_56788212/b2_codec_candidate.
The candidate remains a separate, unmerged public-format topic, draftPR51.
