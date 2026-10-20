# Broker preparation STOP

STATUS: LIVE, 2026-10-20.

MEASURED02:52:15UTC: the unmodified interactive.sh prep guard refused because its required nvkvm-pv badf2d7 reference repository is absent at /root/nvkvm-pv on own instance56816419. Exit2; no desktop VM/image initialization happened. The current-product broker had already built successfully (SHA001791413c1ab10c4eed8fbd60cd6bf7d55842774ab4060e5c573fdccefd25b8). Both graphics suites passed first.

COMMON.md: “If a step is blocked or refused by your own safety checks, STOP and report; do not reword or work around it.” Both595/580 broker arms are STOPPED, not product FAIL. No guard/source edit or alternate launch was attempted. Independent native trace and GPU-free review continue.

Proposed resolution, pending coordinator approval: clone the public reference into this rental's own /root/sweep-nvpv-reference; verify pinned badf2d7; supply its documented NVPV_DIR to the unchanged prep command. The auxiliary reference build satisfies prep's existing dependency; the actual benchmark uses /opt/sweep-broker/nvkvm-display-broker from frozen product0c87acda8, with process/binary hashes checked. Preserve current VM/grader guards and deadlines. No changes to nvkvm-pv or product code. See restore_reference.sh; it has NOT run.
