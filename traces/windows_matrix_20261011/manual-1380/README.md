# Owner manual1380: freeze report and preserved convenience image

STATUS: LIVE, 2026-10-11 21:39UTC.

Owner reports a GPU freeze and Minecraft freezing at startup, then explicitly
released172.22.1.20 for termination and further tests. Product3252a087,
Windows580.88, same manual boot ready20:17:45UTC. Minecraft edition/version,
launch workload and same-workload Windows reference are UNMEASURED. Owner's
Linux success is a report, not measured Windows/Linux parity. This observation
does not change the canonical104-row matrix.

## Measured observations

- Read-only host snapshot21:24:45UTC: no kernel Xid/IOMMU fault matches since
  readiness; query succeeded. No FIFO-update command printed in this log;
  that does not exclude an unobserved Passthrough trigger. See live-host-check.json.
- Corrected guest query21:27:57UTC: NVIDIA CM_PROB_NONE; three nvlddmkm153 at
  21:18:47–48UTC; no Display4101 in the queried System records. Same boot.
  Application totals1002=2,1000=0,1001=39; individual WER/LiveKernel141
  classifications UNMEASURED. Event153 alone is not TDR proof.
- Seven Minecraft processes reported Responding=true at query time. This is
  not a rendering/completion oracle and does not refute the reported freeze.
- guest-health-at-shutdown.txt preserves an earlier failed event mapper before
  the corrected result. Its empty event list is INVALID, not zero events.
- Owner-authorized stop: ACPI/QGA shutdown failed to finish; QEMU terminated
  after300s. Broker marker WINDOWS_EXIT stop=UNCLEAN-kill21:33:57UTC;
  host Xid total unchanged30. Parent exited21:34UTC; no QEMU and group11DMA-FQ
  before the next campaign. Do not equate wrapper exit0 with clean shutdown.

## Local convenience retention

Installed1380disk and baseline parent are retained on the trusted host.
KEEP.json records the full chain and copied firmware hash. Original1380disk
is read-only; owner-state/manual1380/resume.qcow2 is a separate writable child
for the next personal session. Tests use baseline-backed fresh overlays.
qemu-img-check.txt reports no qcow2 errors/exit0; NTFS/application shutdown
consistency is UNMEASURED after forced termination.

User explicitly says this is convenience only: all contents can be regenerated,
redownloaded or signed in again. No offsite disk backup requested or claimed.
No disk, credentials, tokens or account command lines are published or sent to
Vast. Public artifacts are filtered health observations and retention metadata.
