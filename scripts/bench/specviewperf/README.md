# SPECviewperf 15 unattended guest runs

CODE READ: this wrapper uses SPEC's supported CLI; it does not modify vendor
binaries or workload definitions. The CLI must already be installed with the
requested workloads. The guest must have an active kfadmin desktop session.

MEASURED limitation from VF1: the Windows PowerShell launch returned a null
Process.ExitCode after both full iterations, although all 18 workload statuses
were Completed. Do not treat the wrapper's process exit as authoritative until
handle retention/direct Process launch is verified with isolated exit 0/exit 7
children. The original suite's exit code cannot be recovered retroactively.
Do not run a test containing exit directly inside the shared command broker;
use an isolated child process and read its result file. The selected follow-up
needed for final runner validation was not executed after the safety stop.
[PowerShell issue20400](https://github.com/PowerShell/PowerShell/issues/20400).

Generate the guest-agent installer on Linux:

```sh
python3 scripts/bench/specviewperf/guest_suite.py \
  --workloads blender-01,catia-07,creo-04,energy-04,enscape-01,maya-07,medical-04,solidworks-08,unreal_engine-01 \
  --iterations 2 --output install-suite.ps1
```

CODE READ: run the generated file through the existing guest agent as SYSTEM.
It places files under `C:\Users\Public\codex-VF1`, grants kfadmin access, and starts
an interactive-logon scheduled task as that standard user. Refusal of an already
running task prevents overlapping runs. The CLI resolution is explicitly FullHD;
its timeout and workload-gap settings are not changed. Each iteration has its own
vendor result directory. The task records start/exit markers, stdout and stderr,
and emits `subtest-fps.csv` and `subtest-fps.txt` when the CLI exits.

CODE READ: the generated table preserves workload status independently of numeric
subtest FPS. Failed workloads can have valid individual numbers, and an unfinished
workload stays `Incomplete`. The last `results.xml` alone is insufficient: it holds
only the most recent subtest. The table reads all XML fragments in `stdout.log`.
Do not calculate an official composite from partial output. Preserve the vendor
CSV/HTML score and the completion status separately.

Generate a read-only table export for an existing/partial run:

```sh
python3 scripts/bench/specviewperf/guest_suite.py --report-only --output emit-table.ps1
```

CODE READ: `export-text.ps1` produces the text-only JSON export from an existing
run, with per-file read errors recorded and workload composites selected from
vendor `result.json`. It preserves file-sharing restrictions and omits raw
system-info objects. Run its source through the guest agent as a file; its
default root matches the runner.

CODE READ: `result_table.py` also emits CSV from a text-only guest export JSON
object with `root` and `files: [{path, text}]`. It does not export raw system info:

```sh
python3 scripts/bench/specviewperf/result_table.py export.json --arm vfio --output fps.csv --scores-output scores.csv
python3 -m unittest discover -s scripts/bench/specviewperf -p 'test_*.py'
```

CODE READ: `vfio_session.py` is the trusted-host VF1 lane supervisor. It needs the
GPU lock for its entire lifetime and an explicit restoration deadline. Launch it
in a dedicated system service when stopping login services; an SSH session scope
can otherwise be closed with its child VM. `--desktop-takeover` implements the
owner's explicit desktop-process authorization while protecting unknown compute
users. Native driver restoration releases VFIO before module loading. New Xids
are watched through a kernel journal cursor, independent of dmesg ring rollover.

CODE READ: SPEC documents at least 15 GB available RAM, and warns about CLI
background-task behavior on newer Windows versions. This lane preserves the
owner's requested 12 GiB VM configuration; its results are diagnostic references,
not submission-compliant scores. Retain resolution, driver, foreground observations
and vsync settings with comparisons. Vendor references:
[SPEC user guide](https://gwpg.spec.org/wp-content/uploads/2025/12/SPECviewperf15-User-Guide.pdf),
[systemd descriptor store](https://systemd.io/FILE_DESCRIPTOR_STORE/).

