# LiveKernelEvent 193 phase-0 record

STATUS: RESEARCH, 2026-11-09. MEASURED in coordinator-supplied scrubbed records on Vast instance 59742721, RTX 4090 AD102, checkout/binary `8d495e90`, guest driver 580.88. DX1 reviewed the records GPU-free and did not rerun the hardware experiment. Provenance hashes are in `provenance.json`; selected files are copied verbatim except header files, which retain only file/header/count lines.

| Gate-flow boot | Enable UTC | Fresh report EventTime UTC | Fresh WER event-1001 entries | Distinct fresh reports | Stale entries |
|---|---|---|---:|---:|---:|
| p193_h1 | 02:19:42 | 02:20:15 | 3 | 1 | 2 |
| p193_h2 | 02:35:48 | 02:36:27 | 3 | 1 | 2 |

MEASURED at `8d495e90`, 2026-11-09 (h1/h2): the fresh reports have different report-directory identities and EventTimes from the old image report. The three fresh entries in each boot are re-submissions of that single report. The old gate counter therefore reports 3, not 1. Its `livekernelevent193` field counts entries. The report identities/EventTimes are visible in each `g_wer.txt`, and old counters/stale classifications in `g_tdr.txt`.

MEASURED at `8d495e90`, 2026-11-09 (h1/h2): all readable 193-report dump copies in these header files contain bugcheck `0x193`, p1 `0x815`, p2 `2`, p4 `0`; p3 changes between boots. The new dumps are 262,144 B. The 1,394,548,736 B root dump is an older gate-run artifact, not a new dump from either boot. The duplicate old report-directory copy is 790,596 B. Only 4,096 bytes were read from each file.

MEASURED in the coordinator's timestamped records at `8d495e90`, 2026-11-09 (h1/h2/h3): first event entries occur about 35 to 43 seconds after enable (h1 34.7 s, h2 40.4 s, display-off h3 42.9 s); h1/h2 dump mtimes follow host GSP Running by about 1.5 to 2 seconds. Host timestamps are tailer-read times, and host/guest alignment uncertainty is about 2 seconds. These timings show correlation, not cause. The display-off h3 boot also produces many non-193 events and nvlddmkm resets; its 50 retained headers are `0x141`, so it is not a clean 193 control. Its pruned 193 dump was not read.

CODE READ: [Microsoft's 0x193 reference](https://learn.microsoft.com/en-us/windows-hardware/drivers/debugger/bug-check-0x193--video-dxgkrnl-livedump) identifies the dxgkrnl live-dump code, lists p1 reason `0x100` Internal and reserves p2-p4. It does not document the observed reason `0x815`. [The 0xC1 reference](https://learn.microsoft.com/en-us/windows-hardware/drivers/debugger/bug-check-0xc1--special-pool-detected-memory-corruption) describes a different code. The observed header establishes the code for these dumps; it does not establish the reason or harmlessness.

No dump body, vendor stack, ETW or symbols were analyzed. The owner keeps the ratchet report-only. The enabled-at-boot control produced no new report during its idle period; that does not establish a causal explanation for the enable-after-desktop correlation.
