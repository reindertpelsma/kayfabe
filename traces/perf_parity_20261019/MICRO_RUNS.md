# Individual tiny-probe runs

MEASURED: each row is one independent3000-iteration invocation, with five warmup iterations excluded (2995 retained). Values are submit-return to polled fence in microseconds. These are pure probe runs, separate from instrumented trace runs. Full actual common flags, revision, binary hashes and completed capture health are in RESULTS.json. Mirror has neither P1 measurement flag; all other rows use NSI tracing active, with the indicated pacer setting; GET stays unset. No outlier is removed.

| Run | Revision | NSI / pacer us | Iterations | Median us | p99 us | Max us |
|---|---|---|---:|---:|---:|---:|
| mb4-q0-r1 | 0d7ba743 | 0 / 0 | 2995 | 337.4 | 623.6 | 3734.1 |
| mb4-q0-r2 | 0d7ba743 | 0 / 0 | 2995 | 254.5 | 468.4 | 111743.7 |
| mb4-q0-r3 | 0d7ba743 | 0 / 0 | 2995 | 256.5 | 514.6 | 54513.2 |
| mb4-q0-r4 | 0d7ba743 | 0 / 0 | 2995 | 251.5 | 433.8 | 2302.8 |
| mb4-q0-r5 | 0d7ba743 | 0 / 0 | 2995 | 252.8 | 451.4 | 111727.5 |
| mb4-q2-r1 | 0d7ba743 | 0 / 0 | 2995 | 215.9 | 438.9 | 623.7 |
| mb4-q2-r2 | 0d7ba743 | 0 / 0 | 2995 | 217.6 | 432.4 | 50614.9 |
| mb4-q2-r3 | 0d7ba743 | 0 / 0 | 2995 | 217.4 | 437.8 | 50580.5 |
| mb4-q2-r4 | 0d7ba743 | 0 / 0 | 2995 | 219.6 | 428.5 | 639.0 |
| mb4-q2-r5 | 0d7ba743 | 0 / 0 | 2995 | 274.6 | 499.2 | 913.8 |
| mb4-q3-r1 | 0d7ba743 | 0 / 0 | 2995 | 187.7 | 212.6 | 318.5 |
| mb4-q3-r2 | 0d7ba743 | 0 / 0 | 2995 | 185.4 | 232.3 | 1037.9 |
| mb4-q3-r3 | 0d7ba743 | 0 / 0 | 2995 | 186.5 | 230.0 | 2230.8 |
| mb4-q3-r4 | 0d7ba743 | 0 / 0 | 2995 | 188.1 | 209.3 | 289.7 |
| mb4-q3-r5 | 0d7ba743 | 0 / 0 | 2995 | 186.6 | 209.7 | 283.0 |
| p1on1-q0-r1 | 41a8beb0 | 1 / 0 | 2995 | 240.9 | 454.7 | 1374.4 |
| p1on1-q0-r2 | 41a8beb0 | 1 / 0 | 2995 | 242.8 | 428.5 | 50737.3 |
| p1on1-q0-r3 | 41a8beb0 | 1 / 0 | 2995 | 246.9 | 449.6 | 81905.7 |
| p1on1-q0-r4 | 41a8beb0 | 1 / 0 | 2995 | 251.5 | 484.4 | 77397.9 |
| p1on1-q0-r5 | 41a8beb0 | 1 / 0 | 2995 | 333.7 | 632.8 | 1281.0 |
| p1on1-q2-r1 | 41a8beb0 | 1 / 0 | 2995 | 213.2 | 417.9 | 1194.0 |
| p1on1-q2-r2 | 41a8beb0 | 1 / 0 | 2995 | 217.7 | 426.6 | 634.3 |
| p1on1-q2-r3 | 41a8beb0 | 1 / 0 | 2995 | 214.1 | 420.2 | 2418.9 |
| p1on1-q2-r4 | 41a8beb0 | 1 / 0 | 2995 | 217.7 | 425.1 | 642.6 |
| p1on1-q2-r5-quiet | 41a8beb0 | 1 / 0 | 2995 | 302.2 | 501.7 | 1673.7 |
| p1on1-q3-r1 | 41a8beb0 | 1 / 0 | 2995 | 185.8 | 211.0 | 345.5 |
| p1on1-q3-r2 | 41a8beb0 | 1 / 0 | 2995 | 259.5 | 325.1 | 545.5 |
| p1on1-q3-r3 | 41a8beb0 | 1 / 0 | 2995 | 188.0 | 232.4 | 622.9 |
| p1on1-q3-r4 | 41a8beb0 | 1 / 0 | 2995 | 281.1 | 629.4 | 1295.4 |
| p1on1-q3-r5 | 41a8beb0 | 1 / 0 | 2995 | 189.4 | 303.2 | 395.6 |
| pacer20-q0-r1 | 41a8beb0 | 1 / 20 | 2995 | 347.7 | 767.1 | 3198.3 |
| pacer20-q0-r2 | 41a8beb0 | 1 / 20 | 2995 | 267.1 | 497.9 | 386192.1 |
| pacer20-q0-r3 | 41a8beb0 | 1 / 20 | 2995 | 268.1 | 474.8 | 2326.6 |
| pacer20-q0-r4 | 41a8beb0 | 1 / 20 | 2995 | 257.7 | 482.9 | 89768.3 |
| pacer20-q0-r5 | 41a8beb0 | 1 / 20 | 2995 | 265.1 | 507.4 | 133353.7 |
| pacer20-q0-r6 | 41a8beb0 | 1 / 20 | 2995 | 265.3 | 503.3 | 1568.8 |
| pacer20-q2-r1 | 41a8beb0 | 1 / 20 | 2995 | 208.0 | 431.1 | 1252.5 |
| pacer20-q2-r2 | 41a8beb0 | 1 / 20 | 2995 | 211.7 | 427.6 | 1447.0 |
| pacer20-q2-r3 | 41a8beb0 | 1 / 20 | 2995 | 304.7 | 687.0 | 3431.1 |
| pacer20-q2-r4 | 41a8beb0 | 1 / 20 | 2995 | 213.3 | 419.6 | 2577.2 |
| pacer20-q2-r5 | 41a8beb0 | 1 / 20 | 2995 | 208.9 | 419.9 | 674.0 |
| pacer20-q3-r1 | 41a8beb0 | 1 / 20 | 2995 | 186.4 | 221.1 | 325.2 |
| pacer20-q3-r2 | 41a8beb0 | 1 / 20 | 2995 | 186.9 | 211.9 | 296.2 |
| pacer20-q3-r3 | 41a8beb0 | 1 / 20 | 2995 | 185.4 | 231.9 | 312.2 |
| pacer20-q3-r4 | 41a8beb0 | 1 / 20 | 2995 | 185.8 | 227.5 | 314.2 |
| pacer20-q3-r5 | 41a8beb0 | 1 / 20 | 2995 | 252.8 | 354.7 | 2043.8 |
| pacer50-q0-r1 | 41a8beb0 | 1 / 50 | 2995 | 410.7 | 870.3 | 71434.5 |
| pacer50-q0-r2 | 41a8beb0 | 1 / 50 | 2995 | 296.9 | 512.6 | 1385.8 |
| pacer50-q0-r3 | 41a8beb0 | 1 / 50 | 2995 | 291.3 | 547.9 | 2023.0 |
| pacer50-q0-r4 | 41a8beb0 | 1 / 50 | 2995 | 289.3 | 503.9 | 50664.9 |
| pacer50-q0-r5 | 41a8beb0 | 1 / 50 | 2995 | 292.2 | 522.7 | 2151.6 |
| pacer50-q0-r6 | 41a8beb0 | 1 / 50 | 2995 | 289.4 | 554.1 | 137458.0 |
| pacer50-q2-r1 | 41a8beb0 | 1 / 50 | 2995 | 309.9 | 726.2 | 1574.6 |
| pacer50-q2-r2 | 41a8beb0 | 1 / 50 | 2995 | 192.7 | 429.7 | 676.8 |
| pacer50-q2-r3 | 41a8beb0 | 1 / 50 | 2995 | 199.0 | 427.6 | 769.7 |
| pacer50-q2-r4 | 41a8beb0 | 1 / 50 | 2995 | 194.0 | 417.6 | 1681.2 |
| pacer50-q2-r5 | 41a8beb0 | 1 / 50 | 2995 | 192.6 | 419.3 | 545.4 |
| pacer50-q3-r1 | 41a8beb0 | 1 / 50 | 2995 | 186.8 | 221.2 | 812.5 |
| pacer50-q3-r2 | 41a8beb0 | 1 / 50 | 2995 | 187.4 | 237.7 | 331.1 |
| pacer50-q3-r3 | 41a8beb0 | 1 / 50 | 2995 | 245.9 | 321.5 | 915.4 |
| pacer50-q3-r4 | 41a8beb0 | 1 / 50 | 2995 | 188.7 | 218.5 | 325.1 |
| pacer50-q3-r5 | 41a8beb0 | 1 / 50 | 2995 | 186.5 | 211.5 | 280.1 |
| mirror-b5-q0-r1 | 0d7ba743 | 0 / 0 | 2995 | 251.6 | 431.3 | 905.6 |
| mirror-b5-q0-r2 | 0d7ba743 | 0 / 0 | 2995 | 252.1 | 456.9 | 80721.1 |
| mirror-b5-q0-r3 | 0d7ba743 | 0 / 0 | 2995 | 243.8 | 446.8 | 50607.5 |
| mirror-b5-q0-r4 | 0d7ba743 | 0 / 0 | 2995 | 254.0 | 464.5 | 77782.1 |
| mirror-b5-q0-r5 | 0d7ba743 | 0 / 0 | 2995 | 250.2 | 426.2 | 194039.3 |
| mirror-b5-q2-r1 | 0d7ba743 | 0 / 0 | 2995 | 218.2 | 430.1 | 884.2 |
| mirror-b5-q2-r2 | 0d7ba743 | 0 / 0 | 2995 | 215.7 | 437.9 | 971.3 |
| mirror-b5-q2-r3 | 0d7ba743 | 0 / 0 | 2995 | 218.7 | 438.1 | 653.5 |
| mirror-b5-q2-r4 | 0d7ba743 | 0 / 0 | 2995 | 213.5 | 434.1 | 646.4 |
| mirror-b5-q2-r5 | 0d7ba743 | 0 / 0 | 2995 | 215.4 | 434.9 | 698.9 |
| mirror-b5-q3-r1 | 0d7ba743 | 0 / 0 | 2995 | 184.6 | 208.1 | 321.0 |
| mirror-b5-q3-r2 | 0d7ba743 | 0 / 0 | 2995 | 187.2 | 210.6 | 312.1 |
| mirror-b5-q3-r3 | 0d7ba743 | 0 / 0 | 2995 | 187.0 | 209.1 | 287.2 |
| mirror-b5-q3-r4 | 0d7ba743 | 0 / 0 | 2995 | 185.7 | 223.1 | 342.1 |
| mirror-b5-q3-r5 | 0d7ba743 | 0 / 0 | 2995 | 188.6 | 288.7 | 711.2 |

MEASURED: three final DIRECT instrumented3000-iteration traces additionally underpin W2_EDGE_RESULTS.json; their latency is not pooled with the pure rows. Rejected calibration traces, incomplete terminal-marker invocations and early setup runs are retained and described in HARDWARE_PROGRESS.md. A GET-mode Windows launch has zero completed benchmark samples.
