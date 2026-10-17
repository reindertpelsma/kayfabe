# GB206: one derived tiling group moves Linux mode validation, presentation remains red

**STATUS: LIVE, 2026-10-16 20:58UTC.** B2 one-group experiment DONE with a partial result;
Windows rank-1 probe remains FAIL. No desktop or family acceptance.

MEASURED on only own instance56286918 (GB206/PCI2d04,8151MiB), native595.91.07.
Both Linux guests used fresh sibling overlays of the same prepared580.159.04 parent,
8vCPU/8GiB RAM/4GiB virtual FB, same X11 SHM broker. Actual process receipts are adjacent.
The baseline f8485f6dbe2cdec62c3b1fa69133b5f266801dcc binary SHA256 is
`e9744a23ab38ff45afbf788d9e426d71913d1f1e6954fa1ea0da1ea1920b25a6`.
The derived tiling candidate36d134bc578b052072718573cff19fa2cb08388b SHA256 is
`f3175f77e1ed9e33a762b2e22c21b4056e931483ef4d448b3efb98721d630e82`.
Candidate native gates9/9+gate10 PASS,12/12 USER births, before the private build.

| Die | OS / actual build | Actual flags | Result | Evidence |
|---|---|---|---|---|
|GB206|Linux580.159.04 baseline|KF3_REV=f8485f6d; KF3_RPC_TRACE=1|FAIL: No valid modes; Cinnamon0/Xorg0|gb206phase_broker_baseline_guest.log|
|GB206|Linux580.159.04 tiling group|KF3_REV=36d134bc; KF3_RPC_TRACE=1|FAIL presentation: mode validation passes, Cinnamon1/Xorg1, black SHM frame, flip timeout|gb206phase_broker_tiles_guest.log; gb206phase_broker_tiles.log|
|GB206|Windows580.88 /32.0.15.8088, GSP1|19 Windows flags + RPC_TRACE1/BODY_TRACE1, all21 printed in gb206phase-tiles-flags.txt|FAIL Code43;385 metadata commands; teardown starts277, explicit unload384 unchanged|gb206phase-tiles-census.log; api-full.tsv; process-receipt-106606.json|

MEASURED Linux candidate: NVIDIA OpenGL renderer; no "No valid modes"; glxgears3 frames/15.1s
(0.198FPS); SHM capture1920x1080,1 color (black), pixel SHA256
`788ae0147bdf979a6575938ca2d7d4403788588f7be2010f03776c968fd1ab49`.
Guest dmesg: baseline34 nvAssert/0 flip timeouts/0 Xid; candidate12 nvAssert/1 flip timeout/0 Xid.
`vkcube` rc127 is a missing test utility, not a measured Vulkan driver failure.
Broker received299 attach/299 commit/0 rejected; receiving commits is insufficient for a nonblank-frame pass.
Host Xid count remains5, all previously attributed to raw intentional-fault arms; no new Xid.
Both owned Linux VMs shut down cleanly; Windows owned-stop completes20:47:44UTC.

FALSIFIER stated before the run in gb206phase_candidate.log: modes remain invalid with this group.
MEASURED falsifier did NOT fire for Linux mode validation; full B2 desktop acceptance still fails.
MEASURED Windows rank-1 unchanged-unload falsifier FIRED: lastCA7A276, teardown277, unload384.
This candidate did not fix Windows init. The preceding on/off comparison already starts teardown277
with no display classes in OFF; numeric equality across different paths is not proof of one common cause.
INFERRED: contiguous virtual tile capability plus required IMP assignment was a Linux mode-validation
prerequisite. The black-frame/flip wall requires a separate investigation; stop after this one group.

Preparation negatives are preserved: PPA wayland-protocols1.34 HTTP503 twice before GPU work;
private benchmark recipe pins official Jammy1.25-1, retries3, builds the intended X11 broker.
No captured caps-page injection and no second capability group were attempted.
Window diagnostics export44 public control bodies and21 structured display allocations, with48 opaque
bodies omitted on the rental; pointer values are redacted. Metadata effective status remains UNKNOWN
because the policy logger precedes held actions. The Linux baseline QEMU log exceeds1MiB: only
screened excerpt, raw SHA/size manifest returned. Images/executables/raw opaque Windows logs stay on rental.

Next ranked Windows probe: public pure-[In] SEND_CMC_LIBOS_BUFFER_INFO(a89), one change,
falsifier unchanged teardown boundary. Public Linux kernel_gsplite.c already treats NOT_SUPPORTED
as nonfatal, which weakens the hypothesis for Linux and does not establish closed Windows behavior.
CBCB remains NOT DERIVABLE (native120-byte replies,43 distinct); CCU guesses remain prohibited.
PR29 stays draft. One active own instance only; timer22:06:28UTC/hard22:11:25UTC; current
lease maximum2.17USD, campaign ceiling9.23USD plus unbilled transfers.
