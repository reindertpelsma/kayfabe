# P1 mirror baseline, before P1 diagnostic binary

MEASURED: mirror source0d7ba7438, production cursor6 broker, run41004, same persistent P1 disk. Three captures; five 3000-iteration probes per queue. All three captures and final health checks have zero TDR/153/app-crash/new-Xid. Raw ETLs, zipped CSVs and summaries retained under the host lane directory. No comparative P1 gain is claimed.

| Capture | Guest steady fps | Broker new fps | 1% low | 0.1% low | Repeat/s |
|---|---:|---:|---:|---:|---:|
| mb4-sn-r1 | 29.883 | 29.884 | 14.999 | 11.999 | 30.008 |
| mb4-sn-r2 | 29.331 | 29.183 | 11.999 | 8.571 | 30.672 |
| mb4-sn-r3 | 29.560 | 29.396 | 11.999 | 10.634 | 30.515 |

MEASURED: new-content fps median29.396, min–max29.183–29.884. Full gap counts/max, per-run flags and sample values are in `BASE_RESULTS.json`. Scores2967 and2955 are visually read; the second UI score was not collected and is not inferred from ETW.

| Tiny queue | n | Median of run medians, us | Min–max, us |
|---|---:|---:|---:|
| DIRECT | 5 | 254.5 | 251.5–337.4 |
| COMPUTE | 5 | 217.6 | 215.9–274.6 |
| COPY | 5 | 186.6 | 185.4–188.1 |

CODE READ: these are submit-return→polled-fence medians, matching the prior “837-style” metric. DIRECT=0, COMPUTE=2, COPY=3. First-run/warm-state variation remains included; no outlier or failed run is silently removed. The earlier reset/recovered and incomplete runs remain separately documented in `HARDWARE_PROGRESS.md`.
