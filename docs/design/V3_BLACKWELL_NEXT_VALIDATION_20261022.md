**STATUS: LIVE, 2026-10-22.** Owner GO authorizes ONE RTX 5090 lease with the tested guard and N0 gate.
The earlier documentation-only restriction below is superseded by that explicit GO. Native Xid32 stays fatal.

Owner follow-up supersedes the old B1 readiness/no-merge paragraph below **only for an isolated
optional arm**: tested B1 `1c6452a276d7014bece3adf42d68cbd4edffbc18` (runtime `d8c1e65ef`)
may be merged with frozen primary `8f3a9bb3a` on a separate own branch/build; record both inputs,
merge revision and binary hash. It stays after CAPC ON and OFF/TDR, within the unchanged timer.
The B1 CI-repair parent is a comparison confounder; no RC-only causality claim. Hardware n=0.
If preparation or the arm cannot fit, it is the first arm for a later authorized lease.
The current lease's N0 passed; bind reached disabled Code22/580.88, then its immediate shutdown
census failed. No Windows arm may run while the required decision remains pending.

Original future-run plan, now authorized subject to its guard prerequisites: The latest owner
task authorizes documentation/derivation only: **no hardware and no rental now**. The plan becomes
executable only after later authorization and the prerequisites below; budget availability is not permission.

First finish the exact native-source guard and the allocated-CE-class fixture topic described in
[V3_BLACKWELL_GUARD_REVIEW_20261022.md](V3_BLACKWELL_GUARD_REVIEW_20261022.md), with test-first
commits, at least two killers per mutant, complete touched-crate release counts, no added clippy
diagnostics and rustfmt. Parked patches are not ready for deployment. Native causal sufficiency
will then be tested by N0; do not suppress a remaining native Xid 32 to reach Windows.
Pin the source/binary, guard and controller hashes before renting. Use a product build with the
public CAPC topic from draft [PR #69](https://github.com/reindertpelsma/kayfabe/pull/69), without
`KF3_DIAG_CAPS_PAGE_FILE` or any captured caps page. Freeze the chosen integration revision and
all applied topic SHAs in the run receipt; no unrelated product change between paired arms.

| Order / arm | Question and expected outcome | Falsifier / decision |
|---|---|---|
| N0: host 595.91.07, native health/gates 9/9 +10, raw 30/30, CUDA 4/4, corrected GR/CE fixture | The allocated CE class removes the unmatched native HCE fault; exact intentional source-marked controls and recovery are the only allowed baseline faults | Any unmatched fault, including the same native Xid 32, or suite failure: export, stop and destroy; no Windows arm |
| A1 `on_precision`: Windows LTSC 580.88, derived precision only, display/GOP ON | Expect progression beyond the fourth CA7A allocation without 0x50; capture actual CIM Code 0 separately from RPC progress | Same 0x50 around ordinal 277: precision alone insufficient; capture and stop this hypothesis, no wall walk or captured-word fallback |
| A2 `off_c`: same revision, display=off,gop=off,gop-efi=, completion probe=500 | Reproduce the previously measured Code 0 -> exact bounded Xid pair -> RESET_CHANNEL refusal/unload/0x116, or demonstrate that this path changed | No Code 0 or a changed fault sequence disproves equivalent reproduction; any out-of-scope Xid stops the whole lane |
| Optional A3: separate binary with a ready, GPU-free-tested B1 RC fix | If A1/A2 reached the TDR path, test whether that one fix removes the same first fault/recovery failure | Same failure sequence disproves sufficiency; no fix-to-fix iteration on this lease |

A1 takes priority. A2 remains useful if A1 still fails 0x50 because it isolates the known display-off
TDR path. At most two attempts per red cell, with a written reason before retry; total time/cost limits
can end the run sooner. No fresh diagnostic caps-page arm or new native trace capture is planned.

A Code 0 observation alone is not a Windows PASS. For any arm surviving initialization, require a
600 s zero-TDR hold, usable desktop where display is ON, CUDA and DX11/DX12 checks, and healthy host
journal after each workload. If an allowed Xid pair occurs, stop workloads, retain the bounded recovery
sequence/dump-header/event metadata and mark FAIL; the exception permits diagnosis, not acceptance.
Capture zero-loss canonical RPC ordinals (including Other(4)), reply status/size, System events, CIM
state, QMP panic data and timestamped probe records. Preserve both host/guest clocks without applying
a recovery-boot clock offset to the crashing boot. Map host physical channels to owned RM handles and
ring identities in fresh host-authored receipts; do not infer them from numeric debug words.
The old OFF probe's zero overdue samples did not identify or exonerate the failing ring.

Keep the printed 19-flag set from
[the retained process receipt](../../traces/blackwell_resume_cont_20261022/arms/on_c3/arm_on_c3_process_receipt.json):
KERNEL_GR_WORK, GFX_POOL_PROBE, TRANSLATED_CE_RELAY, DISPLAY_CTRL_PROBE, MEMORY_LIST_PROBE,
WIN_USER_CHANNELS_PASSTHROUGH, SW_RUNLIST_PROBE, ZCULL_BIND_PROBE, DEFERRED_API,
WIN_TWIN_DEFAPI_OBJECT, DISPLAY_ILUT_OFFSET_256, SW_RUNLIST_HOST_OWNED, DISPLAY_SDR_COLOR,
KERNEL_GR_CE, DISPLAY_LUT_MIRROR, ASYNC_PREEMPT, DISPLAY_IMP_ENABLE, PREEMPT_BIND_PROBE,
DISPLAY_CAPS_PROBE (each `KF3_` prefixed, value 1). Add RPC_TRACE=1, RPC_BODY_TRACE=1 and
DIAG_PROGRESS=1000, with the same prefix, and record any bounded completion-probe setting explicitly.
The compatibility profile label `guest-driver=580.65.06` in QOM is not proof of the installed Windows
driver: verify staged driver files and CIM report 580.88 before enabling, using a fresh sibling overlay.
The approved display-off arm omits the GOP ROM property; retain that guard.

B1 readiness is checked before any future lease. A fresh read-only remote check during this review
found `codex/translated-rc-20261022` at `82effe99f18c27c55a3326e6e95fd375e43098c7`.
It contains design/seam and EXPECTED RED tests, followed by a merge of the master CI-repair baseline;
there is no completed RC implementation/verification receipt at that revision. It is not yet an arm.
Re-check readiness later; do not merge its branch. If ready, pin and build a separate binary before
timing any guest, and keep unrelated baseline changes out of the paired comparison.
The older optional CE2 ring branch `claude/ce2-kernel-ring-20261019` / `95e23df77` also remains unmerged;
select at most one optional cause/fix arm after review, never combine fixes in the same comparison.

Provisioning evidence and cheaper critical path:

| Retained marker, lease 57117614 | UTC on 2026-10-21 | Elapsed |
|---|---|---|
| Created / SSH ready | 22:04:32 / 22:06:11 | ~1m39s readiness |
| Controller starts / build ready | 22:06:14 / 22:19:13 | 12m59s |
| Native baseline complete | 22:20:17 | 1m04s after build; GR fixture follows |
| Install/check/bind ready | 22:51:00 | 30m43s after native marker |
| Creation to bind ready | — | 46m28s |

Sources: retained [provision](../../traces/blackwell_resume_cont_20261022/lease_metadata/sweep_20261020_provision.log),
[native](../../traces/blackwell_resume_cont_20261022/lease_metadata/sweep_native_baseline.log) and lease receipts.
The reported ~65 minutes is a conservative allowance, not this lease's measured duration.
The largest measured block combines media, LTSC install/check and binding. Its detailed substage logs
were not retained, so attributing the entire block to media download or LTSC installation is unproven.

Start checksum-pinned public media fetching alongside CPU preparation/build instead of after native
fixtures. Reuse only a hash-verified public media cache if it actually exists on the new box; do not
assume a destroyed box's cache persists. `win_vm.sh install` uses standard VGA and accepts an explicit
WINVM_QEMU path, so a compatible distro QEMU could run the generic installation while the product builds.
That overlap is a CODE READ option, not a measured saving: first verify TPM, Secure Boot and networking
support using the unchanged harness; a guard refusal stops that option. Keep the sealed LTSC base,
unattended checks, actual 580.88 bind and normal driver/TDR settings. No smaller or stripped Windows
edition is proposed. Changing qcow2's maximum size alone does not establish an installation speedup.
Stop CPU builds and generic guest preparation before GPU baselines/workloads, preserving one GPU job
at a time and uncontended measurements. No NVIDIA enable/bind validation before N0 passes.

Add start/end/exit markers, monotonic durations and process-liveness checks for fetch, apt, build,
install boot, check boot and bind. Export their small text summaries. A 30–40 minute setup is an
unverified optimization target; budget 65 minutes until measured. Do not rent an idle box as a cache.

Cost and hard limits, if a future lease is authorized:

- Prior lane runtime estimate: **2.640452 USD**; latest owner hard stop **7 USD total**, with account
  credit reserve **5 USD**. Before creation, read/record API credit and reserve transfer/disk overhead;
  account credit deltas are not exclusive settled invoices. Quote and observed actual rate must both fit.
- At **0.78 USD/h**, 65 minutes costs **0.845 USD**; 150 minutes costs **1.95 USD**; the full
  170-minute auto-destroy envelope costs **2.21 USD**. Projected lane total at 170 minutes is
  **4.850452 USD**, or **5.350452 USD** with a separate 0.50 USD non-runtime allowance. The allowance
  is a budget guard, not measured spend. Recompute if the actual rate is higher; do not assume this offer persists.
- Plan 65 min setup +15 min native guard/audit +25 min paired arms +20 min surviving-arm hold/apps
  +10 min optional arm +15 min collection/export/teardown =150 min. Target teardown by150 min;
  arm auto-destroy at170 min FIRST and leave at least25 min before any new arm; never exceed3 h.
- One lease at a time, own IDs only, no secrets/executables copied back, strict SSH pins. Push evidence
  after every arm and before teardown, destroy by exact own ID, verify API absence, then cancel own timer.
  On any fatal guard event, export available text and destroy immediately; no second lease or wall loops.

No instance is created by this plan. The trusted host, GPU, VMs and locks remain untouched.
