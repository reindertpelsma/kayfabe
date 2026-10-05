# Native instrumentation check

**STATUS: RESEARCH, 2026-10-05.** Instrumentation verdict **FAIL**; this is not a
passing Kayfabe isolation test. Probe source `e25e1bf9`, evidence branch
`codex/tspace-probes-2026-10-05` at `c17f6a14`; trusted controller binary SHA256
`2a22b40727ccb02295a540469d8098f8b7d255bb143c7e3310ff6f8e0ff3454e`.

Native RTX 4070 / Linux open NVIDIA 595.91.07. Launched under the serial bench
lock with UID/GID 65534, no supplementary groups or capabilities, and
no-new-privileges. The 90-second external timeout did not fire: process exit 1.
The test uses only its own allocations and USER channels. It verifies an alias
read, unmaps that alias, and submits a four-byte read through the invalid alias.

Both initial copies and the mapped-alias read passed. The invalid-alias read
produced exception 31, notifier status 65535 and a fresh timestamp; neither
fence nor CE completion occurred and the destination poison remained intact.
The independent channel still copied correctly, the source was unchanged, and
cleanup passed. However, the notifier reports engine type 1 whereas the current
checker requires COPY0 (9). That mismatch makes the verdict fail.

The host kernel independently logged Xid 31 for this process/channel:
`MMU Fault: ENGINE CE0 HUBCLIENT_CE1 ... FAULT_PTE ACCESS_TYPE_VIRT_READ`.
Host nvidia-smi remained healthy afterward. The checker assumption must be
reconciled with public RM source before changing it; a single captured engine
value does not justify weakening fault attribution. No product code or isolation
default was changed. The full guest/canary coordinator and hardware merge bar
remain incomplete.
