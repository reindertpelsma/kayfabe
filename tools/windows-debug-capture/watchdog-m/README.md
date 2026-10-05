# Bounded WATCHDOG collection for probe M

**STATUS: RESEARCH, 2026-10-05.** Prepared offline. No PC/QGA/VM/GPU access was performed. PowerShell AST
and Python compilation pass; journal normalization reproduces all 24 L assertions
when compared with itself. New transfer/collection orchestration is not yet runtime-tested.

These three committed scripts are byte-identical to the originally prepared
controller recipes; their exact hashes are in [source-sha256.json](source-sha256.json).
Only sources and documentation are stored here. Dumps, debugger binaries, full
journal output and the local self-comparison result are deliberately excluded.

The absolute paths below record the original experiment's controller/host layout.
Substitute a private controller output directory and the currently running guest's
work directory when reproducing. `collect-m.ps1` retains fixed Windows paths and
refuses reuse. The source code never initiates a VM, restarts an adapter, or shuts
down a guest.

Trusted inputs used on the original controller:
- `/data/kayfabe-vfio-gsp-observer-20261005/tools/windows-debug-capture/analyze-dump.ps1`
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
python3 /data/kayfabe-vfio-gsp-observer-20261005/tools/windows-debug-capture/decode-nvcd.py \
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
python3 tools/windows-debug-capture/fetch-debugger.py /PRIVATE/kd-bundle
python3 -m py_compile tools/windows-debug-capture/watchdog-m/qga-files.py \
  tools/windows-debug-capture/watchdog-m/compare-journal.py
```

The fetcher retrieves pinned official Microsoft CABs and recreates the exact ZIP;
`7z` is required. The wrapper is [../analyze-dump.ps1](../analyze-dump.ps1), originally
introduced by `4f27aadbdaca65b85f088c3fe0e17e5cd976f469`. To validate the recipe before
first use, parse it with PowerShell's `System.Management.Automation.Language.Parser`
and require zero errors. The transfer and collection orchestration has no Windows
runtime claim yet; the underlying pinned KD wrapper has the earlier L result.
