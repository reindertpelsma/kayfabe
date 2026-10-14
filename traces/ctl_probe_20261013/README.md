# kf-ctl-probe on the home host, unprivileged (2026-10-13)

**STATUS: LIVE, 2026-10-13.** Evidence for the tier-2 column of `docs/design/V3_WINDOWS_ONLY_CONSTANTS_20261013.md`
("a value kayfabe can get as a client without `CAP_SYS_ADMIN` needs no table and no sweep"). **[M]** = measured by the
run below, **[I]** = inferred.

## What was run [M]

- Host: the trusted home host, one RTX 4070, host driver `595.91.07` (`PROBE_SESSION driver=595.91.07`), no VM running,
  the lock `/tmp/kf-bisect.lock` held for the run.
- Binary: `kf-ctl-probe` built from commit `f2a9a829d` (branch `claude/ctl-probe-20261013`),
  sha256 `39c6a31a0eb75149fa2d732ae2b739b5e21041ceb860f50dc827692a81f20dc0`.
- Identity: `sudo -u ubuntu env KF_PROBE_I_KNOW=1 kf-ctl-probe` (the desktop user, not root).
  Printed `PROBE_START ... uid=1000 CapEff=0x0 cap_sys_admin=false`, and the final line `PROBE_END admin_run=false`.
  CapEff is zero, so bit 21 (CAP_SYS_ADMIN) is clear. The tool would have refused otherwise.
- `dmesg | grep -c 'NVRM: Xid'` was identical before and after both runs (5 and 5); `nvidia-smi` unaffected; nothing else touched.
- **Pass 1** = the 32 built-in rows (`probe_pass1.tsv`: 142 calls plus 2 rows reported `NOT_PROBED`). Zeroed input, one size per
  row taken from the reply sizes the Windows captures recorded, a 15-size ladder (4..4096) for rows with no recorded size,
  continued only while RM answers a size error (`0x3a`, `0x1f`, `0x02`).
- **Pass 2** = the same 142 built-in calls again (identical statuses and identical 16-byte reply prefixes for the 9 `OK` rows:
  the first repeat measurement) plus a sweep of every size 4..4096 step 4 for the five rows the desk check expects to be
  refused but whose ladder found no accepted size (`0x20800a2e`, `a87`, `ab8`, `b03`, `0x2080121f`;
  `probe_pass2_sweep.tsv`, run-length encoded). Sweeping only refusal-expected rows keeps the unknown-semantics
  GSS-legacy rows from being sent more zeroed buffers than the single pass.
- Reply bytes of the `OK` rows are **not** committed (not needed for the ruling); the tool prints a 16-byte prefix only.

## Reading a status [M for the status, I for the meaning]

| status | meaning for the tier ruling |
|---|---|
| `OK` | the host answers this control to an unprivileged client: **kayfabe can ask at runtime** (tier 2) |
| `NOT_SUPPORTED` (0x56) | the host does not answer it to this client (INTERNAL flag, or no such id, or the GPU itself answers 0x56): no value obtainable unprivileged |
| `INSUFFICIENT_PERMISSIONS` (0x1b) | needs admin: **not obtainable unprivileged** (tier 3) |
| `INVALID_PARAM_STRUCT` (0x3a) / `INVALID_ARGUMENT` (0x1f) | **no verdict.** RM reached the parameter check. [M] The size check precedes the permission check: `0x20802a12` gives 0x1f at size 4 and 0x1b at size 8; `0x20800a3f` gives 0x1f at 4..128 and 0x56 at 256. [I] For a GSS-legacy row 0x1f can mean a wrong size or a zero buffer the firmware rejects; the probe cannot tell the two apart. |

## Result against the desk check (`tier_checks/tier2_compute_host.tsv`) [M]

30 controls on the NV2080/NV0080 objects (plus 2 not probed) (`desk_check_vs_measured.tsv`):

| verdict | rows | which |
|---|---|---|
| MATCH | 17 | OK: `852a`, `8524`, `9001`, `9009`, `a060`, `a088`, `a630`, `a70a`, `a801` (9, all predicted obtainable). Refused as predicted: `2806` NOT_SUPPORTED, `2801` and `2a12` INSUFFICIENT_PERMISSIONS, `121f` INSUFFICIENT_PERMISSIONS (at size 40, sweep), `a2e` NOT_SUPPORTED (size 96, sweep), `a3f` (256), `a80` (16), `a87` (1064, sweep) NOT_SUPPORTED |
| NOT CONFIRMED | 12 | desk check says reachable or refused, the probe got only `INVALID_ARGUMENT`/`INVALID_PARAM_STRUCT` at every size it tried: `852e`, `852f`, `853a`, `853b`, `9004`, `9064`, `a618`, `a619`, `b201`, `0x00809908` (all predicted obtainable: still no value, layout unknown, i.e. still the "MEASURE-LAYOUT" class) and `ab8`, `b03` (predicted refused, no accepted size in 4..4096) |
| DIFFERS | 1 | `0x2080a02f`: desk check "YES, forwarded"; measured `NOT_SUPPORTED` at size 1. The inventory already records that the real GPU answers 0x56 to it (5 calls, 1 B) in the Windows captures, so the host answer equals the real answer: reachable, but **there is no value to obtain**. This is a contradiction of the desk check's "obtainable", not of its routing claim. |
| NOT PROBED | 2 | `0x20810108`, `0x2081010d` (NV2081 binary-API object) and every NV0073 row: the probe has no object of those kinds (stated in its header), so groups B/C4 and the NV2081 rows have no measurement yet |

No row contradicts a *refusal* prediction: every control the desk check put in P/K/I/A that returned a decisive status returned the predicted one
(group P: 2801, 2a12; group K: 121f; group I: a2e, a3f, a80, a87; group A: 2806). Nothing predicted refused answered `OK`.

What this changes for the ruling [I]: the 9 `OK` rows can be asked at runtime today (tier 2), if the zeroed-input reply is the one kayfabe needs; the 12
undecided rows need one more thing before they leave "MEASURE-LAYOUT": a request layout (from the Windows capture) tried against the host; `0x2080a02f` should be
removed from the "obtainable" list; the INTERNAL/KERNEL/PRIVILEGED rows are confirmed tier 3/4 on this host.

## Limits

One host, one driver (595.91.07), one run plus one repeat, zeroed inputs only. A decisive status at one size says nothing about the answer a *non-zero*
input would get. `INVALID_ARGUMENT` rows are undecided, not refused.
