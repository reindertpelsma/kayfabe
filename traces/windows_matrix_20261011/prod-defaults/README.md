# Windows production defaults and manual session

**STATUS: RESEARCH, 2026-10-11.** Owner requests required boot behavior in code defaults, no measurement knobs and no automatic input in the owner session. This overrides the older D4 timing dependency on a complete TDR fix; derive-never-capture still applies.

Measured update20:19UTC: exact3252a087 Linux gates9/9+gate10 and fast30/30
PASS; timer-flag-free Windows1390 gears/vectorAdd/CUP8 allPASS, no recorded
guest events/GSP recovery or precise-window hostXid. This supersedes the
hardware-pending sentence below. Evidence: linux-result/ and focus-result/.

The caps-page ablation1400 fails setup: NVIDIA Code43, zero NVIDIA DXGI
adapters, no app execution. This removes KF3_DISPLAY_CAPS_PROBE (the captured
PDISP MMIO page), separately from the system GET_CAPS_V2 feature bitmask.
The second guest1401 also fails sign-in; remaining attempts were explicitly
interrupted after the first falsifier, not retried until success. No second
app verdict is claimed. Guest1401 required an unclean stop after the normal
shutdown attempts; precise-window hostXid0, DMA-FQ restored. Evidence:
caps-ablation-result/. No captured caps default was promoted.

Fresh normal production control1460 also passes all three apps, zero recorded
guest events/GSP recovery and precise-window hostXid0; clean exit20:17:13UTC.
Evidence: control-result/. Manual1380 READY20:17:45UTC after the finite native
diagnostic: adapter Code0, autologon disabled, no input or apps started.
Credentials were supplied to the owner privately and are never committed.
The owner session keeps19 remaining behavior flags, with measurement knobs off;
this is not yet zero-flag stock Windows. A separate finite native lifetime/RC
diagnostic precedes manual VM creation, under the same lock, and never runs
after the owner session becomes ready.

First promotion: native PTIMER mapping without KF3_TIMER_MAP. Source: unprivileged host TIMER_GET_REGISTER_OFFSET and independently compiler-derived exact host/guest timer layouts. The existing read-only4KiB mapping is unchanged. Missing/mismatched layouts or invalid spans explicitly report timer unsupported and retain no mapping/advertised native offset; they no longer fail device realize. No captured constant is introduced. Prior ablation604 demonstrated Code43 when the mapping was absent. GPU-free validation saved alongside; exact-revision Linux and Windows validation pending.

Other production flags remain under audit. Captured display caps, GfxP placeholder sizes and status-only control echoes are not promoted by merely flipping an environment default. No zero-flag stock-Windows boot claim yet.

Owner session plan: a fresh run1380 under the shared GPU lock after finite matrix/diagnostic jobs, stock guest580.88, production settings only, measurement flags absent, manual sign-in. QGA may set the local account password and disable autologon/MPO, but no input injection, scripted login, app launch or repeated reboot. Hold the lock for the lifetime of the owner VM. Credentials stay on the trusted host and are never committed.

Campaign queued32315: exact3252a087 Linux gates9+10/fast30, timer-flag-free smokes1390, single-variable caps ablation1400, fresh original production control1460, then manual1380. Controls must be clean before manual approval marker is written. No personal session is started by the matrix harness.
