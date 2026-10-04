# Confirmed ordinary-user ioctl origins

**STATUS: RESEARCH, 2026-10-04. Native Linux test, not a Windows-through-Kayfabe test.**

[Per-control table](privileges.md) · [Full correlation evidence](privileges.json)

On RTX 3060 GA106, Linux 6.8.0-59 and NVIDIA 575.51.03, **48 of the 129 Windows
control IDs reached native GSP from matching ordinary-user control ioctls**.
Eight IDs reached GSP internally during ordinary-user ioctls; one overlaps the
direct set, giving **55 distinct IDs**. These are positive observations, not an
allowlist and not a claim that every request succeeded in firmware.

The internal set is `2080012b`, `2080012f`, `20800a9a`, `20800aff`, `20802a08`,
`90f10106`, `a06c010a`, `a06f0103`. In particular, an internal PROMOTE_CTX caused
by user allocation does **not** make a direct user PROMOTE_CTX ioctl permitted.

Seven workload executions passed the identity guard: `nvidia-smi -q`, CUDA
context create/destroy and GPU vector addition on both the stock **closed**
575.51.03 module and instrumented **open** 575.51.03; plus read-only queries and
negative controls on the latter. All real/effective/saved UIDs/GIDs are 65534,
supplementary groups are empty, all five capability sets are zero, and
NoNewPrivs is one. The guard execs the workload at the recorded PID. NVIDIA
descriptors are opened by those workloads after dropping privilege.

## What is proved, at which boundary

- The stock closed module has **37** Windows IDs with successful direct user
  control ioctls. The open module has **38**. These are ioctl results, not
  closed-module GSP observations. Proprietary symbols could not be probed; no
  closed-module GSP census is claimed.
- The open recorder retains **3,781** records. Its strict decoder reports no
  wrap, dropped records or RX errors. Credential stamps match each record by
  sequence **and exact monotonic timestamp**, not by PID proximity.
- **641** actual sends join to a same-TID ioctl interval with successful
  transport outcome, no NOT_SENT flag, kernel UID/EUID 65534, permitted and
  effective capabilities zero, and NoNewPrivs one. There are 272 RM_CONTROL,
  206 RM_ALLOC, 108 FREE, 50 DUP_OBJECT, four SET_PAGE_DIRECTORY and one CONTINUATION
  send. RPC names come from the exact source's `rpc_global_enums.h`.
- Another **792** credentialed sends have no enclosing traced ioctl. They
  are excluded from the ioctl-origin proof: open/close paths also perform
  resource management. They are not evidence of lost RPC records.
- A matching outer ioctl command gives the **direct** category; other
  controls and allocation/UVM ioctls give **indirect**. Firmware support and
  ioctl success are separate columns. In particular, successful transport of
  an unsupported query proves reachability, not successful functionality.
- Logs from the first diagnostic attempt were appended by the preload shim.
  Analysis accepts only this capture's guarded TGIDs and traced TIDs for open
  tests; closed ioctl evidence is restricted to the guarded main TID. Earlier
  records remain visible in the raw logs but are not counted.

Negative controls on fresh caller-owned objects agree with source:
`GR_GFX_POOL_QUERY_SIZE (2080121f)` and admin display `0073028b` return
`INSUFFICIENT_PERMISSIONS (1b)`; internal `GR_GET_FECS_TRACE_HW_ENABLE (20800a38)`
returns `NOT_SUPPORTED (56)`. These three direct ioctls emit no matched GSP
request. This is a bounded user-interface test, not a kernel-caller rejection.

## Source and instrumentation

The official open module is tag 575.51.03 at
`e00332b05f433f1fe2e465d4741dfa7e41bf0a72`, matching the installed userspace and
firmware. Existing `scripts/rpctrace` records transport payloads and outcomes;
[credential-stamp.patch](../../audit/unprivileged/credential-stamp.patch) adds
only diagnostic credential/sequence printk lines. The kernel log is streamed
because a final dmesg snapshot lost early stamps in a preliminary attempt.
No GPU algorithm was modified. No timing comparison is claimed.

The source table records ordinary export flags for exact 575.51.03 and
580.65.06 (`307159f2623d3bf45feb9177bd2da52ffbc5ddf9`). The latter is the public
ABI counterpart used for the Windows capture. The dispatch checks are in
`src/nvidia/src/kernel/rmapi/control.c`: `INTERNAL` requires internal entry;
`PRIVILEGED` requires admin; the default requires a kernel caller;
`NON_PRIVILEGED` permits a user at this gate, subject to ownership, access
rights and other checks. On Linux, the admin test uses CAP_SYS_ADMIN.
Generic GSS and BinAPI routes are separate; a missing export is not proof of
uncallability. No exhaustive all-version claim is made.

Only text data returned from the disposable rental (compressed JSONL contains
hex-encoded capture bytes). The analyzer reconstructs those bytes in memory,
checks SHA-256 and invokes the existing strict decoder. It never executes
rental-produced binaries. Input source and instrumented module hashes are
retained. Rental **54195016** was destroyed after evidence commit `089559c1`
was pushed; its absence was verified ([receipt](retirement.json)). Other lanes'
rentals were not touched.

## Reproduce

From the research checkout, with local official OGKM clones at the pins above:

```sh
python3 tools/windows-gsp-trace/audit/unprivileged/analyze.py \
  tools/windows-gsp-trace/evidence/2026-10-04-ga106-unprivileged-575.51.03 \
  --ogkm575 /path/to/ogkm-575.51.03 --ogkm580 /path/to/ogkm-580.65.06
```

The test harness is deliberately for a dedicated disposable 575 fixture, not
an installer to run on an arbitrary personal host. Unprivileged reachability
alone authorizes no Kayfabe forwarding: each proposed host verb still needs
versioned parsing, bounded fields, handle ownership, pointer/address treatment,
output validation and a review of effects on other clients.
