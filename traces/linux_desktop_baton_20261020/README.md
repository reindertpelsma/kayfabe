# Linux desktop baton, stage A (2026-10-20)

STATUS: LIVE, 2026-10-20. MEASURED, n=1 boot, kf3 0d7ba743 (KF3_TSG_MIRROR=0, no other KF3_* variable),
broker /data/claude-cursor6 (0452613b) in the host GNOME/Wayland session, guest = Cinnamon/Xorg image
(NVIDIA 580.159.04 open, kernel 6.8.0-142), tooling = scripts/bench/baton/desktop_baton.sh at this branch.
Two windowed GLX clients (glwin, K=5 ms, 1015 layers, vsync 0) side by side, 945x700 on 1920x1080.

| phase | client | fps | p50/p99 ms | longest gap | zero-s |
|---|---|---|---|---|---|
| solo 30 s | A | 225.5 | 4.43/4.66 | 26.0 ms | 0 |
| pair 300 s | A / B | 112.5 / 112.6 | 8.90/9.15, 8.91/9.17 | 28.3 / 45.4 ms | 0 / 0 |
| pair+canary 300 s | A / B / C(vsync 1, 240x160) | 111.1 / 111.2 / 56.7 | 8.98/9.33, 8.99/9.35, 17.8/35.9 | 42.8 / 38.7 / 78.6 ms | 0 |

Verdict NO_EVENT_OBSERVED in both pair phases (no BATON, no stall).  Host Xid 25 -> 25, guest Xid and flip
timeouts 0, host GPU util 97 % mean during pairs.  Broker: frames arrive as dma-buf (zero-copy), Present COPY
(compositor compositing, refresh 16.667 ms), broker ticks LIVE; relay counters sent=22227 gpucopy=22226
dropped=0 uncommitted=0 (whole boot).  Owner VM restored as run 3431 (OWNER_SESSION_READY, agent hostname ok).
Raw per-frame CSVs stay on the host: /data/claude-desktop-baton-A1.
