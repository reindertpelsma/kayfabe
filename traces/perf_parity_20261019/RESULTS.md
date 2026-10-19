# P1 hardware captures

MEASURED: completed captures passed acquisition, clock, health and archive checks. Application online validation is separate: the20us result UI shows warning icons, and the inspected r4 tooltip explicitly says it was not validated online; its UI FPS was occluded. Runtime flags, source revisions, individual tiny probes, archive hashes, pairing checks and health/counter results are in `RESULTS.json`. Common production flags are recorded there in full. The optional source binary is41a8beb0; the author mirror control is0d7ba743. Pacing/lazy/cache arms compare to diagnostic-ON, changing one variable. All use the same disk and cursor6 broker, with reboot between configurations.

| Arm | n | New-content fps median [min,max] | ISR entries/frame median [min,max] |
|---|---:|---|---|
| mirror | 5 | 29.200 [28.773,29.884] | 22.613 [22.034,23.639] |
| diagnostic ON | 5 | 29.828 [29.281,30.115] | 22.578 [22.447,22.670] |
| pacer20us | 5 | 29.854 [29.584,30.351] | 22.481 [22.006,23.694] |
| pacer50us | 5 | 29.748 [28.988,30.153] | 23.469 [23.084,24.588] |

MEASURED: spread means the minimum and maximum of independent run averages. Overlapping primary-fps arms require at least five captures; incomplete arms do not settle their falsifiers. No outlier is removed.

| Run | Arm / revision | Guest steady fps | Broker new fps | 1% / 0.1% low | >33 / >50 / >100ms gaps/min | Max gap ms | Repeats/s | App score |
|---|---|---:|---:|---|---|---:|---:|---:|
| mb4-sn-r1 | mb4 / 0d7ba743 | 29.883 | 29.884 | 14.999 / 11.999 | 1096.62 / 663.49 / 1.06 | 116.673 | 30.008 | 2967 |
| mb4-sn-r2 | mb4 / 0d7ba743 | 29.331 | 29.183 | 11.999 / 8.571 | 1127.74 / 622.20 / 3.15 | 183.342 | 30.672 | uncollected |
| mb4-sn-r3 | mb4 / 0d7ba743 | 29.560 | 29.396 | 11.999 / 10.634 | 1062.87 / 691.44 / 2.10 | 116.672 | 30.515 | 2955 |
| p1on1-sn-r1 | p1on1 / 41a8beb0 | 29.685 | 29.612 | 11.999 / 10.668 | 1061.84 / 670.75 / 2.10 | 200.009 | 30.348 | 2972 |
| p1on1-sn-r2 | p1on1 / 41a8beb0 | 30.002 | 29.828 | 11.999 / 11.327 | 1083.26 / 646.60 / 2.10 | 150.007 | 30.021 | 2993 |
| p1on1-sn-r3 | p1on1 / 41a8beb0 | 30.138 | 30.115 | 11.999 / 10.721 | 1101.73 / 624.56 / 2.12 | 116.673 | 29.796 | 3015 |
| p1on1-sn-r4 | p1on1 / 41a8beb0 | 30.105 | 30.053 | 11.999 / 9.118 | 1086.96 / 640.38 / 3.16 | 150.007 | 29.842 | 3014 |
| p1on1-sn-r5 | p1on1 / 41a8beb0 | 29.285 | 29.281 | 11.999 / 7.790 | 1038.59 / 726.80 / 3.19 | 250.012 | 30.718 | 2936 |
| pacer20-sn-r1 | pacer20 / 41a8beb0 | 29.606 | 29.584 | 11.999 / 7.833 | 1040.40 / 683.69 / 7.43 | 150.007 | 30.310 | 2963 |
| pacer20-sn-r2 | pacer20 / 41a8beb0 | 29.726 | 29.714 | 11.999 / 6.666 | 1066.73 / 658.73 / 7.44 | 150.008 | 30.227 | 2979 |
| pacer20-sn-r3 | pacer20 / 41a8beb0 | 29.949 | 29.921 | 11.999 / 8.777 | 1079.68 / 644.63 / 2.12 | 433.354 | 30.009 | 2988 |
| pacer20-sn-r4 | pacer20 / 41a8beb0 | 30.380 | 30.351 | 19.999 / 11.999 | 1193.15 / 562.04 / 0.00 | 83.338 | 29.590 | 3031 |
| pacer20-sn-r5 | pacer20 / 41a8beb0 | 29.880 | 29.854 | 11.999 / 8.571 | 1070.48 / 651.64 / 5.32 | 150.007 | 30.049 | 2988 |
| pacer50-sn-r1 | pacer50 / 41a8beb0 | 29.001 | 28.988 | 11.999 / 2.609 | 1053.58 / 634.70 / 9.57 | 416.686 | 30.902 | 2905 |
| pacer50-sn-r2 | pacer50 / 41a8beb0 | 29.016 | 29.015 | 11.999 / 6.162 | 1049.45 / 663.76 / 8.52 | 216.676 | 30.986 | 2885 |
| pacer50-sn-r3 | pacer50 / 41a8beb0 | 29.761 | 29.748 | 11.999 / 6.666 | 1073.89 / 653.67 / 4.24 | 183.342 | 30.190 | 2980 |
| pacer50-sn-r4 | pacer50 / 41a8beb0 | 30.146 | 30.153 | 12.048 / 10.721 | 1097.82 / 638.09 / 2.12 | 150.007 | 29.799 | 3020 |
| pacer50-sn-r5 | pacer50 / 41a8beb0 | 30.163 | 30.150 | 14.999 / 11.999 | 1089.21 / 649.70 / 1.06 | 183.342 | 29.831 | 3012 |
| mirror-b5-sn-r1 | mirror-b5 / 0d7ba743 | 28.784 | 28.773 | 11.999 / 3.326 | 1072.87 / 619.45 / 8.51 | 1066.717 | 31.132 | 2880 |
| mirror-b5-sn-r2 | mirror-b5 / 0d7ba743 | 29.197 | 29.200 | 11.999 / 10.515 | 1063.53 / 640.67 / 2.12 | 1083.384 | 30.758 | 2905 |

| Arm | Tiny queue | n | Submit-return to polled-fence median [min,max], us |
|---|---|---:|---|
| mirror | DIRECT | 10 | 252.4 [243.8,337.4] |
| mirror | COMPUTE | 10 | 217.5 [213.5,274.6] |
| mirror | COPY | 10 | 186.8 [184.6,188.6] |
| diagnostic ON | DIRECT | 5 | 246.9 [240.9,333.7] |
| diagnostic ON | COMPUTE | 5 | 217.7 [213.2,302.2] |
| diagnostic ON | COPY | 5 | 189.4 [185.8,281.1] |
| pacer20us | DIRECT | 6 | 266.2 [257.7,347.7] |
| pacer20us | COMPUTE | 5 | 211.7 [208.0,304.7] |
| pacer20us | COPY | 5 | 186.4 [185.4,252.8] |
| pacer50us | DIRECT | 6 | 291.8 [289.3,410.7] |
| pacer50us | COMPUTE | 5 | 194.0 [192.6,309.9] |
| pacer50us | COPY | 5 | 187.4 [186.5,245.9] |

CODE READ: no GPU-fetch execution timestamp is inferred from a CPU marker or overlapping host IRQ. Diagnostic traces and pure tiny probes are kept separate. Earlier setup/reset/incomplete probes remain in `HARDWARE_PROGRESS.md`; they are not pooled. The original p1on1-q2-r5 is excluded because its terminal PowerShell marker was absent despite all3000 raw iterations; its successful isolated quiet repeat is labelled separately.
