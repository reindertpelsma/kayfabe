# Z1 concurrent Linux guests — preregistration

Recorded before hardware execution, 2026-10-27T23:23Z. Product source is current `next`
`858ca1e8ad4a2b264656b8e499d820856bcc7310`; operator-only commits follow it.
The exact binary hash, host GPU and driver will be recorded before testing.

H1: two separate kayfabe QEMU processes bound to the same selected host GPU complete
cup3, element-checked matrix multiplication, and a short fast-suite subset concurrently.
The outputs must equal the same-box bare-metal oracle. FALSIFIER: wrong output,
hang at the unchanged workload deadline, or a host Xid outside the source-matched
approved negative controls. All other new Xids stop the campaign.

Protocol: wait for ENS11 to finish and acquire the shared host lock with `flock -o`.
Refuse any pre-existing QEMU; never stop the owner's VM. Check MemAvailable before
EVERY VM and retain at least 2 GiB beyond the next guest's configured RAM. Each VM
uses 4 GiB guest RAM for CUDA (2 GiB for raw arms) and 4 GiB VRAM; select the SAME GPU minor explicitly. Run cup3
and the cup8bench matmul instrument on bare metal first, then >=3 solo
measurements and >=3 concurrent pairs with the SAME workload parameters and images.
Record per-VM result and throughput distribution (min/median/max), plus recorded
execution-window overlap. The short concurrent raw subset is timer, engines,
ce-client and guest-ram-pin; no deliberate faults in this subset.

Use host_xid_watch.sh for the fault-free concurrent campaign. Rootless/full native
negative controls use the existing exact channel/VA/type/CE matcher separately.
Capture PIDs/start times, same-GPU binding, source hashes and cleanup. No fairness,
hostile-kernel containment, isolation or Windows claim follows. The optional hostile
kernel latency probe is UNRUN unless separately preregistered and cheap.

Synchronization: the CUDA guest tool waits for a preregistered wall-clock epoch
20 seconds after launching the first VM; both run unchanged workload parameters.
Actual execution-window overlap must be recorded. The GPU-drain observation limit
is 11264 MiB for planned pairs of 4096 MiB stores on the 12 GiB card; the 180-second
workload oracle is unchanged. The short raw subset uses one boot with four arms.

Capture correction before any hardware workload: the accumulated boot journal is
32.7 MB, beyond the existing 16 MiB reader bound. Z1 therefore pins the first of
two readable real kernel records and the boot ID before running. Every scoped
read includes that exact cursor and all later kernel entries, using the same
16 MiB byte bound; cursor loss, vacuum, boot change or malformed/non-kernel data
refuses. The existing source/PID/channel/VA/type/CE matching oracle is unchanged.
The unsupported nvidia-smi inventory field stopped setup before builds or GPU use;
that receipt is retained, and BAR1 is read using nvidia-smi -q -d MEMORY instead.
