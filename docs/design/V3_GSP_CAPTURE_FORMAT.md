# V3: the GSP queue capture format (`KFGSPCAP`) and the census tool

**STATUS: DESIGN-ONLY for the format, 2026-10-13; the scrub stage (section 8) and the golden capture (section 9) are LIVE tooling, added the same day.** The format and the decoder (`crates/kf-census`) exist and are tested GPU-free; the module (`tools/windows-capture`, branch `claude/windows-capture-module-20261013`) now exists as code but nothing was captured in this format on a real GPU, and the module's emit path below is a design. Step 1 of `V3_WINDOWS_ONLY_CONSTANTS_20261013.md` (capture module on rented GPUs, decoder, census); strategy in `V3_CONSTANT_STRATEGY.md` s4. Labels: **[M]** measured (a command run for this page), **[R]** read in source, **[I]** inferred. An inference is never written as evidence.

## 1. What the module does, and what it must not do

The Windows capture module copies the driver<->GSP message-queue entries into a log. It reads two rings that the driver itself published (`msgqTxHeader` + entries, `ogkm-580: src/common/shared/msgq/inc/msgq/msgq_priv.h:49-65`; element = 48-byte header + RPC envelope, `message_queue_priv.h:43-51` [R, per `kf_abi::versions::GspElementWire`]). It does **not** parse elements, pair requests, filter, or interpret anything. Every decision about meaning is made offline by `kf-census`. That is what makes it small enough to audit and to write in kernel C: a module that parses can mis-parse, and a mis-parse is a silent hole in a dataset that exists to find holes.

The format is **append-only**, **fixed-size headers**, **little-endian**, **versioned**, with an **explicit loss marker** and **no board-specific field**.

## 2. The format in ten lines

```
FILE   = FILE_HEADER(128 B) RECORD* [END record]
FILE_HEADER (all LE):  0 magic "KFGSPCAP"[8] | 8 version u32 =1 | 12 header_bytes u32 =128 | 16 record_header_bytes u32 =32
   20 flags u32 =0 | 24 os_id u32 (1 Windows, 2 Linux) | 28 module_version u32 | 32 pci_vendor_id u16 | 34 pci_device_id u16
   36 pci_revision_id u8 | 37 reserved[3]=0 | 40 ts_hz u64 | 48 ts_start u64 | 56 driver_version char[32] | 88 decode_tag char[32] | 120 reserved u64 =0
RECORD = HEADER(32 B) PAYLOAD(payload_bytes):  0 magic "KREC" u32 | 4 kind u8 | 5 dir u8 (0 cmd queue, 1 status queue) | 6 queue_id u16
   8 payload_bytes u32 | 12 counter u32 (0,1,2,... per file) | 16 ts u64 (monotonic ticks) | 24 first_entry u32 | 28 entry_count u32
kind 1 BIND      payload = the queue's raw 32-byte msgqTxHeader, copied verbatim (defines entry size and ring size for (queue_id, dir))
kind 2 ELEMENTS  payload = entry_count consecutive ring entries starting at ring index first_entry (entry_count * msgSize bytes, raw)
kind 3/4/5 LOSS  OVERFLOW (module buffer full) / UNREADABLE (copy faulted or unstable) / UNKNOWN; first_entry = where, entry_count = entries lost or 0xFFFFFFFF
kind 6 END       clean close; entry_count = number of records written before it
```

`driver_version` is the string the module read from the guest, verbatim (printable ASCII, at most 31 characters; Windows may give a form like `32.0.15.8088`, the cell normalises it). `decode_tag` is the ogkm tag the cell decodes with (for example `580.82.07`); the module copies it from its configuration and may leave it empty, and `kf-census --ogkm-tag` overrides it. The tag must be one of `kf_abi::generated::matrix::MEASURED`: an unmeasured tag is refused by name, never mapped to a neighbour. Source of truth for the layout is `crates/kf-census/src/capture.rs` (`FILE_FIELDS`, `RECORD_FIELDS`, the tables that tile the headers byte for byte).

### Why these choices

- **Raw entries, not messages.** The module knows only that entries `[first, first+count)` of a ring changed hands. A message occupies `elemCount` entries (580: `elemCount@40` of its first element; 610: derived from `rpc.length`); the decoder reassembles across record and ring-wrap boundaries. A module that cut at message boundaries would have to read `elemCount`, which is parsing.
- **BIND copies the ring header instead of the module reading geometry.** `msgSize` and `msgCount` come from the driver's own header (`kf_gsp::TxHeader`), so no geometry constant exists in the module or the format (derive, never capture).
- **`counter` and `ts` per record.** The counter makes a lost *record* (module or disk fault) visible even when no loss marker could be written. Timestamps are monotonic ticks since an arbitrary origin; they are information (and a regression is flagged), not an ordering key between the two queues.
- **Loss is a record, not an absence.** `LOSS_*` says where and how much if known. The decoder still checks continuity independently, so a loss the module failed to announce is caught, and a loss it did announce is counted (never swallowed).
- **No parsing, no allocation of unbounded size in the module.** Fixed 32-byte headers, payload bound `MAX_PAYLOAD_BYTES` = 1 MiB (one ring), strings fixed 32 bytes.

### Module emit path (design, for the kernel author)

At module load: read PCI vendor/device/revision from config space, read the driver's version string, write the file header. For each queue pair the driver creates: write BIND for both rings (copy 32 bytes), keep `last_seen` index per ring. At each sampling point (the doorbell write to the command queue head, the status-queue interrupt, a timer): read the ring's `writePtr`, copy entries `[last_seen, writePtr)` as one ELEMENTS record, split at the ring end (two records) and at the buffer bound; if the copy faults or the producer overtook the module (`writePtr - last_seen` exceeds the ring), write LOSS_UNREADABLE / LOSS_OVERFLOW with the entry range and resynchronise at `writePtr`. On unload write END. Nothing else. (The existing Windows scanner and VFIO observer sample at "causal boundaries" the same way: `tools/vfio-gsp-observer/README.md`.)

## 3. What the format contains, and what it cannot keep out

**The headers carry no board identity.** No GPU UUID, serial, board part number, VBIOS or ROM content, MAC, PCI bus/device/function, PCI subsystem ids, IP or host name. Only: vendor/device/revision (the die), the driver and tag strings, the OS, the clock rate. `crates/kf-census/tests/oracle.rs::the_format_has_no_board_specific_field` pins this: the field tables tile the headers with no unnamed byte, no field name matches a banned word, reserved bytes must be zero (a set reserved byte makes a file unreadable), and the two strings accept printable ASCII only.

**The payload is the driver's queue content and can contain board data inside message bodies** [I]: controls such as the GID/UUID query (`0x2080014a`), static-info and VBIOS-related replies. The module cannot remove them without parsing, so the rule is policy, not format:

1. capture files are **host-only** (like the observer's `gsp.jsonl` today: `/var/lib/kf-windows-20261005/...`, mode 0600); never committed, never attached to a doc;
2. what leaves the host is the **census**: `kf-census` writes ids, counts, sizes, statuses and the *number* of distinct reply bodies (never a digest, never a byte) (`the_census_outputs_carry_counts_and_sizes_but_no_message_bytes` plants a serial-looking reply and checks no output contains it);
3. any step that does need values (the later "constants" extraction) is a separate, reviewed tool with an explicit per-control allow-list; it is out of scope here (open question 1).

> **⊘ UPDATE 2026-10-13.** That tool's first half now exists: `kf-census scrub` (section 8) rewrites a capture default-deny, keeping headers and the bodies of allow-listed controls only, and the census reports scrubbed bodies as such. Rules 1 and 2 above are unchanged; the content of the allow-list is an owner decision (section 8.6).

## 4. The decoder and the oracle (`crates/kf-census`)

`kf-census [--ogkm-tag 580.82.07] [--tier1 docs/design/tier_checks/tier1_ogkm.tsv] [--out PREFIX] INPUT...` reads observer `gsp.jsonl` (sniffed by a leading `{`) and `KFGSPCAP` files, `-` for stdin. It writes `PREFIX.controls.tsv`, `.functions.tsv`, `.anomalies.tsv`, `.json` and exits 0 only if the oracle found nothing.

**Reuse, not re-implementation.** Element framing, the 8-byte XOR checksum, `elemCount`/length, the envelope and the control header are `kf_gsp::element::{peek_seq_num, peek_len, decode_message}`, `kf_abi::versions::DriverAbiTable::{decode_rpc_envelope, decode_rpc_control, rm_control_wire}`, built by `kf_rm::abi::gsp_abi_for(tag)`. Large controls (a head plus `CONTINUATION_RECORD`s) are joined by `kf_gsp::large::Assembler`, the transport's own joiner. Function and control names come from the driver matrix (`kf_abi::generated::matrix`, measured per tag). No NVIDIA layout is typed in the new crate.

**The oracle** (every item is an anomaly kind; counts are exact, only stored examples are capped at 12 per kind):

| check | anomaly kinds |
|---|---|
| sequence numbers continuous per (source, queue instance, direction), starting at 0 | `seq_gap` (forward jump, missing messages summed), `seq_rewind` (backwards or repeated), `prefix_missing` (first number is not 0) |
| every message parses: checksum folds to zero, length within bounds, envelope signature, header version, control header, declared params fit | `parse_failure`, `elem_count_mismatch`, `header_version`, `params_truncated`, `control_extra_bytes` |
| no trailing bytes: an observer message is exactly its declared length rounded to 8; a capture stream ends on a message boundary; a message's run is exactly `elements * msgSize` | `trailing_bytes` |
| every function id is defined by ogkm at the decode tag | `unknown_function` |
| the source framing is intact | observer: `bad_line`, `footer_problem` (header/footer present, `records` and `source_bytes` equal what was read), `observer_loss` (`missing_before`, `driver_stats.dropped`), `meta_mismatch` (observer fields vs the payload); capture: `capture_loss`, `counter_gap`, `timestamp_regress`, `stream_truncated`, `end_missing`, `end_mismatch`, `bad_bind`, `bad_elements`, `discontinuous_entries`, `desync_entry` |

Any anomaly makes the exit status 1. Gaps are never repaired or skipped: after a gap the analyzer resynchronises on the number it saw, counts the gap, and keeps decoding. A late message is one `seq_gap` (when the jump happened) plus one `seq_rewind` (when it arrived); a duplicate is one `seq_rewind`.

**Request/reply pairing.** [M, `rpc_sequence_nonzero = 0` in all captures run, section 6] the Windows 580.88 driver writes `rpc.sequence = 0` in every message, so the RPC sequence number cannot pair a control with its reply. The queue sequence numbers are per queue, not per transaction. The pairing key is therefore `(queue instance, rpc.sequence, hClient, hObject, cmd)` taken FIFO per key (the GSP is a single in-order processor), independent of how the two queues interleave in the file. Leftovers are counted per control as `unanswered` and `orphan_replies`; they are reported under pairing, not as oracle failures (a capture may legitimately stop with a request in flight). When a driver does number its RPCs, `rpc.sequence` enters the key automatically.

**Tests** (`crates/kf-census/tests/oracle.rs`, 27, GPU-free, synthetic fixtures built with `kf_gsp::encode_message`): valid stream (both formats, and the two formats give byte-identical census TSVs), gap, missing prefix, reordered, duplicate, truncated element, file cut inside a record, message missing its last element, unknown function id, corrupted message, observer metadata disagreeing with the payload, bad footer, loss marker, unmarked ring jump, record-counter gap, missing End, large control join, an overlong/garbage observer line, the binary's exit status, the round trip below, and the no-board-data checks. Round trip: 100+ messages (single- and multi-element, past the ring end) are encoded by the transport encoder, written as a capture in chunks of 3 entries, read back, and every reassembled message run equals the encoded run byte for byte; decoded function, sequence and payload equal what was sent. Run: `CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 cargo test -p kf-census`.

**Census columns** (per control id): `requests`, `replies`, `paired`, `unanswered`, `orphan_replies`, `echo` (paired where reply params equal request params byte for byte), `serialized` (requests with the serialized-params flag), `req_sizes` / `reply_sizes` (declared `paramsSize` histograms, `value:count`), `statuses` (reply control `status`), `rpc_results` (reply envelope), `distinct_replies` (count of distinct reply bodies), `scrubbed` (messages whose body was scrubbed, section 8; such bodies are left out of `echo` and `distinct_replies`; 0 for an unscrubbed file), `ogkm` and `ogkm_basis`. `--tier1` may be given more than once; the first table that knows an id wins.

**The ogkm column** answers from data the repo already holds (`--tier1` takes `tier1_ogkm.tsv` and `tier1_ogkm_captured87.tsv`, which together cover all 221 ids of the five captures) and says `unknown` when it has none: `defined` + `matrix:NAME` if the driver matrix has the control at the tag (131 ids kayfabe consumes); `defined` / `absent_all_tags` from `tier1_ogkm.tsv` (221-tag search over the 140 inventory rows; `NOT_IN_ANY_VERSION` there means no tag defines that number, with the file's own convention for number collisions); otherwise `unknown`, which is **not** `absent`.

## 5. Assumptions that a first real capture could falsify

1. [I] Consecutive ELEMENTS records of a queue are contiguous modulo `msgCount`, and a multi-element message never skips ring tail entries. Falsifier: `discontinuous_entries` or `desync_entry` on a real module capture with no loss marker.
2. [I] The padding after a message inside its last entry is zero (the sender zero-fills, as `kf_gsp::encode_message` does). Counted, not failed: `nonzero_padding`.
3. [I] The per-direction element `seqNum` starts at 0 for each queue instance. [M] true for every queue instance in the five observer captures (section 6: no `prefix_missing`, no `seq_gap`).
4. [I] A message declared larger than one queue message is joined from `CONTINUATION_RECORD`s by the same rule in both directions. [M] 30 such controls per direction over the five captures (60 joins) decode with no `params_truncated` and no `control_extra_bytes`.
5. [I] The 580.82.07 tables decode a 580.88 Windows driver. 580.88 is not a measured ogkm tag; the element header and `rpc_gsp_rm_control` layouts are the same at all measured 575..615 tags (`RmControlWire` doc), and section 6 repeats the run with a second tag (580.94.02): [M] the controls and functions TSVs are byte-identical.

## 6. Results: the census over the five real captures

**Inputs.** The five observer captures of the Windows 580.88 guest on one RTX 4070 / AD104 (`vfio-dvi-20261008/boot1..3`, `vfio-gl-20261011/boot1..2`, host-only `gsp.jsonl`, read-only, copied to a scratch directory and deleted afterwards). Command: `kf-census --ogkm-tag 580.82.07 --tier1 docs/design/tier_checks/tier1_ogkm.tsv --out all5 dvi1 dvi2 dvi3 gl1 gl2` (about 21 s unoptimised). Everything in this section is [M] unless marked [I].

**Totals.** 34 833 messages in the command queue, 35 200 in the status queue, 26 469 `GSP_RM_CONTROL` requests and 26 467 replies, 221 distinct control ids, 60 large controls joined from a head plus `CONTINUATION_RECORD`s (30 per direction). Function mix (command direction): 26 469 control, 5 248 alloc, 3 031 free, 30 continuation, and one-offs for the boot RPCs; the status queue adds 359 `POST_EVENT` and 5 `GSP_INIT_DONE` events. `rpc.sequence` is 0 in every message of every capture (0 non-zero of 70 033), which is why pairing is keyed as in section 4.

**Oracle.** Four captures PASS (`dvi1`, `dvi2`, `dvi3`, `gl2`: no gap, no parse failure, no trailing bytes, no unknown function, nothing in the footer). One does not: `gl1` carries the observer's own loss admission, `driver_stats.dropped = 1` and `limited = true` (it stopped recording at its cap), and 2 requests at its tail (`0x2080a0a8`, `0x2080a0d1`) have no reply in the file. The sequence oracle finds **zero** gaps in any of the five (the observer's own `sequence_gaps` is 0 in all five too), which is consistent with the loss being at the tail where no later message can reveal it. [I] The `gl1` tail is cut by the observer; its counts are lower bounds. This is a finding about the capture that no earlier number in the repo stated.

### Falsifier: census against the inventory's measured numbers

The inventory (`v3_constant_inventory.tsv`, "M:" notes, produced by the tier-2 desk check with separate scripts and FIFO pairing on `(client, object, cmd, size)`) against `kf-census` over the same five files; the last column is an independent 20-line Python recount straight from the hex (no kf code: `cmd` at payload +88, `status` +92, `paramsSize` +96).

| control | inventory: calls | inventory: reply B | inventory: status | kf-census: requests / replies / paired | kf-census: reply B | kf-census: status | python recount | agrees |
|---|---:|---:|---|---|---:|---|---|---|
| `0x2080852a` | 24 | 1784 | 0x0 x24 | 24 / 24 / 24 | 1784 x24 | 0x0 x24 | 24, 1784, 0x0 | yes |
| `0x2080852e` | 19 | 528 | 0x0 x19 | 19 / 19 / 19 | 528 x19 | 0x0 x19 | 19, 528, 0x0 | yes |
| `0x20809004` | 640 | 1544 | 0x0 x640 | 640 / 640 / 640 | 1544 x640 | 0x0 x640 | 640, 1544, 0x0 | yes |
| `0x00730128` | 17 | 20 | 0x0 x17 | 17 / 17 / 17 | 20 x17 | 0x0 x17 | 17, 20, 0x0 | yes |
| `0x007302a5` | 157 | 16 | 0x0 x157 | 157 / 157 / 157 | 16 x157 | 0x0 x157 | 157, 16, 0x0 | yes |
| `0x2080a02f` | 5 | 1 | 0x56 x5 | 5 / 5 / 5 | 1 x5 | 0x56 x5 | 5, 1, 0x56 | yes |
| `0x00809908` | 70 | 16 | 0x0 x70 | 70 / 70 / 70 | 16 x70 | 0x0 x70 | 70, 16, 0x0 | yes |

**No discrepancy.** All seven rows agree on call count, reply size and status in three independent counts (the inventory's scripts, this decoder, the Python recount), so there is nothing to explain and no decoder adjustment was made. Further agreement with the inventory's and tier-2's other statements: `0x20809004` has 118 distinct reply bodies in 640 calls (tier 2: "118 distinct replies in 640 calls"); reply == request in every call for `0x20801111` (565 calls), `0x20801211` (282) and `0x20801208` (159) (tier 2 s3 item 5 and the inventory: same three counts); run with `--ogkm-tag 580.94.02` the controls and functions TSVs are byte-identical to the 580.82.07 run, and a 595.91.07 run over `dvi3` also passes the oracle. What this does **not** show: it cannot catch an error common to every reader of these files (they share the observer's bytes; the per-message checksums and sequence continuity are the only internal evidence that the bytes are what the driver sent).

### What the captures contain

Top controls by call count (requests over the five captures): `0x0073010b` 9 800 (display head routing map), `0x20808539` 1 571, `0x20809037` 1 571, `0x2080a0a8` 1 566, `0x2080a0d1` 1 566, `0x20809004` 640, `0xc3720101` 623, `0x20801111` 565, `0x20800a38` 482, `0x2080110b` 441. Reply control status: 26 267 of 26 467 are 0x0; the rest are 0x56 (108), 0x1f (60), 0x57 (12), 0x14 (5) and 0xffff (15). `serialized` is 0 for every request (no control used the serialized-params flag).

> **⊘ UPDATE 2026-10-13.** The 87 `unknown` below were searched over all 221 tags (next subsection but one, *the 87 `unknown` controls*): 49 are defined, 38 are absent, none is unknown; overall 157 defined, 64 absent. The paragraph is kept as written, with its counts as of the first run.

**Controls that appear in the captures that ogkm lacks** (by the repo's own tier-1 search, 221 tags): 26 control ids, 1 643 calls: `0x20809004` (640), `0x20801111` (565), `0x007302a5` (157), `0x00809908` (70), `0x2080852a` (24), `0x2080a618` (24), `0x2080852e` (19), `0x00730128` (17), `0x20810108` (16), `0x20809001` (15), `0x2080a088`, `0x2080a801`, `0x2080b201` (10 each), `0x00730122` (7), and the 5-call group `0x007302a3`, `0x20802806`, `0x20808524`, `0x2080853a`, `0x20809009`, `0x20809064`, `0x2080a02f`, `0x2080a060`, `0x2080a630`, `0x2080a70a`, `0x2081010d`, plus `0x2080852f` (4). Of the 221 ids, 108 are `defined` (matrix at the tag, or tier 1), 26 `absent_all_tags`, **87 `unknown`** (19 236 calls, including the five most-called ids): neither source covers them because the tier-1 search only ran over the 140 inventory rows and the matrix only holds the 131 controls kayfabe consumes. `unknown` does not mean absent.

[M, supplementary, single tag, presence only; not part of the tool] A scan of `#define NV*CTRL_CMD*` in `/data/nouveau-trace-research/ogkm-575.51.03/src/common/sdk/nvidia/inc` finds 48 of those 87 ids defined at 575.51.03 (11 849 calls; for example `0x0073010b` = `NV0073_CTRL_CMD_SYSTEM_GET_HEAD_ROUTING_MAP`, `0x0073010c` = `..._SYSTEM_GET_ACTIVE`); 39 ids (7 387 calls) have no such define at that tag, among them `0x20808539`, `0x20809037`, `0x2080a0a8`, `0x2080a0d1` and `0x2080b202`. A one-tag scan can show presence, not absence (class-method controls such as `0xc3720101` do not use that define pattern); the 39 stay `unknown` until the 221-tag search of tier 1 is run over them (open question 5).

### The 87 `unknown` controls, searched over every tag (open question 5, answered)

**STATUS: answered, 2026-10-13.** The earlier paragraph ("Controls that appear in the captures that ogkm lacks") says 108 defined, 26 absent, 87 `unknown`. All 87 were searched over **every one of the 221 tags** (`tier_checks/ogkm_search_ids.py`, a bare clone of `NVIDIA/open-gpu-kernel-modules`; its `git tag` list is identical to `tier_checks/ogkm_evidence/tags_searched.txt`, 515.43.04 .. 615.78.08), by three independent scans: (P3) every `#define NV*_CTRL_* (0x...)` line of every header, parsed by value, so a number defined under another spelling or name is found; (P1) the number in any spelling anywhere in the tree, generated code included (`0x0*<hex>`); (P2) for ids with no P1 hit, a raw scan with binary files included, hex and decimal. Struct sizes: the params struct named in the `finn:` comment (else by naming convention) compiled with `gcc -S` at **each** tag where the id is defined.

Result [M]:

| | ids | calls in the five captures |
|---|---:|---:|
| defined in at least one tag (all 49 in at least one 580.x tag) | 49 | 11 922 |
| absent from every tag (P3, P1 and P2 all found 0 lines in 221 tags) | 38 | 7 314 |
| left unknown | 0 | 0 |

- Layout against the observed reply size at the 580.x tags: **43 MATCH**, 6 have no params struct and were observed with 0 B, **0 MISMATCH**. (Size equality only; field offsets and values were not compared.)
- Two numbers are different controls in different tags (**number collisions**): `0x00730108` is `SYSTEM_GET_CONNECT_STATE` (16 B) from 555.42.02 and `SYSTEM_GET_SCANLINE` (20 B) before; `0x20801357` is `FB_GET_STATUS` at 570.86.15..570.211.01 and `FB_QUERY_DRAM_ENCRYPTION_INFOROM_SUPPORT` from 575.51.02. The 580.x meaning matches the observed size in both.
- `0x00801812` (`NV0080_CTRL_DMA_SET_DEFAULT_VASPACE`) is defined in all 221 tags under a name without `_CMD_`; a scan that needs `CMD` in the name misses it. That is the one-tag difference between the earlier supplementary scan (48 defined, 39 not, 575.51.03 only) and this one: 39 = 38 + this id, 48 + this id = 49.
- Totals over the five captures: **157 of 221 control ids are defined in some ogkm tag (108 + 49), 64 are absent from every tag (26 + 38), none is unknown**. The census, run with `--tier1 tier1_ogkm.tsv --tier1 tier1_ogkm_captured87.tsv`, reports exactly these numbers (157 `defined`, 64 `absent_all_tags`, 0 `unknown`).
- The 38 absent ids, most-called first: `0x20808539` (1 571 calls), `0x20809037` (1 571), `0x2080a0a8` (1 566), `0x2080a0d1` (1 566), `0x2080b202` (421), `0x20809029` (101), `0x20808159` (70), `0x2080902a` (68), and `0x0073011a`, `0x0073117a`, `0x2080852b`, `0x2080852c`, `0x20808530`, `0x20808537`, `0x2080880f`, `0x20809019`, `0x2080901b`, `0x2080902b`, `0x2080902c`, `0x20809063`, `0x2080a026`, `0x2080a028`, `0x2080a079`, `0x2080a080`, `0x2080a081`, `0x2080a095`, `0x2080a0a7`, `0x2080a0c4`, `0x2080a0c5`, `0x2080a0c8`, `0x2080a0cc`, `0x2080a612`, `0x2080a637`, `0x2080b209`, `0x2080b210`, `0x2080b216`, `0x2080d02d`, `0x2080e0af`. Together with the 26 of tier 1 these are the controls the Windows driver issues that no open source defines: 64 ids, 8 957 of the 26 469 control calls in the five captures. The 34 Windows-only ids of `V3_WINDOWS_ONLY_CONSTANTS_20261013.md` were searched in the same run: 33 absent from every tag, 1 (`0x00730122`) defined only before 555.42.02 as a different 16-byte control (tier 1's number collision; it keeps its `NOT_IN_ANY_VERSION` row there).
- Files: `tier_checks/tier1_ogkm_captured87.tsv` (87 rows; loads with `--tier1`; verdicts `DEFINED_580X` / `NOT_IN_ANY_VERSION`, columns as `tier1_ogkm.tsv`), evidence in `tier_checks/ogkm_evidence/captured_ids_search.{defs,hits,p2,summary}.tsv` (118 ids: the 87 plus the 31 Windows-only ids not among them), the ids in `captured87_ids.txt` / `captured_ids_searched.txt`.
- **Falsifier**: one line in any tag for one of the 38 ids (they would appear in `captured_ids_search.hits.tsv` or `.p2.tsv`, both empty for them), or a layout MISMATCH between an id's struct at 580.x and the observed reply size.
- Not shown [I]: "absent from every tag" means no open-source definition at a *tag* (untagged commits were not searched, as in tier 1), not that the driver or the firmware lacks the control; "defined" gives a name and a layout, not values (tier 1 shows most replies are produced by firmware: `ROUTE_TO_PHYSICAL`). The Windows driver (580.88) is not itself a tag.

### Not verified

- Nothing was captured in the `KFGSPCAP` format by a real module; the format is exercised only by synthetic streams produced by `kf_gsp::encode_message` (section 5 items 1 and 2 remain untested against real ring behaviour).
- The Windows driver is 580.88 and no measured ogkm tag has that number; the identical output at 580.82.07 and 580.94.02 is evidence the tag choice does not matter *within 580.x for these captures*, not for other driver families.
- The footer sha-256 digests of the observer files are not recomputed by this tool (the counts, byte totals, per-message checksums and sequence continuity are).
- One AD104 die, one driver, one monitor: nothing here is a per-die or per-driver comparison.

## 7. Open questions for the owner

1. **Payload scrubbing.** *(Built 2026-10-13: section 8. What remains is the owner's decision on the allow-list content, section 8.6.)* The format cannot keep board data out of message bodies (section 3). Accept the host-only rule plus a census that never emits bytes, or require a scrub stage with an explicit control allow-list before any capture leaves the rental?
2. **Which controls may the module not copy at all?** The cheapest filter is on the host side of the module (skip the reply of `0x2080014a`), but that is parsing. If the answer to 1 is "scrub offline", this question disappears.
3. **Windows version string.** The module can read the file version of the display driver; the mapping to `580.88` is by convention. Ask for the string verbatim (as designed) and let the cell record the mapping.
4. **ogkm tag for a non-measured driver.** 580.88 is not in the matrix; the tool refuses to guess. Choose the tag per cell by decision (nearest measured below, here 580.82.07) or extend the matrix sweep to the Windows driver versions.
5. **The 87 `unknown` controls.** *(Answered 2026-10-13: 49 defined, 38 absent from every tag; section 6, *the 87 `unknown` controls*; `tier1_ogkm_captured87.tsv` loads with `--tier1`.)* Run the 221-tag number search (the tier-1 method) over them; it is GPU-free and decides which of the captured controls are really Windows-only. The result would load straight into `--tier1`.
6. **`gl1` is truncated by the observer cap.** Re-capture with a higher cap before using its tail, or accept its counts as lower bounds?

## 8. The scrub stage (`kf-census scrub`): keeping a capture without its board data

**STATUS: built and tested GPU-free, 2026-10-13** (`crates/kf-census/src/scrub.rs`, `tests/scrub.rs`, 19 tests; run on the five real captures below). Not run on a module capture: none exists. Labels as above: **[M]** measured, **[R]** read in source, **[I]** inferred.

### 8.1 What it does

`kf-census scrub [--ogkm-tag T] [--no-default-allow] [--allow-screened] [--allow-list FILE] [--allow ID]... --out SCRUBBED INPUT` reads a `KFGSPCAP` capture or an observer `gsp.jsonl` (sniffed) and writes a **default-deny** copy (mode 0600, still host-only). Exit 0 if the input was clean, 1 if it contained something the scrub had to drop or zero (the output is written either way), 2 for a usage or I/O error.

| kept verbatim | replaced by the fill pattern `KFSCRUB!` (`scrub::FILL`) | replaced by zero |
|---|---|---|
| file header (die, driver string, tag, clock) with `flags` bit 0 set; every record header (counter, timestamp, queue, entry range); `BIND`, `LOSS`, `END` records; per message: `seqNum`, `elemCount` (and the MCTP words where the layout has them), the 32-byte RPC envelope, and for an RM control the control header (client, object, **cmd**, status, `paramsSize`, flags) | the **body** of every RM control (`params[]`) whose cmd is not on the allow-list; the payload of a `CONTINUATION_RECORD` whose head is not allow-listed (the count continues across the join, so a large body is one unbroken pattern); the payload of every other function (events, alloc, init, static info ...) | the 32 CC bytes and any other unnamed byte of the element header; the zero padding after a message inside its last entry (a ring entry can hold stale bytes of an earlier message); an entry that starts no message and the unfinished tail of a message the decoder drops (reduced to its header fields so that the decoder raises the same anomaly); an `ELEMENTS` record that does not match its `BIND`; a malformed `BIND`; any payload on a `LOSS` or `END` record |

The checksum is **recomputed** so the scrubbed message still verifies. A message that did **not** verify keeps its original residual (`checksum_field = fold(scrubbed) XOR fold(original)`), so a corrupt message stays corrupt and the oracle reports the same anomaly kinds: the scrub never heals what it did not understand. Lengths, sequence numbers and record counters are untouched, so `seq_gap`, `counter_gap`, `capture_loss`, `discontinuous_entries` and `end_missing` survive a scrub [M, tests `a_message_that_did_not_verify...`, `entries_that_never_formed...`, `a_loss_marker_and_the_record_stream_are_kept`].

Observer files: header, record and footer lines keep only keys on explicit lists (counters, flags and short printable words; `driver_stats` counters). `payload_hex` is rewritten with the same length, the footer's `source_sha256` / `observations_sha256` are dropped (they would no longer match), unknown keys and unreadable lines are not copied, a record line before the header is refused (the marker lives in the header). The header gains `"kf_scrubbed":1`.

Memory is bounded: a hostile guest can start a message and never finish it while the other queue carries on; past 256 MiB of held-back records every incomplete message is reduced to its header and the records are written (`stats.forced_flushes`, exit status 1) [M, test `a_message_that_never_completes...` with a 40 KiB cap].

### 8.2 How the census says "body scrubbed"

A capture with `flags` bit 0 (observer: `kf_scrubbed`) tells the census that a control body which is exactly the fill pattern (byte `i` = `FILL[i % 8]` from the first byte of `params[]`) is a scrubbed body. The census then counts it in a new `scrubbed` column (TSV and JSON), and leaves it out of `echo` and `distinct_replies` (a scrubbed request and reply are equal, which would otherwise count as an echo); every count, size, status and pairing is unaffected. A file without the flag is never searched for the pattern: a real body that happens to equal it is a real body [M, test `a_fill_looking_body_in_an_unscrubbed_file_is_a_real_body`]. Remaining ambiguity [I]: inside a scrubbed file, an allow-listed body that is *exactly* the pattern would be counted as scrubbed; no real body is.

Format change: `flags` was reserved zero, bit 0 is now `FLAG_SCRUBBED`, set only by the scrub stage; every other bit must stay zero. The Windows capture module's C parser (`kfwc_cap_file_parse`, `lib/kfwc_record.c:133`) rejects any non-zero `flags`; it never reads scrubbed files, so nothing breaks, but the two must agree if a tool of the module ever opens one.

### 8.3 The allow-list

One control id per line, hex or decimal, `#` comments. Four layers; nothing is allowed unless named:

| layer | what | how to get it |
|---|---|---|
| **default** (`allow_default.txt`, 8 ids) | the Windows-only ids whose purpose is documented from measured behaviour and whose observed bodies pass the structural screen | on by default; `--no-default-allow` removes it |
| **screened** (`allow_screened.txt`, 20 ids) | the Windows-only ids that were observed, pass the screen, but whose **purpose is not known** | `--allow-screened` |
| **explicit** | anything else the operator names | `--allow ID`, `--allow-list FILE` |
| **never** (`scrub::BOARD_DATA_CONTROLS`, 17 ids) | controls that return board data | refused by every route (list, flag, file): exit 2 |

The rule for the default layer (owner, 2026-10-13: unknown means off): the id is in no ogkm tag (so ogkm cannot say what it returns; section 6, *the 87 unknown controls*; `0x00730122` is the one number that exists in older tags, as a different 16-byte control), its purpose is stated in `V3_WINDOWS_ONLY_CONSTANTS_20261013.md` from measured behaviour, it was observed in the five captures, and none of its observed bodies trips `tier_checks/body_screen.py`. The screen looks for a printable-string run (>= 6 bytes, >= 4 alphanumeric), a random-looking 16-byte window (>= 14 distinct values, >= 12 non-zero) and a ROM signature. **It is a screen on one die and one driver, not a proof**: an identifier that looks structured passes it, and a die whose firmware puts a serial into a field that is zero on the RTX 4070 would pass today and leak tomorrow. That is the reason the default layer is small and the rest is opt-in.

#### The 40 rows are 34 control ids

`V3_WINDOWS_ONLY_CONSTANTS_20261013.md` lists 40 inventory rows. Four are not RM controls (display timing and behaviour rows), two repeat the id of another row (`const-echo-730122`, `const-reply-730128`), and `const-gssreplay-rows` / the host asks name `0x2080a026/a028/a084/9064` and the encoder-session pair `0x20808163/64` (not in the table as rows of their own). The distinct control ids are **34**: 24 `ctl-` rows of group A, `0x0073011a`, `0x2080a028`, `0x2080a026`, `0x2080a084`, `0x20808163`, `0x20808164`, and the four of group B. A test pins that the three lists together cover exactly these 34 and that each is named in the constants page (`the_lists_cover_the_windows_only_control_ids_exactly`).

#### Risk table: each of the 34

"ogkm" is the 221-tag search of section 6 (*the 87 unknown controls*) (33 of the 34 are absent from every tag; `0x00730122` exists only before 555.42.02, as a different control. ogkm therefore **cannot** say what any of them returns, which is why no row says "ogkm: returns X"). "calls" are requests/replies over the five captures [M]; "screen" is `body_screen.py` over every distinct request and reply body (a = printable run, h = random-looking window, r = ROM signature).

| id | what the constants page says | calls req/rep | reply B | distinct replies | screen | decision |
|---|---|---:|---:|---:|---|---|
| `0x0073011a` | GET_DISPLAY_MODE {displayId, active w/h, total w/h, 0, refresh Hz} | 15/15 | 28 | 2 | clean | kept by default |
| `0x00730122` | per-display query; reply echoes the request {0, displayId} | 7/7 | 8 | 1 | clean | kept by default |
| `0x00730128` | display query; reply {0,1,0xff,0,0} for an all-zero request | 17/17 | 20 | 1 | clean | kept by default |
| `0x007302a3` | display query (12 B); kayfabe echoes the request | 5/5 | 12 | 1 | clean | kept by default |
| `0x007302a5` | display query (16 B reply), issued very often; UUID-sized, so kept off by hand | 157/157 | 16 | 1 | clean | kept with `--allow-screened` |
| `0x00809908` | NV0080 object control (class 0x9908 assumed) | 70/70 | 16 | 3 | clean | kept with `--allow-screened` |
| `0x20801111` | software-runlist submit (40 B) | 565/565 | 40 | 227 | clean | kept by default |
| `0x20802806` | GSS-legacy, purpose not known | 5/5 | 4 | 1 | clean | kept with `--allow-screened` |
| `0x20808163` | encoder-session pair with 0x20808164 (4 B), hand-typed from a host probe | 0 | - | - | not observed | **UNKNOWN: never seen**, off |
| `0x20808164` | encoder-session pair with 0x20808163 (4 B) | 0 | - | - | not observed | **UNKNOWN: never seen**, off |
| `0x20808524` | GSS-legacy, purpose not known | 5/5 | 4 | 1 | clean | kept with `--allow-screened` |
| `0x2080852a` | GSS-legacy, purpose not known | 24/24 | 1784 | 1 | clean | kept with `--allow-screened` |
| `0x2080852e` | GSS-legacy, purpose not known | 19/19 | 528 | 1 | clean | kept with `--allow-screened` |
| `0x2080852f` | GSS-legacy, purpose not known | 4/4 | 776 | 4 | h2,h2 | **UNKNOWN: screen positive**, off |
| `0x2080853a` | GSS-legacy, purpose not known | 5/5 | 1456 | 1 | clean | kept with `--allow-screened` |
| `0x2080853b` | GSS-legacy, purpose not known | 0 | - | - | not observed | **UNKNOWN: never seen**, off |
| `0x20809001` | GSS-legacy CUDA-runtime init row (8 B) | 15/15 | 8 | 1 | clean | kept with `--allow-screened` |
| `0x20809004` | by far the most frequent; 71 distinct replies | 640/640 | 1544 | 71 | clean | kept with `--allow-screened` |
| `0x20809009` | GSS-legacy CUDA-runtime init row (8 B) | 5/5 | 8 | 1 | clean | kept with `--allow-screened` |
| `0x20809064` | GSS-legacy clock-domain listing (520 B) | 5/5 | 520 | 1 | clean | kept by default |
| `0x2080a026` | GSS-legacy cudart max-clock query (a host ask too) | 10/10 | 532 | 2 | clean | kept by default |
| `0x2080a028` | GSS-legacy per-domain clock query (a host ask too) | 60/60 | 2192 | 7 | clean | kept by default |
| `0x2080a02f` | the GPU itself answers 0x56 (1 B) | 5/5 | 1 | 1 | clean | kept with `--allow-screened` |
| `0x2080a060` | GSS-legacy, purpose not known | 5/5 | 28 | 1 | clean | kept with `--allow-screened` |
| `0x2080a084` | GSS-legacy cudart clock query (host ask) | 0 | - | - | not observed | **UNKNOWN: never seen**, off |
| `0x2080a088` | GSS-legacy, purpose not known | 10/10 | 12 | 1 | clean | kept with `--allow-screened` |
| `0x2080a618` | the largest reply (8620 B) | 24/24 | 8620 | 1 | clean | kept with `--allow-screened` |
| `0x2080a619` | GSS-legacy, purpose not known | 0 | - | - | not observed | **UNKNOWN: never seen**, off |
| `0x2080a630` | GSS-legacy, purpose not known | 5/5 | 1160 | 1 | clean | kept with `--allow-screened` |
| `0x2080a70a` | GSS-legacy, purpose not known | 5/5 | 26 | 1 | clean | kept with `--allow-screened` |
| `0x2080a801` | GSS-legacy, purpose not known | 10/10 | 1028 | 1 | clean | kept with `--allow-screened` |
| `0x2080b201` | GSS-legacy, purpose not known | 10/10 | 2188 | 1 | clean | kept with `--allow-screened` |
| `0x20810108` | NV2081 control | 16/16 | 992 | 1 | clean | kept with `--allow-screened` |
| `0x2081010d` | needed for Windows StartDevice; kayfabe answers OK with zero-filled params | 5/5 | 0 | 1 | clean | kept with `--allow-screened` |

Reading the table: **default** rows have a purpose in the constants page and a negative screen. **screened** rows have a negative screen and an unknown purpose: a board-specific field cannot be excluded by reading, only by looking at the values on more dies, so they are one flag away and not on by default. **neither** rows: `0x2080852f` trips the screen (2 of its 4 distinct bodies hold a random-looking window, 776 B), the other five were never issued in the five captures, so nothing at all is known about their bodies (**UNKNOWN**, left off).

#### Controls that must never be allow-listed (hard deny-list)

Read in ogkm 595.84 (and the same ids at 580.178.04); the struct fields are the evidence [R]. "seen" is the request count in the five captures [M].

| id | ogkm name | what identifies the board | seen |
|---|---|---|---:|
| `0x2080014a` | GPU_GET_GID_INFO | the GPU UUID (`data[]`) | 0 |
| `0x2080013f` | GPU_GET_OEM_BOARD_INFO | `serialNumber`, `productPartNumber`, memory part ids, `buildDate` | 5 |
| `0x20800169` | GPU_GET_OEM_INFO | `oemInfo[]` | 0 |
| `0x2080014b` | GPU_GET_INFOROM_OBJECT_VERSION | board InfoROM object versions | 20 |
| `0x20800156` | GPU_GET_INFOROM_IMAGE_VERSION | board InfoROM image version | 0 |
| `0x2080015a` | GPU_GET_PHYSICAL_BRIDGE_VERSION_INFO | board bridge chip firmware | 0 |
| `0x20800802` | BIOS_GET_INFO | VBIOS version, OEM version | 0 |
| `0x20800810` | BIOS_GET_INFO_V2 | VBIOS version, OEM version | 5 |
| `0x20800803` / `0x20800806` / `0x2080080e` | BIOS_GET_NBSI / _OBJ / _V2 | VBIOS NBSI registry strings and objects | 0 |
| `0x20800808` | BIOS_GET_SKU_INFO | board id, chip SKU, project SKU | 0 |
| `0x20801801` | BUS_GET_PCI_INFO | `pciSubSystemId` (the board vendor and model) | 0 |
| `0x20803039` | NVLINK_SYNC_LINK_MASKS_AND_VBIOS_INFO | VBIOS data | 0 |
| `0x20803088` | NVLINK_GET_LOCAL_DEVICE_INFO | `deviceUUID[16]`, PCI location | 0 |
| `0x0073011d` | SYSTEM_GET_CONNECTOR_TABLE | the board's connector table (VBIOS-derived) | 5 |
| `0x00730245` | SPECIFIC_GET_EDID_V2 | the attached monitor's EDID (model, serial number) | 39 |

[M] Five of these are in the real captures (`0x2080013f`, `0x2080014b`, `0x20800810`, `0x0073011d`, `0x00730245`: 74 requests), and the structural screen flags `0x2080013f` (printable run) and `0x00730245` (printable run, random-looking window) and `0x0073011d` (random-looking windows): the screen is not blind to the ids that matter. This list is a **floor**, not the policy: the allow-list is the policy (default deny), and the non-control messages that carry board data (`GET_GSP_STATIC_INFO`, `SET_GUEST_SYSTEM_INFO`, `GSP_SET_SYSTEM_INFO`: 3 + 3 + 6 command-queue messages over the three `dvi` captures, with their replies) are always scrubbed because no non-control payload is ever allow-listed. The ids are those of 595.84; ids are renumbered per branch (section 6, *the 87 unknown controls*), the three that moved were checked at 580.178.04 as well.

### 8.4 Tests, and what they would catch

`cargo test -p kf-census` runs 19 scrub tests (50 tests in all: 27 oracle, 19 scrub, 4 golden, 1 ignored regenerator). A fixture plants a serial string, a textual and a raw UUID, a 3 KB VBIOS-looking image (ROM signature, version string, noise), stale bytes in the ring padding and bytes in the CC area, in non-allow-listed control bodies (request and reply), an event payload and the middle of a 70 000-byte large control, and writes it as captures cut at 1, 3, 8 and 64 entries per record and as an observer export.

- no planted needle survives in the scrubbed bytes (byte search; hex search for the export) [M];
- the allow-listed reply body survives byte for byte, in both formats; a scrub that allows every id in the stream leaves every message run identical except the events and the deny-listed GID control [M];
- the census of the scrubbed file equals the original in requests, replies, pairing, request and reply size histograms, statuses, `rpc_results`, functions and `large_joined`; `echo`, `distinct_replies` and the new `scrubbed` are the only columns allowed to change, and for allow-listed controls they do not [M];
- an empty body is not a scrubbed body; a deny-listed id is refused in the list parser, the `--allow` flag and the binary; the output file is mode 0600 [M].

Mutation check (the tests must fail when the stage is broken) [M]: keeping every body fails 13 of 17 tests; not recomputing the checksum fails 9; keeping the trailing padding fails 2; keeping the element header's unnamed bytes fails 12. (Run at 17 tests; the stage then gained the memory bound and two tests.) **Falsifier of the whole claim**: a real body of a non-allow-listed control found in a scrubbed file (`scrub_check.py`, last column below, or any byte search).

### 8.5 Result on the five real captures

`kf-census scrub` with the default list, then `kf-census` on original and scrubbed, then `tier_checks/scrub_check.py` (independent, no kf code: layout offsets 48 / 32 / 40 only). Everything [M].

| capture | messages | control bodies kept / scrubbed (bytes scrubbed) | other payloads scrubbed | oracle original -> scrubbed | census cells differing outside `echo` / `distinct_replies` / `scrubbed` | bodies the census reports as scrubbed | `scrub_check.py`: kept bytes changed / bytes that are neither pattern nor zero / checksums not folding to 0 | random-looking 16-byte windows in scrubbed bodies: original -> scrubbed |
|---|---:|---|---:|---|---:|---:|---|---|
| `dvi1` | 13 770 | 262 / 9 820 (23 004 264) | 3 676 | PASS -> PASS | 0 of 209 rows | 9 806 | 0 / 0 / 0 | 1 730 -> 0 |
| `dvi2` | 15 899 | 406 / 9 990 (16 710 604) | 5 491 | PASS -> PASS | 0 of 215 | 9 972 | 0 / 0 / 0 | 1 588 -> 0 |
| `dvi3` | 10 537 | 206 / 7 598 (9 891 280) | 2 721 | PASS -> PASS | 0 of 213 | 7 580 | 0 / 0 / 0 | 1 477 -> 0 |
| `gl1` | 17 843 | 244 / 14 896 (63 840 780) | 2 691 | FAIL (2: the observer's own `dropped=1`) -> FAIL (the same 2) | 0 of 216 | 14 882 | 0 / 0 / 0 | 2 843 -> 0 |
| `gl2` | 11 984 | 250 / 9 264 (16 487 476) | 2 458 | PASS -> PASS | 0 of 213 | 9 242 | 0 / 0 / 0 | 1 589 -> 0 |

Every scrub exited 0 (no undecodable message, no dropped line, 12 continuation records scrubbed in each: the 6 large controls per direction). `functions.tsv` of original and scrubbed is byte-identical in all five. The "scrubbed" count in the census is a little lower than the scrub's own count because a control with `paramsSize` 0 has no body to hide and is counted as neither. Time for `dvi1` (53 MB): 9 s CPU for the scrub, a debug build on a loaded box. `gl1` and `gl2` were streamed from the host over ssh into the tool (`-` as input) and the scrubbed output went to a private tmpfs; no original left the host's reach beyond what the earlier census run already did.

Reading: the scrub kept the default-list bodies (262, 406, 206 ...) and replaced the rest; the oracle result is the same before and after (`dvi1`..`dvi3`, `gl2` pass; `gl1` keeps the observer's own `dropped=1` admission); every census cell except `echo`, `distinct_replies` and `scrubbed` is equal, `functions.tsv` is byte-identical; `scrub_check.py` finds no kept byte changed, no byte outside the keep list that is not the pattern or zero, every checksum folding to zero, and **zero** random-looking 16-byte windows in scrubbed bodies where the originals hold some (the test has power).

### 8.6 Not covered, and decisions for the owner

- The scrub is only as good as the keep list. The control header, the RPC envelope and the allow-listed bodies are the only places board data could remain; the first two are protocol fields [R], the third is the screen above [M, one die].
- The element header's `seqNum`/`elemCount` and the envelope's `rpc_result`, `sequence` and `spare` word are kept: a driver that put an identifier in `spare` would leak it (580.88: it is 0 in all five captures [M]; `rpc_sequence` is 0 as well).
- Scrubbed files still tell *which* controls the driver issued, how often, with which sizes and statuses, and the die id and driver string of the header: that is the census, by design.
- **Decision 1.** Accept the default layer (8 ids), or move any of them to the opt-in layer, or add a content screen to the scrub itself (not built: an allow-listed body that trips the screen would be scrubbed anyway and counted, which makes the list safe against a die whose firmware differs; the cost is losing real data wherever a legitimate body looks random).
- **Decision 2.** Promote some of the 20 screened ids once a second die or driver agrees with the RTX 4070 (the same bodies, or bodies of the same structure) or someone with the Windows capture module reads their layout.
- **Decision 3.** The six "neither" ids stay off until a capture contains them.

## 9. The golden capture, and how the two writers are kept from drifting

`crates/kf-census/tests/golden/conversation.kfgspcap` (95 168 bytes, 24 records) is **written by the Windows capture module's own engine and record writer** (`tools/windows-capture/lib`, branch `claude/windows-capture-module-20261013`, commit `92306ee5b`, built with gcc and ASan/UBSan), from `conversation.runs`: 21 messages (10 commands, 11 status messages: 7 distinct controls with 4 echoes, a 0x56 and a 0x1f status and a reply of 8 620 bytes over three entries, an event, an alloc) encoded by kayfabe's transport encoder. The C generator (`gen_golden.c`) and the script (`regen.sh`) are in the same directory; nothing in the module's tree was edited.

`tests/golden.rs` (4 tests): (1) `conversation.runs` is exactly what the encoder produces today; (2) the golden decodes with the oracle passing and the **exact** per-control census (every cell; I derived the expected values from the message list, not from the decoder's output, and the one cell I had got wrong, the `echo` of `0x2080a02f`, was my arithmetic: request and reply are both one zero byte); (3) every record read from the module's file, re-written with this crate's reference `Writer`, reproduces the file **byte for byte** (it found one disagreement: the module stamps the queue it was following on the `END` record, `queue_id` 1, the reference wrote 0; the field is not interpreted by either decoder, so the reference gained `Writer::end_at` rather than the module a change); (4) the golden scrubs and the scrubbed census agrees.

**Drift control (the module's CI must run this).** `KFWC=<module>/tools/windows-capture crates/kf-census/tests/golden/regen.sh --check` rebuilds the generator against the module's library, regenerates the capture and `cmp`s it with the committed file; it exits non-zero if the module's writer no longer produces these bytes. Together with (3), a change to either writer or to the record format shows up as a failure on the side that did not change: the format change must land in `capture.rs`, the golden, and the module in one step. Reproduced [M] against the module at its commit `92306ee5b` and against its dirty working tree on 2026-10-13. When the encoder changes: `KF_CENSUS_REGEN=1 cargo test -p kf-census --test golden -- --ignored`, then `regen.sh`. **Falsifier**: `regen.sh --check` failing, or test (3) failing, on an unchanged format.

Not covered by the golden: the pre-610 layout only (the module also writes 610; its own round trip test does), no loss record, no ring wrap (95 KB is the size budget; a wrap needs 63 entries per queue), no `LOSS` kinds.
