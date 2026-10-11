# Windows production defaults and manual session

**STATUS: RESEARCH, 2026-10-11.** Owner requests required boot behavior in code defaults, no measurement knobs and no automatic input in the owner session. This overrides the older D4 timing dependency on a complete TDR fix; derive-never-capture still applies.

First promotion: native PTIMER mapping without KF3_TIMER_MAP. Source: unprivileged host TIMER_GET_REGISTER_OFFSET and independently compiler-derived exact host/guest timer layouts. The existing read-only4KiB mapping is unchanged. Missing/mismatched layouts or invalid spans explicitly report timer unsupported and retain no mapping/advertised native offset; they no longer fail device realize. No captured constant is introduced. Prior ablation604 demonstrated Code43 when the mapping was absent. GPU-free validation saved alongside; exact-revision Linux and Windows validation pending.

Other production flags remain under audit. Captured display caps, GfxP placeholder sizes and status-only control echoes are not promoted by merely flipping an environment default. No zero-flag stock-Windows boot claim yet.

Owner session plan: a fresh run1380 under the shared GPU lock after finite matrix/diagnostic jobs, stock guest580.88, production settings only, measurement flags absent, manual sign-in. QGA may set the local account password and disable autologon/MPO, but no input injection, scripted login, app launch or repeated reboot. Hold the lock for the lifetime of the owner VM. Credentials stay on the trusted host and are never committed.
