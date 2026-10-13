# V3: the GSP queue capture format (`KFGSPCAP`) and the census tool

**STATUS: DESIGN-ONLY, 2026-10-13.** The format and the decoder (`crates/kf-census`) exist and are tested GPU-free; no capture module exists, nothing was captured in this format on a real GPU, and the module's emit path below is a design. Step 1 of `V3_WINDOWS_ONLY_CONSTANTS_20261013.md` (capture module on rented GPUs, decoder, census); strategy in `V3_CONSTANT_STRATEGY.md` s4. Labels: **[M]** measured (a command run for this page), **[R]** read in source, **[I]** inferred. An inference is never written as evidence.

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

**Tests** (`crates/kf-census/tests/oracle.rs`, 26, GPU-free, synthetic fixtures built with `kf_gsp::encode_message`): valid stream (both formats, and the two formats give byte-identical census TSVs), gap, missing prefix, reordered, duplicate, truncated element, file cut inside a record, message missing its last element, unknown function id, corrupted message, observer metadata disagreeing with the payload, bad footer, loss marker, unmarked ring jump, record-counter gap, missing End, large control join, an overlong/garbage observer line, the binary's exit status, the round trip below, and the no-board-data checks. Round trip: 100+ messages (single- and multi-element, past the ring end) are encoded by the transport encoder, written as a capture in chunks of 3 entries, read back, and every reassembled message run equals the encoded run byte for byte; decoded function, sequence and payload equal what was sent. Run: `CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 cargo test -p kf-census`.

**Census columns** (per control id): `requests`, `replies`, `paired`, `unanswered`, `orphan_replies`, `echo` (paired where reply params equal request params byte for byte), `serialized` (requests with the serialized-params flag), `req_sizes` / `reply_sizes` (declared `paramsSize` histograms, `value:count`), `statuses` (reply control `status`), `rpc_results` (reply envelope), `distinct_replies` (count of distinct reply bodies), `ogkm` and `ogkm_basis`.

**The ogkm column** answers from data the repo already holds and says `unknown` when it has none: `defined` + `matrix:NAME` if the driver matrix has the control at the tag (131 ids kayfabe consumes); `defined` / `absent_all_tags` from `tier1_ogkm.tsv` (221-tag search over the 140 inventory rows; `NOT_IN_ANY_VERSION` there means no tag defines that number, with the file's own convention for number collisions); otherwise `unknown`, which is **not** `absent`.

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

**Controls that appear in the captures that ogkm lacks** (by the repo's own tier-1 search, 221 tags): 26 control ids, 1 643 calls: `0x20809004` (640), `0x20801111` (565), `0x007302a5` (157), `0x00809908` (70), `0x2080852a` (24), `0x2080a618` (24), `0x2080852e` (19), `0x00730128` (17), `0x20810108` (16), `0x20809001` (15), `0x2080a088`, `0x2080a801`, `0x2080b201` (10 each), `0x00730122` (7), and the 5-call group `0x007302a3`, `0x20802806`, `0x20808524`, `0x2080853a`, `0x20809009`, `0x20809064`, `0x2080a02f`, `0x2080a060`, `0x2080a630`, `0x2080a70a`, `0x2081010d`, plus `0x2080852f` (4). Of the 221 ids, 108 are `defined` (matrix at the tag, or tier 1), 26 `absent_all_tags`, **87 `unknown`** (19 236 calls, including the five most-called ids): neither source covers them because the tier-1 search only ran over the 140 inventory rows and the matrix only holds the 131 controls kayfabe consumes. `unknown` does not mean absent.

[M, supplementary, single tag, presence only; not part of the tool] A scan of `#define NV*CTRL_CMD*` in `/data/nouveau-trace-research/ogkm-575.51.03/src/common/sdk/nvidia/inc` finds 48 of those 87 ids defined at 575.51.03 (11 849 calls; for example `0x0073010b` = `NV0073_CTRL_CMD_SYSTEM_GET_HEAD_ROUTING_MAP`, `0x0073010c` = `..._SYSTEM_GET_ACTIVE`); 39 ids (7 387 calls) have no such define at that tag, among them `0x20808539`, `0x20809037`, `0x2080a0a8`, `0x2080a0d1` and `0x2080b202`. A one-tag scan can show presence, not absence (class-method controls such as `0xc3720101` do not use that define pattern); the 39 stay `unknown` until the 221-tag search of tier 1 is run over them (open question 5).

### Not verified

- Nothing was captured in the `KFGSPCAP` format by a real module; the format is exercised only by synthetic streams produced by `kf_gsp::encode_message` (section 5 items 1 and 2 remain untested against real ring behaviour).
- The Windows driver is 580.88 and no measured ogkm tag has that number; the identical output at 580.82.07 and 580.94.02 is evidence the tag choice does not matter *within 580.x for these captures*, not for other driver families.
- The footer sha-256 digests of the observer files are not recomputed by this tool (the counts, byte totals, per-message checksums and sequence continuity are).
- One AD104 die, one driver, one monitor: nothing here is a per-die or per-driver comparison.

## 7. Open questions for the owner

1. **Payload scrubbing.** The format cannot keep board data out of message bodies (section 3). Accept the host-only rule plus a census that never emits bytes, or require a scrub stage with an explicit control allow-list before any capture leaves the rental?
2. **Which controls may the module not copy at all?** The cheapest filter is on the host side of the module (skip the reply of `0x2080014a`), but that is parsing. If the answer to 1 is "scrub offline", this question disappears.
3. **Windows version string.** The module can read the file version of the display driver; the mapping to `580.88` is by convention. Ask for the string verbatim (as designed) and let the cell record the mapping.
4. **ogkm tag for a non-measured driver.** 580.88 is not in the matrix; the tool refuses to guess. Choose the tag per cell by decision (nearest measured below, here 580.82.07) or extend the matrix sweep to the Windows driver versions.
5. **The 87 `unknown` controls.** Run the 221-tag number search (the tier-1 method) over them; it is GPU-free and decides which of the captured controls are really Windows-only. The result would load straight into `--tier1`.
6. **`gl1` is truncated by the observer cap.** Re-capture with a higher cap before using its tail, or accept its counts as lower bounds?
