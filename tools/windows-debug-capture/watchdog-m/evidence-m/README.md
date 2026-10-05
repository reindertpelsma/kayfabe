# M passes the ILUT constructor check and stops at the TMO constructor

**STATUS: RESEARCH, 2026-10-05.** Offline analysis of the recovered M WATCHDOG
live dump. Product revision `d2c7ca1b` enables the isolated ILUT constructor
probe on top of L's TMO declaration, while retaining all-display-method refusal.
M still reports Code 43, nvidia-smi exit 9 and 365 traced RPC records. This result
shows construction progress, not working ILUT/TMO processing or Windows support.

M's fatal saved assertion moves from L's driver RVA`0x16e97f4` to`0x16e97c6`.
The first 17 of 24 normalized assertions match, and the final six propagation
assertions also match. The only differing assertion is ordinal 17 (zero-based).
Both dumps record live-dump bugcheck`0x1b0`, parameters1–3
`2`, `0xffffffffc000009a`, `0x100`. The generic failure status remains unchanged.

## What changed in the saved path

In the exact Windows 580.88 driver, SHA256
`31c79cce80b573e21647ace8d1a2b65697f992e7f86196f89e30af477b1bd47c`:

| RVA | Meaning in this path |
|---|---|
| `0x16e9675` | First call to constructor`0x16f03c0`, using the ILUT descriptor. |
| `0x16e9689` / `0x16e9692` | Tests its success byte; failure would reach L's assertion`0x16e97f4`. M reaches later code, so this check passed for the current window. |
| `0x16e96a7` / `0x16e96af` | Tests the per-window TMO predicate. A false value skips the second constructor; M enters it. |
| `0x16e972c` | Second call to the same constructor, using descriptor`rbp+0xd00+window*0xb0`. |
| `0x16e974b` / `0x16e9753` | Tests that descriptor's success byte. Zero branches to`0x16e97c1`. |
| `0x16e97c1` / `0x16e97c6` | Second-constructor failure assertion and its saved return address, found in M. |

The remaining chain is unchanged:
`16b4f33 → 1615268 → 1615285 → 1614af6 → 1959910 → 1965bd1`.
This places the current failure at the analogous TMO descriptor. No live
constructor fields or window index were recovered, so the journal alone does
not identify which guard or CPU allocation failed inside that second call.
The earlier source-defined
[ILUT/TMO descriptor analysis](https://github.com/reindertpelsma/kayfabe/blob/667b3ae9/tools/windows-boundary-compare/CONSTRUCTOR.md)
remains the starting point for a separate CAPD audit; this evidence does not
implement or authorize another capability declaration.

## Recovery and confidence limits

The new collection recipe preserved the 343710-byte original dump before KD.
After QGA stopped responding, the owner stopped the VM and recovered files
read-only from its qcow2. Original and preserved dumps have identical SHA256:
`d1fd05107c458a985cd0dc1f3e341aceded78a1fccca3b0360d2e058ad14928e`.
No clean Windows shutdown was verified. The host GPU was restored and healthy.

KD did **not** complete analysis. Metadata records 19 valid signatures and a
180-second KD budget, but stdout contains 5957 repetitions of
`kd: Could not write to pipe, 1450`. It has no bugcheck analysis, stack section,
`.enumtag` output, completion marker or recovered result.json. The timeout does
not identify why the guest or pipe stopped responding. [Collection status](collection-status.json)
retains hashes and these negative checks; no raw debugger output is public.

The raw dump remains useful independently of KD:

- The bounded recognizer discovers one `NVCD` signature afresh in each input,
  then validates its source-defined version, sizes, records and protobuf schema.
  Both happen to place it at file offset`0x41d98`; the tool does not assume that
  offset or reuse the earlier tag offset`0x1d68`.
- Re-decoding L directly from its raw dump gives exactly the same full decoded
  journal as the earlier successful KD `.enumtag` output.
- The recognizer locates a unique counted UTF-16 `nvlddmkm.sys` name and its unique
  saved module record, then checks image size, entry-point RVA, checksum and
  timestamp against the hash-pinned retail PE. The same recognition on L must
  match KD's independently printed loaded range exactly. No absolute loaded
  address is published.
- This is a bounded, checked profile for these PAGE/DU64 dumps, **not** a general
  Windows dump parser or a documented universal loader-record layout.
- Both outer NVCD records lack one declared byte and end with a three-byte
  partial record header. Whole-NVCD checksums therefore cannot be verified.
  Each contains one complete protobuf record holding the 24 assertions; unknown
  fields stay identified as unknown. Absence outside that record is not proved.

[comparison.json](comparison.json) contains only hashes, file offsets, scalar
status and module-relative assertion RVAs. Raw dumps, decoded journals, absolute
kernel addresses, driver bytes and disassembly bytes remain private.

## Reproduction

Materialize the exact trusted decoder/schema dependencies using
[the recipe's pinned-source commands](../README.md#reproduce-the-trusted-bundle-and-validate-source).
The required decoder SHA256 is
`a9c7b3f5abea4f27fceeb2b48574d4c17e8e9bb397127acade48d65ae50a10a7`;
the compiler-derived schema SHA256 is
`47aae92da4e4872693b399f75ceff2c66ff01b0a519d8532c07eee61f609fc42`.
Private inputs are not distributed in Git.

```sh
python3 tools/windows-debug-capture/watchdog-m/compare-raw.py \
  --decoder /PRIVATE/nvcd-sources/decode-nvcd.py \
  --schema /PRIVATE/nvcd-schema/schema.json \
  --driver /TRUSTED/580.88/Display.Driver/nvlddmkm.sys \
  --l-dump /PRIVATE/L/WATCHDOG-20261005-0234.dmp \
  --l-kd /PRIVATE/L/analysis.stdout --l-decoded /PRIVATE/L/nvcd.json \
  --m-dump /PRIVATE/M/watchdog.dmp > /PRIVATE/M/comparison.json
python3 tools/windows-debug-capture/watchdog-m/test_compare_raw.py
python3 tools/windows-debug-capture/watchdog-m/inspect-m.py \
  --driver /TRUSTED/580.88/Display.Driver/nvlddmkm.sys --disassemble
```

The last command prints bounded disassembly locally; its output is not committed.
Negative synthetic tests refuse altered PE identity, bad counted names, duplicated
module records, ambiguous names, truncated input, missing/duplicate NVCD markers
and an unsupported dump signature. Dump/header reads are capped at 8 MiB and
candidate/traversal budgets are explicit. Bugcheck header offsets were separately
compiler-checked against QEMU 10.2.4's
[`WinDumpHeader64`](https://github.com/qemu/qemu/blob/3e0bcba1ca7d6607ca49a988d165f052a3a53323/include/qemu/win_dump_defs.h).
