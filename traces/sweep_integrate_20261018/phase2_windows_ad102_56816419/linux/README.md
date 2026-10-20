# AD102 Linux refresh

STATUS: LIVE, 2026-10-20.

Own instance56816419, frozen current product0c87acda802b18a610739e75f2b50049a9247067, host open595.91.07. Independent standalone595.91.07 and580.159.04 images (no mutable backing parent) and the unchanged graders/time budgets. Both raw suites PASS30/30 at180seconds per arm; every serial module banner matches its axis. Both CUDA suites PASS4/4 with exact payload validation, n=1, timing spread N/A. Native595 raw30/CUDA4 and gates passed before guest runs.

595 raw carries1562 heartbeat warning lines; raw580 has zero. Both raw suites contain zero guest Xid and zero flip timeout lines. Counts and raw-zero/nonzero split are in heartbeat_counts.json. Warning presence is retained despite suite PASS; no heartbeat fix or full family completion is claimed. Graphics/broker remain pending. The old AD102 timer failure is not reproduced here; different boxes and n=1 do not establish a universal timing result.

All30 serial logs per raw axis, canonical CUDA tables, rung logs and source/export hashes are retained. GPU identifiers were redacted and whitespace normalized by the bounded exporter; no VM image or executable was copied back.
