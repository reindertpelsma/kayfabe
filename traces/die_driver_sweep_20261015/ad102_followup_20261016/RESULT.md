# AD102 missing lanes and passing Windows display reference

**STATUS: LIVE, 2026-10-16 16:15 UTC. Guest results pending.**

Owned instance56267214, verified machine43132. MEASURED: actual146GiB root,
30logical CPUs,24389MiB RAM and KVM. Unprivileged UID65534/CapEff0 RM probe
identifies arch0x190/impl2 AD102 and stock Linux595.91.07. Native gates9/9,
gate10 PASS and12/12USER births at16:10:50UTC. Private source2feb22a9f built
at16:11:25UTC. The native595 caps page is an oracle only.

| Die | OS / lane | Actual driver | Flags | Verdict | Evidence |
|---|---|---|---|---|---|
|AD102|native Linux gates|595.91.07|gate harness|PASS9/9+gate10|adaref2_gates.log|
|AD102|Linux guest broker|595.91.07 planned|pending actual process receipt|UNRUN|pending|
|AD102|Windows guest short / display trace|580.88 planned|production19 plus bounded metadata/body diagnostics|UNRUN|pending|
|AD102|Windows guest seven-minute hold|580.88 planned|production19, diagnostics off|UNRUN|pending|

The Windows diagnostic boot will collect an ordered passing connected-display
reference for prompt section2. Native GB206 was headless and cannot supply that
reference. Hypothesis H-display-reference (INFERRED): presence of73010c and
c3700201 in the native sequence may identify missing display initialization.
Falsifier, stated before this run: a passing Ada guest also skips these controls;
then their presence in native alone cannot identify the Blackwell abort.
Also report whether a89,ab2,ab4,cbcb,2801 and2806 are served or never issued.

The diagnostic short suite and production hold use separate QEMU processes.
Each must assert the actual source, private binary SHA, host595, guest580 public
ABI580.65.06, actual Windows32.0.15.8088 and every process KF3 variable. The hold
requires Code0, DWM/Explorer, hardware D3D11+12, at least420seconds, zero readable
System4101 events, before/after CUDA cup2/3/8 and nonblank screenshot receipts.
Screenshots and unscreened reply bodies stay on the rental. No new native Windows
installation is needed because the per-die reference already exists.

CPU harness correction: draft PR26 adds explicit KF_GUEST_DRIVER selection to
interactive.sh; its previous hardcoded580 ABI would make a595 disk's receipt
wrong. Failing-first assertions are saved; three test methods now pass. This
does not itself validate a hardware broker or change VM shutdown behavior.

No die family is complete. Timer18:58:22UTC; hard cap19:03:21UTC. Cost ledger
includes the full maximum1.83USD for this lease, campaign ceiling7.20USD plus
unbilled transfers. Only this owned ID may be retired.
