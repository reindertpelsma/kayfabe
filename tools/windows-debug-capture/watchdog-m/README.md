# Bounded WATCHDOG collection for probe M

**STATUS: RESEARCH, 2026-10-05.** Prepared offline. No PC/QGA/VM/GPU access was performed. PowerShell AST
and Python compilation pass; journal normalization reproduces all 24 L assertions
when compared with itself. The later M attempt preserved its dump, then KD/QGA failed; see the
[offline recovered result](evidence-m/README.md). A completed end-to-end collection
is not claimed.

The original three committed scripts are byte-identical to the originally prepared
controller recipes; their exact hashes are in [source-sha256.json](source-sha256.json).
Only sources and documentation are stored here. Dumps, debugger binaries, full
journal output and the local self-comparison result are deliberately excluded.

The absolute paths below record the original experiment's controller/host layout.
Substitute a private controller output directory and the currently running guest's
work directory when reproducing. `collect-m.ps1` retains fixed Windows paths and
refuses reuse. The source code never initiates a VM, restarts an adapter, or shuts
down a guest.

Trusted inputs used on the original controller:
- `/PRIVATE/kd-sources/analyze-dump.ps1`
  SHA256 `7373e87e3f2681ccd223ada92f1c561e8f29b147c83c599d3865aa1a2049dc6a`
  Introduced at `4f27aadbdaca65b85f088c3fe0e17e5cd976f469`.
- `/tmp/kf-kd-reproduced-v4/kf-kd-bundle.zip` (13 MiB)
  SHA256 `a3883456c4631bb46b9091a2d3197f17984df99ba89c36c2f8e54c976052d800`.
  Reproducible via source `fetch-debugger.py`; wrapper verifies the bundle,
  each pinned member and Microsoft Authenticode before executing KD.
- Schema `/tmp/kf-nvcd-schema-public-v1/schema.json`.
- L private evidence `/data/kayfabe-runtime/windows-pool-20261005/probe-l/private-kd`;
  these inputs are not supplied in this public repository.

The wrapper includes `.enumtag`, so raw analysis and decoded journals remain private.
Do not commit raw dump, driver bytes, absolute kernel pointers or complete output.

## Execution steps before M shutdown

The existing experiment's controller helper is
`/data/kayfabe-boundary-20261005/scripts/bench/windows/run_pc_boundary_series.py`.
Load it without calling `main()`:

```python
import base64, importlib.util
from pathlib import Path
spec = importlib.util.spec_from_file_location('boundary', HELPER_PATH)
h = importlib.util.module_from_spec(spec)
spec.loader.exec_module(h)
```

The helper must expose `remote(argv, timeout=..., check=...)`, `guest(work, script)`,
`SCP` and `REMOTE`, using the already verified host identity. The remote QGA module
is the repository's `scripts/bench/windows/qmp.py` staged at the path shown below;
it must expose `qga_open`, `send`, and `recv`. If using a different controller,
the equivalent remote CLI commands are written out explicitly in steps 2–4. Set `work` to the *currently running M* host directory.
Do not run the boundary controller's main(), which would launch/shutdown guests.
Use its strict `h.remote`, `h.guest`, `h.SCP` transport only, serially.

1. List up to 16 WATCHDOG dumps through `h.guest(work, script)`:
   ```powershell
   Get-ChildItem 'C:\Windows\LiveKernelReports\WATCHDOG' -Filter 'WATCHDOG-*.dmp' -File -ErrorAction SilentlyContinue |
     Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 16 Name,Length,@{n='utc';e={$_.LastWriteTimeUtc.ToString('o')}} | ConvertTo-Json
   ```
   Choose the fresh M filename and the M boot/start UTC lower bound, not the old L file.
   If no WATCHDOG file remains, inspect WER ReportQueue separately; this recipe refuses
   automatic WER selection or a stale dump. Do not restart the adapter to manufacture one.

2. Check the two trusted files in `C:\ProgramData\KayfabeDebugViewStage` with
   Get-FileHash. If missing, create **fresh** `C:\ProgramData\KayfabeDumpMTools`
   using h.guest. On the Linux host create `work+'/dump-m-tools'` using
   h.remote(['mkdir','-m','700',...]); upload the trusted ZIP, wrapper, and local
   `qga-files.py` there with h.SCP (no keys or credentials are uploaded).
   Then invoke:
   ```text
   python3 WORK/dump-m-tools/qga-files.py --helper /var/lib/kf-windows-20261005/boundary-tools/qmp.py --socket WORK/qga.sock stage WORK/dump-m-tools
   ```
   Even if the trusted Windows files already exist, upload qga-files.py to the
   fresh Linux helper directory for step 4. `stage` has two fixed destination
   filenames and verifies exact controller hashes before writing; use it only
   after explicitly creating the new Windows tools directory. No executable
   downloaded from the guest enters this tool bundle.

3. Invoke collect-m.ps1 as an encoded scriptblock, with execution-policy bypass:
   ```python
   recipe = Path('/data/kayfabe-runtime/windows-pool-20261005/probe-m-recipes/collect-m.ps1').read_text()
   # Filename must pass WATCHDOG-YYYYMMDD-HHMM.dmp validation.
   script = '& {\n' + recipe + '\n} -DumpName ' + repr(dump_name) + ' -MinimumUtc ' + repr(m_start_utc)
   encoded = base64.b64encode(script.encode('utf-16-le')).decode()
   result = h.remote(['python3', h.REMOTE+'/boundary-tools/qmp.py', work+'/qga.sock',
       'qga-exec', r'C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe',
       '-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-EncodedCommand',encoded],
       timeout=360, check=False)
   ```
   Inputs must be the simple validated filename and ISO8601 UTC timestamp, without
   quotes/backslashes. Preserve result stdout/stderr/exit. The original helper's
   qga-exec deadline is 300s; ordinary h.guest outer limit is only150s, so use the
   explicit remote timeout here. KD itself is bounded to180s (L completed62.34s).
   Its pinned wrapper checks a32MiB output threshold. This recipe exports at most
   **8MiB per file**, rejects oversize rather than truncating, preserves the fresh
   dump before KD, and copies outputs to fixed `C:\ProgramData\KayfabeDumpM` names.
   It refuses an existing directory. Exit4 means analysis failure/partial outputs;
   fetch those for diagnosis rather than rerunning automatically.
   A QGA timeout does not kill a guest process: inspect the guest's KD process and
   receipt before shutdown or rerun. The existing benchmark owner retains control of the outer lifecycle.

4. Fetch the fixed private export while Windows remains running:
   ```text
   python3 WORK/dump-m-tools/qga-files.py --helper /var/lib/kf-windows-20261005/boundary-tools/qmp.py --socket WORK/qga.sock fetch WORK/private-kd
   ```
   Run through h.remote with timeout180. This reuses the existing qmp.py protocol,
   not a competing client, and uses one persistent QGA connection with48KiB
   base64 file chunks. It validates a64KiB manifest, known filenames, <=8MiB/file,
   lengths and hashes, creates the host directory mode0700, and never overwrites.
   SCP the resulting fixed files from the PC to a fresh private controller
   directory with h.SCP. Keep export.json, receipt.json, metadata.json, result.json,
   analysis.stdout/stderr, commands.txt, wrapper.stdout, dump-source.json and
   watchdog.dmp. No driver/sys/dll/exe is fetched.

5. Only after the private evidence is durable should the benchmark owner shut down M.

## Decode and compare offline

```sh
python3 /PRIVATE/nvcd-sources/decode-nvcd.py \
  /tmp/kf-nvcd-schema-public-v1/schema.json /PRIVATE_M/analysis.stdout /PRIVATE_M/nvcd.json \
  --offset 0x1d68 --enumtag-guid 270A33FD-3DA6-460D-BA893C1BAE21E39B
python3 /data/kayfabe-runtime/windows-pool-20261005/probe-m-recipes/compare-journal.py \
  /data/kayfabe-runtime/windows-pool-20261005/probe-l/private-kd /PRIVATE_M > /PRIVATE_M/l-m-comparison.json
```

The NVCD offset is the reviewed580.88 tag profile, not a universal Windows ABI;
stop on decoder mismatch. L's outer tag was one byte short and its final record
header truncated; preserve M's integrity report too. Compare the ordered assertions
and call stacks as RVAs relative to each KD nvlddmkm loaded range, bugcheck1b0
arguments, and health's exact driver hash. Do not label `journal_assert.level=1`
an NV_STATUS. The constructor guard at L's `0x16f0481..0x16f04d2` and failure
assert at `0x16e97ef..0x16e981d` are the known ILUT follow-up points; a changed
journal is diagnostic progress, not proof that the remaining display path works.

## Reproduce the trusted bundle and validate source

```sh
mkdir -p /PRIVATE/kd-sources /PRIVATE/nvcd-sources
for name in analyze-dump.ps1 fetch-debugger.py debugger-manifest.json; do
  git show 4f27aadbdaca65b85f088c3fe0e17e5cd976f469:tools/windows-debug-capture/$name > /PRIVATE/kd-sources/$name
done
for name in decode-nvcd.py build-nvcd-schema.sh nvcd-schema.c test_nvcd.py; do
  git show eafe8a02aa3853c436c63a52ff640cbb167a306b:tools/windows-debug-capture/$name > /PRIVATE/nvcd-sources/$name
done
python3 /PRIVATE/kd-sources/fetch-debugger.py /PRIVATE/kd-bundle
bash /PRIVATE/nvcd-sources/build-nvcd-schema.sh /TRUSTED/ogkm-580.65.06 /PRIVATE/nvcd-schema
python3 -m py_compile tools/windows-debug-capture/watchdog-m/qga-files.py \
  tools/windows-debug-capture/watchdog-m/compare-journal.py
```

The fetcher retrieves pinned official Microsoft CABs and recreates the exact ZIP;
`7z` is required. The wrapper is [the pinned analyze-dump.ps1](https://github.com/reindertpelsma/kayfabe/blob/4f27aadbdaca65b85f088c3fe0e17e5cd976f469/tools/windows-debug-capture/analyze-dump.ps1), originally
introduced by `4f27aadbdaca65b85f088c3fe0e17e5cd976f469`. To validate the recipe before
first use, parse it with PowerShell's `System.Management.Automation.Language.Parser`
and require zero errors. These explicit Git object reads also work on a branch
where the earlier diagnostic files were not cherry-picked. The decoder and schema
sources are pinned at
[`eafe8a02`](https://github.com/reindertpelsma/kayfabe/tree/eafe8a02aa3853c436c63a52ff640cbb167a306b/tools/windows-debug-capture).
The underlying wrapper passed L; M's partial collection is documented separately.

## One bounded transport recovery check

The first M attempt encountered the existing QGA helper's 30-second socket read
timeout while polling the analysis process; later plain `guest-sync` pings also
failed. This is a transport failure, not proof that KD exited, reached its time
limit, or caused a guest hang. Preserve the overlay and private dump before the
benchmark's outer deadline. No automatic rerun is justified by a lost response.

`qga-resync-ping.py` copies the previously tested Vast Windows `rpc()` function
unchanged and exposes only a framing reset followed by `guest-ping`. Stop other
QGA clients first: older clients do not honor its per-socket lock. Invoke once on
the host with the already verified socket path:

```sh
timeout 12 python3 qga-resync-ping.py /RUN/qga.sock --timeout 8
```

The total eight-second budget includes lock acquisition, connect, synchronization
and ping. Responses are bounded to 8 MiB; a `0xff`-prefixed `guest-sync-delimited`
discards stale framing before the read-only ping. No guest command is executed.
A local socket test verified recovery past junk/stale responses and verified
that the only requests are the sync and ping. This does not establish recovery
of the M guest; record the actual result separately. If transport remains lost,
the owner can stop the VM, preserve its overlay/backing chain, and extract the
private dump with a read-only filesystem reader. Do not mount the writable live
guest disk from a second host process or repair the original filesystem.

## Labelled follow-up comparisons (including N)

`compare-series.py` leaves the original M recognizer unchanged and verifies its
exact source hash before importing it. It accepts a private JSON manifest of two
to four ordered cases, explicitly named and tied to caller-supplied revisions:

```json
[
  {"label":"L","revision":"b431aeaf","dump":"/PRIVATE/L/watchdog.dmp"},
  {"label":"M","revision":"d2c7ca1b","dump":"/PRIVATE/M/watchdog.dmp"},
  {"label":"N","revision":"9312854d","dump":"/PRIVATE/N/watchdog.dmp"}
]
```

Only put actual recovered N data under N. Keep the run's command/status/recovery
receipts separately; the dump comparator does not independently establish the
Kayfabe product revision. The first case must agree with the independent L KD
module range and decoded journal. Each later dump passes the same unique-name,
retail-PE identity and source-defined NVCD checks. Every adjacent comparison
reports counts, an identical prefix, a nonoverlapping identical suffix, and the
changed/intervening records, including extra tail records when counts differ.

```sh
python3 tools/windows-debug-capture/watchdog-m/compare-series.py \
  --manifest /PRIVATE/ordered-dumps.json \
  --decoder /PRIVATE/nvcd-sources/decode-nvcd.py \
  --schema /PRIVATE/nvcd-schema/schema.json \
  --driver /TRUSTED/580.88/Display.Driver/nvlddmkm.sys \
  --reference-kd /PRIVATE/L/analysis.stdout \
  --reference-decoded /PRIVATE/L/nvcd.json > /PRIVATE/labelled-comparison.json
python3 tools/windows-debug-capture/watchdog-m/test_compare_series.py
```

Before any N result, the labelled tool reproduced the validated real L→M control
exactly: 17 equal prefix assertions, one changed assertion, six equal suffix
assertions. Synthetic tests cover insertion/removal, equality and empty sequences.
The original dump bounds, partial-NVCD limits and no-live-locals caveat still apply.
