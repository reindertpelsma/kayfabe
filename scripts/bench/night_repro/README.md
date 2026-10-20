# Windows multi-client night reproducer

STATUS: BLOCKED / UNQUALIFIED, 2026-10-20. Scripts and observations only; no product fix.

MEASURED: the first hardware collection failed with `PresentMon header absent`.
PowerShell serialized `pm_header` as a metadata object; collection prepended its
string rendering before the actual CSV header. The parser refused that preamble.
The lane's mandatory STOP rule ended experiments and restored the owner session.
This transport defect remains unfixed. Do not use this harness for an A/B claim
until a later authorized session adds a failing transport regression, corrects
serialization, and completes A/A validation. `RESULT.json` from this campaign is
an explicit failure receipt with null silence metrics, written during reporting.
The 25 passing Python tests do not cover PowerShell serialization.

CODE READ: this harness starts a fresh VM from a copy of the owner launcher,
restarts the existing vCPU watchdog and PresentMon monitor, runs windowed Heaven
D3D11 benchmark passes and windowed GravityMark Vulkan passes in separate
interactive scheduled tasks, collects evidence, and shuts down cleanly. Each
client repeats after its own normal benchmark exit. Its child PID lifetimes
exclude normal between-pass pauses. Minecraft's available task opens its launcher;
a loaded unattended world is not established. The scripted pair is a candidate
reproducer until A/A measurements qualify it.

Run on the trusted host. Supply `N1_ROOT` (lane evidence directory), `N1_WINDOWS`
(existing Windows harness root), `N1_LAUNCHER` (owner launcher), `N1_AGENT` (existing
command broker), `N1_LOCK` (shared flock), `N1_STOP` (owner stop flag), and
`N1_DEADLINE` (an explicit ISO timestamp with UTC offset). Optional `N1_PROBE`
names the existing read-only USERD probe. Private host paths stay in the local
configuration, outside this repository.

```
./run.sh --arm A --rev <owner-revision> --minutes 12
./run.sh --campaign
```

The campaign reads `queue.json` in the evidence directory, e.g.
`[{"id":"A1","args":["--arm","A","--rev","<revision>"]}]`.
It holds the lock between runs for review. Append jobs by atomically replacing the
queue file; create `finish` to restore early. At the deadline it starts restoration.
It reserves ten minutes for clean shutdown/restoration before admitting a run.
The operator must stop the existing owner VM cleanly before invoking it. Two
independent lock checks reject running the Python entry points without `run.sh`.
No harness wait changes product timing. Helper copies refuse unclean QEMU kills.
After restoration, the standard owner launcher acquires its own lease; the N1
campaign lease ends. Inspect `restoration.json` and verify `OWNER_SESSION_READY`
in that log, plus guest responsiveness. Restoration uses the original owner
launcher, revision `0d7ba743`, and exactly the three specified lifecycle/mirror
flags. The wrapper does not alter sysctls.

CODE READ: `RESULT.json` uses PresentMon `TimeInQPC` event time, rather than the
extra receipt-QPC prefix added by the existing monitor. Silences are per
application/PID, armed by 100 presents in the preceding 30 seconds, and include
the whole gap when it is at least four seconds. Terminal gaps are right-censored
and labeled. The raw monitor triggers are also retained; its executable-level
grouping and global 20-second trigger cooldown make those triggers incomplete
as a per-process freeze count. App exits and missing load must be reviewed; a
presentation silence alone does not prove a GPU deadlock. `load_present=false`
invalidates a zero-freeze comparison. Host Xid counts are cumulative dmesg line
counts, with pre/post NVRM snapshots; ring/backstop/lifecycle tails and every
matching stall-diagnostic line are retained next to the full run log.

The collector compresses only this run's appended CSV/trigger bytes and copies
small client logs through the existing guest agent. Existing monitor logs and
trace files are preserved. No executable, VM image, secret, or raw host path
belongs in public evidence. `run.json`/`command.json` in the private evidence
record exact revision, environment, GPU/driver/module, duration and script hash.

MEASURED: validation receipts belong in `traces/night_repro_20261020/`.
Tests: `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s scripts/bench/night_repro`.
