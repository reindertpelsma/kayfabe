**STATUS: LIVE-RESEARCH, 2026-10-21 (evening).** GB202 (RTX 5090) Windows 580.88 under kayfabe, resumed from
Codex's sweep (`codex/sweep-win-20261021` @6728c49b9, frozen product source af8bdcda4). Branch
`claude/blackwell-resume-20261021`. Part 1-4 are GPU-free analysis of committed evidence (written before any
run). Part 5 is the run plan; part 7 records what was executed. Labels: **MEASURED** = a committed artifact
named here; **CODE READ** = repo source or public ogkm; **INFERRED** = reasoning, never evidence. No value in
this document is derived from a closed binary; no closed-driver bytes were opened.

# 1. What the committed evidence says (new cross-reads, not in SW2.md)

1. **MEASURED, display OFF timeline** (`traces/sweep_windows_20261021/lease_57046541/bw_off_m1/`,
   `rpc_timing.jsonl` observation clock, host `host_xids.txt`, `OUTCOME.json`). Times UTC, 13:42:
   Code 0 `23.644`; ordinal 3541 `26.829`; **host Xid32 x2 (QEMU pid, channels `0x0000000c` and
   `0x0e000007`, intr `00200000`) `27.205` and `27.234`**, i.e. inside the 0.40 s gap between ordinals 3541
   and 3542; RPCs resume, then a **3.1 s silence** (`29.438` -> `32.545`); then the guest's channel
   recovery, `NV906F_CTRL_CMD_RESET_CHANNEL` (`0x906f0102`) x4 at `32.545`, all `none` (refused, 0x56);
   2.3 s later (`34.850`) the adapter teardown (ordinal 3618-3624, `Free` run) and `UnloadingGuestDriver`
   (ordinal 3920) at `35.965`; **no RPC after it**; fault-dump SystemTime (0x116) `45.709`; QMP panic
   `46.316`. So the Xid32 precedes the guest's recovery attempt by 5.3 s, the unload by 8.7 s and the
   bugcheck by 18.5 s.
2. **MEASURED, Xid32 versus revision.** `host_xid=` in each arm's `run_end.txt`
   (`claude/blackwell-resume-20261020:traces/blackwell_resume_20261020/`): every Code-0 arm before fix4
   (fix2_off_1, fix2_off_2, fix3_off_1 lease 1; fix3_off_1 lease 2; capsdiag_on_1; caps_highb_on) has
   `host_xid=0` and still ends in 0x116. The first arm with the Blackwell 3D report-semaphore methods
   admitted (fix4, `lease2/arms/fix4_off_1`, same board, one variable after fix3_off_1) has
   **`host_xid=2`** (never analysed: HANDOFF_CODEX.md does not mention it), and the af8 arm has the same
   two. n=2 with, n=6 without; identical downstream 0x116 (P3 `0xC000009A`, P4 `4`).
3. **MEASURED, 0x50 signature.** ON bugcheck 0x50 parameters (af8 `bw_on_m1`): P1 `0xffffdb047809fcc7`, P2 2,
   P3 `0xfffff80735efa1b0`, P4 2. The earlier POSTCOMP-only native-page arm (`caps_post_on`, lease 1, a
   different served page) bugchecked with P1 `0xffffe5907c09fcc7`, P2 2, P3 `0xfffff8032fd3a1b0`, P4 2.
   **The low 16 bits of P3 (`0xa1b0`) and the low 24 bits of P1 (`0x09fcc7`) are equal in both**; the
   bits above differ as ASLR/pool placement would. INFERRED from it: the same instruction faults on the
   same fixed offset from a randomised base in both runs (a code location, not noise); the module is not
   identified (no dump left).
4. **MEASURED, which served caps differ from the die's own page, GB202, driver tables 580.65.06.**
   `cargo run -p kf-disp --example caps_oracle -- 580.65.06 0x1b0 0x2 <own-die read>` (the repo's
   exact-equality gate; the own-die read is `bwr_caps_page_580.txt` already in branch
   `claude/blackwell-resume-20261020`; used here as an oracle, never as a source): **125 of the page's
   words differ**. In the 0x680-0x87c region that matters for ON: POSTCOMP serves, per virtual tile,
   only `CAPA` (`0x007b0000`) and `CAPB` (`0x00060802`); `CAPC..CAPG` are 0 and the die has 8 tiles'
   worth of words where 4 are served; PRECOMP serves `CAPA` without its low half, `CAPB` different, `CAPC`,
   `CAPE`, `CAPF` zero, `CAPD` partial. SYS_CAPC (`0x020`) serves `0xf`. (The oracle prints the die values;
   they are not copied here beyond the two POSTCOMP words above.)
5. **MEASURED, the ON outcome ladder across all committed arms** (lease 1/2 and the sweep), by what is
   served in 0x680-0x87c: nothing -> Code 43 (n=10, abort after the 4th `CA7A` cursor alloc); PRECOMP only
   -> Code 43; derived POSTCOMP `CAPA` only (fix5_on_1) -> Code 43; **POSTCOMP with `CAPB` (OLUT) nonzero
   and PRECOMP not the die's: native POSTCOMP (caps_post_on) -> 0x50; derived `CAPA`+`CAPB` (af8) -> 0x50**;
   POSTCOMP and PRECOMP both the die's (whole page, high-B) -> Code 0 then 0x116.
6. **MEASURED, CA7A allocation path** (`bw_on_m1_canonical/allocations.jsonl`, `allocations.receipt.json`):
   all four `CA7A` (cursor) allocations have envelope result 0, body status 0, declared params 16 B,
   posted payload 48 B, header decode `min32,class@12,status@16,paramsSize@20`. 48 = 32 (header) + 16
   (params): the sizes are consistent with the public `rpc_gsp_rm_alloc_v03_00` layout, so the old
   "unknown status" gap is closed and **ordinal 275 versus 277 is only the old regex dropping two
   `Other(4)` records** (original 276-row receipt retained, canonical 278 rows). Nothing in the cursor
   RPC path is a defect on the evidence; the crash is guest-internal, after a fully successful reply.
7. **CODE READ, what the host does on a Translated-ring RC.** The Translated host ring is born with
   `err_notifier: 0` (`crates/kf-chan/src/host.rs:427-433`; `channel_alloc_request_with` passes it as
   `h_object_error`). The RC-forwarding machinery (`PtNotifier`, `RcEvent`, `crates/kf-qemu/src/chan.rs:548-640`,
   `7361-7375`) exists only for **Passthrough twins** (the guest's own error notifier is armed on the
   host channel). A host RC (Xid 32) on a Translated ring is therefore **never observed by kayfabe**: no
   log line, `dead=None` at retire (MEASURED in the census: both GR-tier rings `dead=None`, `gp_get ==
   last_put`), and the completion tail (semaphore release + `NON_STALL_INTERRUPT`) never executes, so the
   fence never completes. That is the intended no-forged-completion behaviour, but it is also why the last
   lease could not say when the work stopped. `KF3_COMPLETION_PROBE=<ms>` (`chan.rs:1860-1880`, dumps a
   Translated fence overdue by `<ms>` with the host ring's cursors and the decoded releases) exists, was
   **not enabled** in the last lease.
8. **CODE READ, the Xid number.** `intr 00200000` is bit 21. The repo's own notes name the neighbouring
   bits (DEVICE 0x00800000, PBENTRY 0x00040000) from `dev_pbdma.ref`; bit 21 is the PBDMA `METHOD`
   interrupt by the same table (INFERRED from the table order, `dev_pbdma.ref` not re-read tonight). A
   PBDMA-level METHOD error says the host front-end rejected a method or subchannel binding on a channel
   QEMU owns; it does not name the method.

# 2. Is the Xid32 a product defect or a consequence of the crash?

**Verdict: not a consequence of the 0x116 bugcheck (MEASURED: it is 18.5 s earlier, and 5.3 s before the
guest's own recovery starts). Probably a product-side event, contained to QEMU's own channels.**
Evidence for "product-side": the pair appears only in the two revisions that forward Blackwell GR methods
to the host twin (fix4, af8) and in none of six Code-0 arms that kill that channel inside kayfabe instead
(item 2); the code already documents Xid32 on QEMU's own twins as the expected hardware answer to a
forwarded method or object the twin cannot take (OWNER_RULINGS section U, V_DISPLAY, chan.rs:1698). Evidence
against / unknown: n=2, one board each; channel identity is unknown (the harness prints host handles at
birth, the Xid prints a hardware id; no map between them), so which two rings died is **INFERRED** to be the
two GR-tier kernel rings (tokens 0x3 and 0x10, the only rings that forward GR methods) and could be a CE
ring; the method is unknown. A hostile guest cannot turn this into more than an RC of its own VM's
channels (the rulings' containment argument), but it is still a defect: kayfabe forwarded something the
GPU refused and then said nothing.

# 3. Ranked hypotheses

Falsifiers are stated here, before any run (part 7 records what was run).

## (a) Display ON: guest bugcheck 0x50 about 11 s after enable, after the 4th cursor allocation

| # | hypothesis | for | against | cheapest decisive experiment | falsifier (written first) |
|---|---|---|---|---|---|
| A1 | **Incoherent display caps.** Serving POSTCOMP `CAPB` (output LUT capacity) opens a driver path that dereferences state initialised from the dependent fields that kayfabe leaves zero or partial (POSTCOMP `CAPC..G`, PRECOMP `CAPC/E/F` and the low half of `CAPA`, tiles 4-7). | MEASURED ladder (item 5): `CAPB` present + PRECOMP not the die's -> 0x50 in two differently served pages; both words the die's -> Code 0; `CAPB` absent -> Code 43. MEASURED same fault signature (item 3) with two different `CAPB` values, so it is the presence of the path, not a value. | n=1 per page; the oracle shows 125 other words differ and only the 0x680-0x87c ones were bisected. | **One binary, one env var:** ON with the derived page (PR65 candidate, see A2) versus the same page with ONLY the die's PRECOMP words (0x780-0x87c) from this box's own read through `KF3_DIAG_CAPS_PAGE_FILE` (diag branch, never shipped). | 0x50 persists with the same low-16-bit P3 / low-24-bit P1 signature while PRECOMP is the die's. |
| A2 | **Missing width contract.** POSTCOMP `CAPA` lacks `FULL_WIDTH/UNIT_WIDTH`; the driver divides or shifts by it (Codex's PR65 `296636c66` derives the unit width from public NVKMS). | CODE READ: public NVKMS asserts the unit width on this family; served `CAPA=0x007b0000` has the low half zero where the die has it set (item 4). Fix5 (`CAPA` without widths) gave Code 43, af8 (plus `CAPB`) gives 0x50: consistent with widths being needed but not sufficient. | GPU-free tested only; widths are the least-informed of the missing bits. | The PR65 candidate in the ON arm, no other change. | PR65 candidate still 0x50 with the same signature. |
| A3 | Display memory the driver maps after the 4th cursor allocation is not backed (a mapping kayfabe must provide). | the crash is guest-VA, immediately after the last reply. | MEASURED: both pages that reach it bugcheck at identical offsets irrespective of what is mapped; Code 0 is reached with a native-equivalent page and the same mappings. P1 is a guest VA (not a host fault). | module + RVA of P3 from a fresh dump (loader facts only, part 6), then BAR0 display access trace across the window (no trapped access seen in `trace_on_1`, untrapped accesses invisible). | P3 resolves into a Microsoft module, or the BAR0 trace shows display access the host did not serve. |
| A4 | An unobserved GSP reply or event (quiet events, CA7A reply params 16 B) is wrong. | `current_blackwell_differential` cannot falsify it; selective logs are not a journal. | Successful headers; a wrong reply would not give a fixed code offset across different caps pages. | Export the four CA7A reply param words (public struct) with the capture kit and compare with the VFIO reference. | The words equal the reference. |

Ranking reason: A1 and A2 are testable with one binary in two ON arms and carry the measured ladder; A3/A4
need new capture before they can be decisive.

## (b) Display OFF: Code 0, then ~40 s class TDR (here 141 reports, 0x116 at +22 s)

| # | hypothesis | for | against | cheapest decisive experiment | falsifier |
|---|---|---|---|---|---|
| B1 | **A forwarded method is rejected by the host PBDMA (Xid32 METHOD) on one or both GR-tier rings; the rings are RC'd; kayfabe never notices; the guest's GR work never completes; Windows times out (141), tries `RESET_CHANNEL` (refused 0x56), resets the adapter.** | item 1 timeline (Xid first, ~3 s of silence, recovery 5 s later); item 2 (Xid only with fix4 admission); item 7 (RC invisible to kayfabe); CODE READ + docs (G17/H-ESCALATE: a refused `RESET_CHANNEL` escalates to an adapter reset, kept refused, owner's decision). | n=2; ring identity and method unknown; the 141 time is unknown (the first 141 could precede the Xid and Xid32 be a reaction to guest recovery). | One OFF arm with: `KF3_COMPLETION_PROBE=500` (earliest overdue fence, host clock), a per-ring forwarded-method tail and host chid at birth, host kernel journal with timestamps, key-line byte offsets for time mapping. | The first overdue fence / first 141 evidence is **earlier than 27.2 s**, or the two RC'd rings' last methods contain no GR method the admission added (then the Xid32 is a reaction, B2 stands alone). |
| B2 | **The terminal 0x116 is independent of what hung: after the guest unloads its driver, no re-init ever arrives, and the reset path fails with the same status.** | MEASURED: identical P3 `0xC000009A`, P4 4, no RPC after `UnloadingGuestDriver`, in every Code-0 arm over four revisions, two boards, with the GR ring dead by kayfabe's own refusal (fix2/3) or by host RC (fix4/af8). Owner item "driver unload/reload" is listed as pending. | It does not say why the first TDR started; a recoverable TDR needs the reload to work. | Read the host side after `UnloadingGuestDriver` (GSP cycle log lines, queue binding: the tail shows `QueueNotBound` x2) and compare with the Ada Windows runs where the same TDR recovers (GSP cycle then reload). | An OFF arm where the first TDR recovers (4101) on GB202, or a GSP cycle line on GB202 after the unload. |
| B3 | Engine other than GR hung (CE ring `0x80f`: 21088 methods, 1385 submissions, 129/129 splits; or paging/copy). | the CE ring carried the bulk of the traffic; optional CE95 branch unrun. | census: `gp_get == last_put` on every ring (not a completion proof: census is after teardown). | `KF3_COMPLETION_PROBE` names the ring and fence that went overdue first. | The first overdue fence is a GR ring (B1) or none is overdue at all. |
| B4 | A reply or event dependency after Code 0 (`0x20809064` and `CBCB` refused with 0x56; quiet events). | Codex's list; CODE READ differential shows both refusals present in OFF runs that reach Code 0. | MEASURED: Code 0 reached with both refused; both refused in the VFIO-aligned native comparison only at the first difference. | none cheap; keep as residual. | -- |

# 4. Capture gaps that wasted the last lease, and the fix for each

| gap | consequence | fix for the next lease |
|---|---|---|
| Raw dumps, images, binaries disposed with the lease | no analysis possible | copy the **small minidump + the module list + versions** (text) off before teardown; keep the 1 GB live dump only on the rental until the analysis step is done, export its header (bugcheck P1-P4 **and** SystemTime; the exporter read only the code and time) |
| First 141 time/engine unknown; WER times are post-reboot | cannot order Xid and 141 | host side: `KF3_COMPLETION_PROBE=500` gives the earliest overdue fence on the host clock; guest side: `LiveKernelReports` listing with NTFS creation times, System log (4101, 41, 1001, 141 sources), and, only if the owner agrees (D3), an ETW trace of the Microsoft DxgKrnl provider with a 1 s flush timer |
| No ordinal-to-guest-time correlation; QEMU log lines have no timestamps | the Xid could be placed between ordinals only by luck | export key log lines with their **byte offsets** (grep -b) and map them to time through `rpc_timing.jsonl` `file_offset_after`; record guest `Get-Date -Format o` + host `date` at enable and again at collection (clock offset) |
| Ring identity / method unknown | Xid32 channels cannot be named | print the host chid next to the host handle at birth; keep the last 32 method headers per Translated ring and dump them at retire and in `PROBE-DUMP`, default off (code task, test first, below) |
| Host Xid text only counted | the count (`host_xid=2`) in fix4 was never analysed | save `journalctl -k -o short-iso-precise` slices per arm, with monotonic clock |
| Symbol path | the debugger refused the wrong kernel symbols; no diagnosis | part 6 |
| `PROGRESS_STRAND` covers only USER relays | zero lines says nothing about Translated rings | use the overdue-fence probe, not the strand diagnostic |

Code tasks (GPU-free, test first, **not started** unless part 7 says so): (C1) method tail per Translated
ring + host chid in the BORN line, default off; (C2) extend the Xid guard with ONE exact expected exception
for the OFF arm (QEMU pid, code 32, intr `00200000`, at most 2, after Code 0) so the reproduction target does
not abort the lease; (C3) cherry-pick `0752115a7` (`KF3_DIAG_CAPS_PAGE_FILE`, applies cleanly on this base,
28 lines) as a clearly labelled DIAGNOSTIC commit that is never merged.

# 5. Run plan, one fresh GB202 lease

Authorisation (coordinator message relaying the owner, 2026-10-21 evening): one fresh GB202 lease under
`scripts/bench/box/README.md`, lane budget hard stop **6 USD**, account credit ~21.4 USD, reserve **5 USD
never touched**; max 3 h per box; auto-destroy armed before touching the box; own ids only; evidence into
git, big logs on the dev box; no second lease while one exists; abort on any new unmatched Xid.

Cost model (observed 1.25 USD/h on the last GB202 lease: runtime estimate 2.871614 USD for 2.3 h, account
delta 2.714426 USD): setup 65 min (create/boot 5, native health 20, Windows install 26 overlapping the
kf3 build, bind 5, parent hash 3), arms 4 x ~12 min incl. collection boot ~50 min, export + teardown 10
min -> ~2.2 h -> **~2.8 USD**. Proposed cap for this lane: **stop 4.0 USD** (3.2 h at 1.25), the
coordinator's 6 USD hard stop stays; auto-destroy at 180 min.

Arms in order (one variable each; riskiest last; every ON arm ends by itself at the bugcheck):

| arm | rev / flags | variable | expected | falsifier (before) | decision |
|---|---|---|---|---|---|
| 0 | native health: unprivileged die probe, gates 9/9+10, raw 30/30, CUDA 4/4 | -- | PASS | any refusal -> stop, no guest | stop |
| 1 `on_a` | R = frozen af8 + PR65 (`296636c66`, 2 commits) + diag hook, hook unset, P19 + `KF3_RPC_TRACE/BODY_TRACE`, capture kit | PR65 vs the n=1 af8 0x50 | A2 predicts Code 43 / Code 0 / a different bugcheck | 0x50 with P3 low 16 bits `a1b0` | A2 falsified -> arm 2; else go to OFF behaviour of an ON that reaches Code 0 |
| 2 `on_b` | same binary, `KF3_DIAG_CAPS_PAGE_FILE` = derived page with ONLY 0x780-0x87c from the box's own read | caps source of PRECOMP | A1 predicts no 0x50 | 0x50 persists, same signature | A1 falsified -> A3/A4 capture (dump module + RVA) |
| 3 `off_c` | same binary, display off (empty ROM, as `bw_off_m1`), P19 + RPC trace + `KF3_COMPLETION_PROBE=500` + C1 if landed | measurement only | B1: first overdue fence before 27.2 s-equivalent, Xid32 pair, 0x116 | first overdue fence after the Xid, or no GR method in the dead rings' tails | B1 stands/falls; B2 read from the GSP log |
| 4 (conditional) | the arm 2 or arm 1 outcome decides; only if it moves ON past 0x50 | -- | -- | -- | -- |

The OFF arm is last because it is the one that may produce Xid32; a quick native health re-check (gate 9
subset, 2 min) follows it. The expected Xid32 pair is a *reproduction target*, not a licence: **abort rule
unchanged except for ONE exact exception, to be implemented in the guard (C2) before the lease**: in arm 3
only, after Code 0, at most two Xid32 from the QEMU pid with intr `00200000`; any third, any other code,
any other pid, any Xid in arms 0-2 stops the lease. If C2 does not land and pass its tests, arm 3 is not run
(report instead). Source-matched intentional fault controls remain the only accepted baseline.

Save BEFORE teardown (small, text/PNG, into git under `traces/blackwell_resume_20261021/`): per arm
`api.tsv`, `rpc_timing.jsonl`, `channel_state.tsv`, key-line byte offsets, `PROBE-DUMP` lines, host Xid
slices (iso + monotonic), `OUTCOME.json` with both clocks, the four CA7A reply param words, bugcheck P1-P4 +
SystemTime of every dump, `LiveKernelReports` listing with times, System/Application events (4101, 41,
1001, 141, WER), minidump sha256 + module list (name, base, size, timestamp, version), screenshot PNG.
On the dev box only (not git): `abort_context`, raw logs, minidump, live dump headers. Teardown by id with
verified API absence, then write the lease into `traces/die_driver_sweep_20261015/COST.md`.

# 5a. Owner rulings relayed during the session (2026-10-21 evening) and what they change

1. **Blackwell is the priority; the card does not matter; constants follow the table.** Anything this lane
   would ship must come from the per-die constant table's tier order (`kf_rm::consttab`; public ogkm, then
   host-derived, then the VFIO reference), never from a captured page. The `KF3_DIAG_CAPS_PAGE_FILE` arm
   (A1) reads this box's own die and therefore stays a **diagnostic on a separate, never-merged branch**
   (`claude/blackwell-resume-diag-20261021`); it can only say *which* words matter, and the fix would then
   be re-derived from public headers. Nothing in that file is copied into the repository.
2. **Black box first, time-boxed.** Time-box: **this lease's arms 1-3 (about 50 minutes of arms) plus the
   GPU-free read of their output (about 60 minutes) are the whole black-box phase.** If they do not
   locate the cause, I stop and message the coordinator **before** any reverse engineering so the setup can be
   confirmed. The rental copies needed for that (small minidump, live-dump header, the guest's own
   `nvlddmkm.sys` and Microsoft images) go to a directory outside every git tree on the dev box
   (`/root/kf-private-data/blackwell_20261021/`), never to the public repository. Under the owner's rules
   RE would be for diagnosis only, notes and decompiles only in `/root/kayfabe-private-clone`, nothing
   derived from it becomes a constant or code value, and this public memo would say only "diagnosed with
   private tooling". **No RE has been done and none is started by this plan.** (Section 6's D1/D2/D4 are
   superseded by items 2 and 5 for the dump-copy question; the Microsoft-symbol part remains as written.)
3. **Budget.** Lane stop stays 6 USD until a first lease shows progress; then this memo / the final message
   asks for the next cap instead of stopping silently. (Part 5's 4.0 USD proposal is inside it.)
4. **Notifier 196 (GC6 reduced-threshold, PR56, section S.1): APPROVED as the silent stub** (owner,
   same evening: power is managed by the host driver and GSP; the guest does not manage it; the system stays
   unprivileged; monitoring is never a reason to allow a privileged verb; S.3 power events matter only if
   needed for correctness, which is unlikely). The plan therefore assumes the stub is in; no event is posted,
   no host action. Recorded for the record: without it the display-OFF path stops at Code 43 (MEASURED, fix1
   arms, n=3), and display-ON never reaches it (ordinal 275 versus 416).
5. **RE scope (binding).** Any inspection of NVIDIA's non-open code, compiled or decompiled (disassembly,
   Ghidra, decompile output, dump disassembly), is RE and follows the RE rules. Traces (interfaces, ordinals,
   RPC and event logs, behaviour) are not RE. Consequence for this plan: the capture kit reads only dump
   **header** fields defined by Microsoft's public structure (bugcheck code, parameters, SystemTime), WER
   fields, event logs and file times; it does not read module lists, code bytes or stacks. Resolving P3 to a
   module and RVA is held back as a step to confirm with the coordinator, together with the private-clone
   setup (read `/root/kayfabe-private-clone` first, do not duplicate its earlier work). Nothing is done on
   that side in this memo.

# 6. Symbols: what can and cannot be analysed (owner decisions listed, nothing assumed)

Public Microsoft symbols for `ntoskrnl` and `dxgkrnl` are fine. The earlier CDB run failed because it had the
wrong kernel symbols (`nt_wrong_symbols`), and the PDB-only proxy refused image downloads.

**Can be analysed without NVIDIA private symbols (all loader / Microsoft facts):** the bugcheck code and
parameters decoded by Microsoft's public documentation; the module that contains the faulting instruction
and its RVA (module list in the dump: name, base, size, timestamp); whether the fault is in a Microsoft
module; Microsoft-side stack frames (`nt`, `dxgkrnl`) with public symbols, and the dump's `!analyze` bucket
for 0x116/141; times (dump header, `LiveKernelReports`, event logs); equality of RVA between runs.
**Cannot:** names, types or meaning of anything inside `nvlddmkm` (NVIDIA publishes no PDB), which GPU
engine hung, what the dereferenced structure is, whether it is a table we should populate, any value we
could "derive". Those need either more differential experiments (this plan) or disassembly, which the
repository's provenance policy confines to narrow, firewalled RE in the private repository with nothing
carried over.

Owner decisions needed:
- **D1** local-image public-symbol method (Codex's `analyze_dumps_local_images_PROPOSED.ps1`: `-i
  System32;drivers`, images already in the same guest, all proxy restrictions kept, wrong-symbol rejection):
  approve or refuse. This plan runs without it; if refused the dumps are only measured for module + RVA by a
  loader-fact read.
- **D2** may the Microsoft images from the guest (`ntoskrnl.exe`, `dxgkrnl.sys`) and a small minidump be
  copied to the dev box for the same analysis (never to git)? `nvlddmkm.sys` stays on the rental.
- **D3** guest ETW for the DxgKrnl TDR events (1 s flush) in one extra arm: the standing preference is
  trace-first comparison before guest tracing; the aligned comparison is done, the first-141 time is still
  missing, and the host overdue-fence probe may already give it. Not used unless approved.
- **D4** disassembly of the faulting function: not requested, listed only so it is not assumed.
- **D5** the single Xid32 exception for arm 3 (above), and the new lease cap (4.0 USD stop).

# 7. Execution record

(see the bottom of this file, appended as the lease proceeds)
