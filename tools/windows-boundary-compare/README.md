# Repeated Windows boundary comparison

**STATUS: RESEARCH, 2026-10-05.** Offline evidence tooling; no product, driver, GPU or VM
operations. This does not enable capabilities or recommend successful replies to unknown calls.

Compare independently repeated native Windows/VFIO and Windows/Kayfabe sessions. The intended
first experiment uses three fresh overlays per arm from one immutable Windows 580.88 baseline,
the same RTX 4070, exact QEMU revision `b431aeaf`, and the explicit L diagnostic flags. Different
virtual VRAM sizes and device topology are intentional: a stable difference is not automatically
wrong. Never use these observations as captured per-die implementation tables.

```sh
python3 tools/windows-boundary-compare/compare.py /private/experiment/manifest.json \
  --json /private/experiment/report.json --markdown /private/experiment/report.md
python3 -m unittest discover -s tools/windows-boundary-compare -v
```

The CLI refuses existing output files and paths that would overwrite any input. Reports are
initially private: review supplied metadata, filenames and capability words before publishing.
RPC payloads, handles, physical addresses, queue addresses, QPCs and RPC sequences are not copied
into reports. Each input file and the manifest have exact SHA256 hashes. Output is deterministic;
no wall-clock timestamp changes a report. Hashes verify the supplied bytes, not the truth of
machine/driver identity declarations.

For the early VFIO bootstrap observer, use [native_cohort.py](native_cohort.py) to preserve
attachment generations, trigger counts and driver sampling statistics and separate source-defined
asynchronous events from the physical status queue that the decoder names `reply`:

```sh
python3 tools/drivermatrix/dm.py probe --src /trusted/ogkm-580.65.06 \
  --spec tools/windows-boundary-compare/source.spec --out /private/source-vocabulary
python3 tools/windows-boundary-compare/native_cohort.py /private/cohort-manifest.json \
  --compact --json /private/cohort.json --markdown /private/cohort.md
```

The normal manifest additionally needs `source: {"values":"/private/source-vocabulary/values.tsv",
"version":"580.65.06","commit":"307159f2623d3bf45feb9177bd2da52ffbc5ddf9"}`. Pin and check the
source checkout before compiling. [source.spec](source.spec) uses the existing compiler/DWARF
mechanism for exact RPC numbers and preprocessor/compiler values for control names. Non-integer
macros such as bit ranges appear in `missing.tsv`; they are not missing RPCs. Names are source
associations, not a forwarding policy or a claim that every private command is publicly defined.

The cohort helper additionally bounds the compared control vocabulary to 1024 IDs and each run to
64 attachment generations; oversized evidence is refused, not truncated.

Run the trusted [early-observer decoder](../vfio-gsp-observer/decode.py) first. This cohort utility
does not reauthenticate raw framing from a decoded JSON assertion. Each attachment remains
separate, including duplicated prefixes; an attachment is not proof of a distinct boot. Unknown
event numbers remain unknown, and enum range markers do not become events. There is no request/reply
pairing. Native controls outside Kayfabe's observed vocabulary are not presented as missing
implementation work. `--compact` omits full per-run censuses while retaining their hashes, stream
lengths, attachment data and all compared control counts/statuses. The native-repeat strata include
QEMU executable hash and artifact revision, in addition to the ordinary identity fields; supplied
metadata should include both when available. The older general comparator still presents aggregate
physical `reply` observations; use the cohort utility for event and attachment separation.

## Manifest

Start with [manifest.example.json](manifest.example.json), replace its placeholders, and add runs
2 and 3 for each arm. Paths are relative to the manifest directory; absolute paths also work.
Each run needs a unique `id`, `arm` (`vfio`/`kayfabe`), explicit `variant`, `flags`, and
`instrumentation` note. Keep instrumentation wording identical for genuinely identical setups:
different instrumentation, variant, flags or declared identities create separate groups.

`experiment` declares `baseline_sha256`, `windows_driver`, `driver_hash`, `gpu`, and
`qemu_revision` as supplied strings. `metadata` may repeat these fields from independently
collected per-run evidence and may record additional facts such as VRAM size, topology, status,
runner revision or elapsed wait. Missing identity fields inherit the experiment declaration with
an explicit warning. Differing identity fields separate groups and warn that the run is not
matched. The utility does not inspect driver executables or trust shortened Git IDs as cryptographic
proof. Metadata remains a supplied assertion. `driver_hash` accepts the caller's hash notation.

Each run has `rpc`, `display_caps`, or both. Missing sections stay unavailable, never a count of
zero. Every present section requires a provenance/coverage `note`.

- `rpc`: `format`, `path`, `coverage` (`partial` or `complete-window`), and a named `window`.
  Only Kayfabe can declare `complete-window`; this asserts logging completeness for that named
  interval, not all driver activity. Native KGWT is always partial. Different windows are compared
  separately. An empty, valid supplied capture has zero **observed** records.
- `display_caps`: `path` to **exactly 4096 raw bytes**, `source` (the present experiment uses
  source-derived `BAR0+0x640000`), and `milestone` describing the identical fixed startup/status
  sampling point. The comparator does not perform a BAR read. It only compares equal declared
  sources/milestones. This is one snapshot at the milestone, not a trace of intermediate writes.

## RPC formats and limits

`kf-rm` accepts the `kf-rm: rpc-trace` text protocol from
[`census.rs`](../../crates/kf-rm/src/census.rs). Unrelated log lines are counted and ignored;
malformed lines containing that marker are errors. Numeric function, control/class ID and returned
status define an observation. `cmd=undecodable`/`class=undecodable` stay unknown. `result=none`
stays null and is **not** converted into `0x56` or success. This logger observes answers at its
own boundary; it does not establish that every possible driver action is an RPC.

`kgwt-decoded-json` accepts the existing trusted native observer decoder's
`kayfabe-gsp-observer/1` output with `complete:false`: either `--all-records` CLI output
(`observations` array plus integer `records`) or its `parse()` result (`records` array). First
run that decoder to validate raw framing, checksums, selected-export metadata and gaps:

```sh
# Use the decoder from the pinned Windows GSP observer source checkout.
python3 /trusted/observer/tools/windows-gsp-trace/decode.py /private/run/gsp.jsonl \
  --jsonl --all-records > /private/run/decoded.json
```

The original raw `kayfabe-gsp-text/1` JSONL and binary KGWT are intentionally rejected here;
this utility does not duplicate their ABI/checksum parser. The current decoder may include
`text_export` source hash/selection/omission provenance, which is retained. Native JSON can be
large because the decoder includes raw payloads; this comparator drops those payloads.

`kgwt-decoded-jsonl` is a compact interchange for already decoded observations. Its first line is
`{"schema":"kayfabe-gsp-observer/1","kind":"header","complete":false,"records":N}`;
exactly N following JSON objects use the same decoded record fields. Required fields are
`direction` (`request`/`reply`), `rpc_function`, `rpc_status`, `missing_before`, and
`prefix_unknown`. Optional `control` uses `command` (or `cmd`), `status`,
`params_bytes_declared`, `params_bytes_observed`, and `params_complete`. Numbers may be uint32
integers or hexadecimal strings. `payload_hex`, `params_hex`, or `data_hex` are never interpreted.
The exact line count detects a missing suffix; `complete:false` still describes partial sampling,
not a corrupt file. Requests/replies, outer RPC status and inner control status remain distinct.
Truncated observed parameters are allowed only when explicitly marked incomplete.

Bounds: 32 runs, 1 MiB manifest, 64 MiB per RPC file, 100000 records per file, 250000 records
across the experiment, and 256 KiB per text-log/decoded-JSONL line. Capability pages are fixed
4096 bytes. Duplicate JSON keys, invalid integer ranges, inconsistent native record counts,
claimed complete native sampling, and malformed records fail rather than silently disappearing.
These are offline resource limits, not product guest-input acceptance rules.

## What the report means

Within each arm/variant/identity/instrumentation group and capture window, report exact observed
counts by function/selector/status, min/max counts, complete normalized order hashes, and the first
order divergence from that group's first run. Native request and reply streams are independent;
there is no pairing based on handles or usually-zero `rpc_sequence`, and no deduplication that
could erase repeated calls. Native retained records may predate collector start and cannot prove
submission timing or initialization completeness. A zero native count **never proves no call**.

Cross-arm RPC rows compare observed function totals and decoded control IDs, with separate native
request/reply and Kayfabe completion counts/statuses. Allocation classes are only available in the
Kayfabe logger, so cross-arm fn103 comparisons stay aggregate. We do not align messages across
arms, equate status namespaces, or infer which absence caused a failure.

Each capability word is little-endian uint32 at its byte offset. All 1024 offsets and every run's
raw value remain in JSON. A word is `stable` only with at least two equal observations in that arm;
one sample is `single-observation`. Differing values are `variable`, **not classified as noise**.
Missing snapshots are null. Stable cross-arm differences are listed only for equal declared
source/milestone pairs, with identity-match status; all variable values remain available.

Markdown is a bounded overview (first 32 variable words and 64 stable cross-arm differences per
section); JSON contains every word/event, the full per-run values, hashes, flags and coverage
notes. Repetition distinguishes reproducible observations from variation. It does not establish
causality, cross-die behavior, or authority to fabricate driver-visible success.

## Source-based capability annotations

`annotate_caps.py` reads the existing compiler-generated class TSV, rather than parsing C macro
bodies or learning register layouts from captures. Select the capability class explicitly:

```sh
python3 tools/windows-boundary-compare/annotate_caps.py --class C773 \
  --page vfio-1=/private/vfio-1/display-caps.bin \
  --page kayfabe-1=/private/kayfabe-1/display-caps.bin \
  --json /private/caps-named.json --markdown /private/caps-named.md
```

`--table` selects another explicitly derived TSV; its version and exact hash are recorded.
Scalar register offsets, array base/stride/count, fields, and enum labels all come from that table.
The existing C373/C573/C673/C773/CA73 capability classes are covered; unknown classes and method
classes whose registers exceed this page fail closed. Header `*_INIT` labels are reported as
names, not claimed valid or required runtime values. Unknown/uncovered bits stay raw. All 1024
words and every page value remain in JSON; Markdown names differences in the first128 differing
words. This helper does not group samples into experimental arms or infer stability; use the
main comparator for that. Bounds are32 pages, exactly4096 bytes each, and4MiB for the source table.

For this audit, rerunning `tools/derive_display_classes.sh` against clean public OGKM580.65.06
reproduced the committed table byte-for-byte (SHA256
`0f434f1b996054f1832fd15087e26881d8565df75c641806a7d98cbc39cb5449`). The source is commit
`307159f2623d3bf45feb9177bd2da52ffbc5ddf9`. The [constructor note](CONSTRUCTOR.md) separates
public field definitions, pinned Windows dataflow, and research-only hypotheses. None of the
annotation output is a product per-die table.
