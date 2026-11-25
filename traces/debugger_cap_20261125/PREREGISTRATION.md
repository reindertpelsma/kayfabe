# Debugger-session cap across VMs: hypotheses, falsifiers and arms, written before the run

**STATUS: RESEARCH, 2026-11-25.** Everything below is design intent or prediction until `README.md` of this directory
records a measurement; nothing here is a measured fact. This file is committed before the first run.

## Question (owner decision A8)

Should kayfabe cap host debugger sessions across VMs? Today (PR #540, #594) kf3 admits at most
`DEBUGGERS_PER_GR_OBJECT` = 4 sessions per GR object and, per VM, the declared `channel-budget` (256 to 2048; the derived default on an idle host
was 1789 in the kf3 logs of `traces/vast_xid_v9_20261018/guest_control_excerpt.log`); nothing is counted across VMs. Each admitted guest
`GT200_DEBUGGER` alloc becomes one host `HostRm::alloc_debugger` (`crates/kf-host/src/channel.rs`, `NV83DE_ALLOC_PARAMETERS`
on the VM's own host device, bound to a twin's GR object). In host RM that object builds an internal client, device and
subdevice of its own (`kernel_sm_debugger_session.c`, `_ksmdbgssnInitClient`); `GT200_DEBUGGER` is also forwarded to GSP.
The per-session cost was INFERRED from source and never measured.

## Method

A host-only tool, no VM: `kf-debugger-cap-probe` (one process = one host RM client, device and subdevice, the same shape
kf3 holds per VM; each process builds M GR objects the way a twin does: bare VA space, GR channel group and channel,
compute object) and `scripts/bench/debugger_cap_arms.py` (arms, `nvidia-smi` samples, `dmesg` Xid watch, leak check). Sessions
are allocated with the same `HostRm::alloc_debugger` kf3 calls, round robin over the process's GR objects.

Arms (processes x GR objects per process; sessions per GR object unlimited unless named):

| arm | processes | GR objects | note |
|---|---|---|---|
| a1 | 1 | 1 | until refused or ceiling |
| a2 | 1 | 4 | |
| a3 | 1 | 16 | |
| a4 | 4 | 1 each | per-client versus global |
| a5 | 16 | 1 each | |
| k1 | 1 | 64 | at most 4 sessions per GR object (kf3 policy): a policy-compliant VM, 256 sessions |
| k2 | 1 | 448 | at most 4 per GR object: a policy-compliant VM at about the 1789 default budget |

Stop (first that happens, per arm): host RM refuses on every GR object of every process; the first new `Xid` line in `dmesg`;
20000 sessions in the arm (all processes); 4 GiB of host FB growth since the arm's start (`nvidia-smi memory.used`); a 15 minute
wall limit. Then every session is freed one by one (time recorded), `memory.used` is read, the processes exit (the client
closes) and `memory.used` is read again after at most 60 s of settling. Recorded per arm (planned, no run yet): sessions reached, per-process
counts, the refusal as the host answered it, FB used at start / after GR setup / at the peak / after freeing the sessions /
after process exit, time per alloc (mean, p50, p99, max, by thousand), the worker's RSS, and `/proc/meminfo` (`MemAvailable`,
`Slab`, `SUnreclaim`) at the same points.

Limits of the method, said up front: kf-host maps `NV_ERR_NO_MEMORY` (0x51) and `NV_ERR_INSUFFICIENT_RESOURCES` (0x1a) to one
`RmError::NoMemory`, so a refusal of either is reported as that pair; the NVRM lines that `dmesg` gains during the arm are
kept to tell them apart when RM logs one. One `HostRm` has one device, so "per device within a client" cannot be separated
from "per client" with kf-host's API; a refusal that depends on the process count, not on the sessions, is global.

## Hypotheses and falsifiers

- **H1.** Host RM refuses debugger sessions by itself at a count comparable to or lower than (per-VM budget x number of VMs)
  on a 24 GB Ada card, that is, within the 20000-session ceiling and not above 1789 x 16 = 28624.
  *Falsifier:* arms a1 to a5 reach the ceiling (20000 sessions, or 4 GiB of FB growth) with no refusal. Then a single hostile
  VM can exhaust the host before any limit applies, and the global cap is the only bound. (Prediction, from the source
  reading: no refusal before the ceiling; the only limits seen are `RS_CLIENT_HANDLE_MAX` = 0x100000 clients and GSP heap
  or host FB, each far above 20000 sessions unless a session costs far more than H2 says.)
- **H2.** One session costs on the order of tens of KiB of host FB. *Falsifier:* FB growth per session (peak minus after-GR-setup,
  divided by sessions, from arms a1 to a3) is below 4 KiB (the cost lives in host system memory, not FB) or above 1 MiB.
- **H3** (leak). Freeing every session returns `memory.used` to within 2 MiB of the value before the sessions, and closing the
  client returns it to within 2 MiB of the value before the arm. *Falsifier:* either stays above that. A leak is a finding.
- **H4** (scope). If a refusal exists, it is global across processes: the processes of a4 and a5 together reach about the
  count of a1 (within 20 percent), not that count each. *Falsifier:* each process reaches about the a1 count.

## Rule for the recommended cap, fixed before the numbers

The owner decides; this only fixes how the recommendation is derived so the result cannot steer it.
- If there is a global refusal count R (the smallest total of a4/a5/a1): recommend a global cap of floor(R / 4), rounded down to a
  multiple of 64, as the headroom left for the host's other clients and GSP.
- If there is none below the ceiling: recommend the count whose recorded FB cost (peak growth per session x count) stays under
  1 GiB (about 4 percent of 24 GB), rounded down to a multiple of 64, and never below 1789 (one VM at the default budget).
- VMs allowed at the default budget = floor(cap / 1789), and at the 256 minimum = floor(cap / 256).
- Whatever the arms say, the policy-compliant ceiling of one VM is bounded by GR objects: k1 and k2 give the cost of a VM that
  uses its whole budget with at most 4 sessions per GR object, which is the case a cap has to be sized for.

## Safety and cost

Untrusted rented box, no secrets on it (`scripts/bench/box/README.md`), evidence committed as text, teardown by our own instance
id with the absence verified, `vast_guard.py --enforce` before and after, at most 2 USD and 2 box-hours, never the Adobe image,
the pc is not used. No product code changes in this task.
