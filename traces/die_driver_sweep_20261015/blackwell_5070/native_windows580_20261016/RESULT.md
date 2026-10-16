**STATUS: LIVE, 2026-10-16 14:17 UTC.** Native Windows580 stock reference passes; GSP-enabled reference is running. Kayfabe Windows on this die is still UNRUN.

MEASURED GB205 (Linux unprivileged MC_GET_ARCH_INFO arch0x1b0/impl5, UID65534/CapEff0), actualPCI2f04, own rental56246613. Windows11 Enterprise LTSC2024 Evaluation build26100, actual NVIDIA580.88/32.0.15.8088. Public580.88 INF explicitly supports2f04 before installation. Windows Update transient591.86 and the initial Microsoft Basic Display Adapter are setup states, not extra suite builds or kayfabe failures.

| Native GB205 Windows580 check | Result | Evidence |
|---|---|---|
| Exact actual driver/Code0 |PASS|native-oracle-before.txt and native-oracle-after.txt|
| CUDA cup2/3/8 |PASS before and after hold|same oracle files|
| Hardware D3D11/12 |PASS before and after hold|same oracle files|
| Desktop/DWM/Explorer |PASS|native-hold.txt|
| Hold420.32s,14 D3D rounds, System4101TDR0 |PASS|native-hold.txt|
| Fixed-aperture caps read |PASS,127 nonzero words,0x00c=0x102|native-caps580-recovered.txt|
| Linux595 versus Windows580 complete4096-byte caps equality |PASS|native-caps-comparison.json|
| GSP-enabled stock reference |RUNNING; no pass claimed|separate mode/oracle output pending|
| Kayfabe Windows580 acceptance |UNRUN|not yet booted on this die|

The completed stock reference reports GSP Firmware Version N/A. Do not present it as a GSP-mode result. A separately declared falsifier tests enabling GSP on only the owned2f04 adapter: nonzero Code, absent expected GSP firmware after bounded restart, or stock CUDA/D3D failure. No GSP queue replay/body capture is being run before the virtual display bodies justify it (prompt section2).

The initial caps command was rejected as unknown option before any hardware read: queue collectorb773… lacked --display-caps. Its module was stopped, the failed output preserved, and the matching caps tools substituted: source32973658ae1600a133ddbe40e19478fa118b3d50, unsigned moduleSHA371744be56f471aca3a5e882bf08afc03525cc008525ef7cf44a41d81306e2af, collectorSHA766dd94a088decfbd76db18b1ad4c18bd819bfa4461797e9dfbfaae755449afd. The module is test-signed with a new disposable certificate generated on the rental, performs a bounded read-only aperture query, and was stopped before all workload validation. No private key or executable is returned from the rental.

Constant provenance: public headers do not name word0x00c and no unprivileged equivalent word query is exposed, so this uses the existing documented tier3 measurement path. DraftPR24 admits only the independently measured GB205580 and595 rows, test-first4a50ae1fe/e389ce3b5 and7f0bb98a0/5b8dd3a5f;192 kf-disp tests pass,2 ignored. No wildcard branch, neighboring die, captured page or invented value is shipped. The row topic is deliberately not yet integrated into the hardware baseline, to preserve a one-word A/B after the initial guest trace. Full native pages are oracles only.

Original timer2319249 remains at15:35:50UTC/hard15:40:48UTC; only own56246613 is active. Current campaign compute ceiling5.09USD plus transfers. Save/push final native text before restoring Linux; this interim commit alone does not authorize an unverified result.
