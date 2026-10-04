# Windows command surface audit

**STATUS: RESEARCH, 2026-10-04.** This audits the first validated RTX 4070 /
Windows 580.88 capture against the actual `v3-windows` branch, revision
`c50fad9ac485f53d45d4ea77a21cb7206267c65a`. It is not a product replay.

The recorder research branch has a different product ancestry and lacks that
branch's Windows identity handshake. Comparing the capture against whichever
checkout happens to contain the recorder would answer the wrong question.
The helper compiles the Windows branch's registries at ABI 580.65.06, the
source-derived Windows twin of 580.88, with the ADA display row.

Run with a clean local checkout of that revision and a local OGKM 580.159.04
source tree; no remote repository search or GPU is needed:

```sh
python3 tools/windows-gsp-trace/audit/audit.py \
  --repo /workspace/kf-windows \
  --sdk /workspace/nvidia-gpu-passthrough/research_clones/ogkm-580.159.04 \
  --capture tools/windows-gsp-trace/evidence/2026-10-04-rtx4070-580.88/gsp.jsonl.gz \
  --output /tmp/windows-command-audit \
  --target-dir /mnt/windows-work/windows-command-audit-target
```

`audit.json` contains every directly captured control, allocation class and
RPC function, plus control IDs nested in the documented deferred-API wrapper.
`audit.md` renders the inventory. `surface.tsv` holds compiled registry results;
`build.stderr` contains build diagnostics and is not part of the evidence.
The saved report lives alongside the capture. No executables are committed.

The strict existing decoder validates framing, checksums and export hashes
before the audit reads any control or allocation fields. Requests and replies
are counted independently: RPC sequence is zero throughout this fixture, so
equal totals are not evidence of pairing. Fragmented heads retain their command
IDs and declared lengths, but this audit does not reconstruct their bodies.

The reviewed policy chain is `kf-rm/src/lib.rs::served_chain`:

- Display claims are checked before later object capability denials.
- Channel controls use the public constants of the matching `ChannelPolicy`.
- Init-table lookup, allocation decoders, object control shapes and authored
  host-fact rows come from compiled code, not a list of command names.
- The recorded controls contain no additional sysmembar, ZBC or page-directory
  command beyond the already inventoried `0x90f10106`. The other policies in
  this chain answer different RPC functions or only observe commands.
- GSS/BinAPI permission alone reaches `GspRuleControlUnserviced`; a listed but
  unmodeled control reaches `UnknownControl`. Neither is counted as implemented.
- Three cached host-fact query IDs also have every captured request checked
  against the actual row's size/input selectors and serialization flag.
- An empirical identity reply and a capture-derived constant reply are
  explicitly separated from modeled commands and authored host facts.

Both the clean source revision and capture hash are pinned deliberately: this
is a reproducible audit of a reviewed snapshot. To audit another branch or
capture, review the complete policy chain, ABI/family selection and any new
wrappers before extending these pins. A numeric SDK name only identifies the
command; it does not verify a layout or establish full semantics. SDK header
hashes and matching name locations are recorded in the JSON.

## Source and Linux comparison

`catalogue.py` extends the handler audit with the [complete command catalogue](../evidence/2026-10-04-rtx4070-580.88/command-catalogue/README.md).
It preserves the distinction between native Linux ioctls, native GSP traffic and
Linux guest GSP requests answered by Kayfabe. Public descriptions and SDK layout
measurements are pinned separately from empirical observations. Absence from a
sample is never classified as Windows-only. The catalogue also corrects the
earlier inference that a routed GSP method's missing CPU handler proves Linux
cannot answer it. See its README for reproduction and remaining hardware work.
