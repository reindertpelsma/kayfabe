# N's subsequent output-LUT constructor

**STATUS: RESEARCH, 2026-10-05 — saved at owner-requested stop; independent review
pending.** No product change, new capability flag, hardware action, or subsequent
experiment is authorized by this note.

**Correction folded into the finding:** the descriptor's `+4` value is not the
buffer count checked at constructor RVA `16f04cc`. The buffer count is `+0x18`,
initialized to **one** for this descriptor. Therefore the count guard passes
with SFCLOAD clear. The concrete candidate mismatch is the same **pointer shape**
condition as the preceding TMO descriptor, not ILUT's two-buffer count mismatch.
An earlier investigation message conflated these two fields; do not use that
message as evidence.

Parent/observer-agent reports N product `9312854d` progresses past the entire
window ILUT/TMO constructor loop, obtains the subsequent `0x8000` CPU allocation,
then records assertion return RVA `16e9967`. The observer agent owns the recovered
raw-journal validation/evidence; this note independently maps the corresponding
static producer. No live descriptor locals or exact inner failure were recovered
by this audit.

Pinned Windows 580.88 driver SHA256:
`31c79cce80b573e21647ace8d1a2b65697f992e7f86196f89e30af477b1bd47c`.
All addresses below are RVAs, not live addresses or capability captures.

## Descriptor origin

The constructor call at `16e98d9` receives
`rsp+0x40+index*0xb0`; loop indices run below four. The selection predicate is
virtual slot `+0xab8` at `16e9839`; its exact implementation has **not** been
followed in this audit. The field producer is conditional on that same slot for
indices 0..3. The capability words are read through virtual slot `+0xdd0`:

| Descriptor index | Capability offset passed to getter | Producer entry |
|---|---|---|
| 0 | `0x684` | `16e9025` |
| 1 | `0x6a4` | `16e90f1` |
| 2 | `0x6c4` | `16e91b7` |
| 3 | `0x6e4` | `16e929e` |

These match public
`NVC773_POSTCOMP_HEAD_HDR_CAPB(i) = 0x684+i*32`, defined in OGKM580.65.06
`src/common/sdk/nvidia/inc/class/clc773.h:373–390` at source commit
`307159f2623d3bf45feb9177bd2da52ffbc5ddf9`. Its named fields are
`OLUT_LOGSZ[9:6]`, `OLUT_LOGNR[12:10]`, `OLUT_SFCLOAD[14:14]`, and
`OLUT_DIRECT[15:15]`. This identifies the descriptor as per-head output LUT,
independently of the loop count. The existing compiler-generated display tables
contain these fields for C573/C673/C773/CA73 in both admitted display versions.

For the first descriptor, `16e903b–16e90c8` derives:

| Input offset | Static derivation |
|---|---|
| `+0` | `1 << OLUT_LOGSZ`, stored at `16e906a` |
| `+4` | `(1 << OLUT_LOGNR)+1`, stored at `16e9084`; not the buffer-count guard |
| `+8` bit0 / bit1 | DIRECT / SFCLOAD, stored at `16e90a0` |
| `+0xc` | One, stored at `16e905b` |
| `+0x10` | Stride from `16f0690`, `align_up((entries+5)*8,256)` |
| `+0x14` | Same virtual-slot `+0x258` instance getter as the earlier descriptors |
| `+0x18` | One, initialized at `16e8c82` |
| `+0x1c` | Offset into backing, sourced from owner `+0x62e4` |
| `+0x28` onward | Per-instance backing pointers written at `16e9890–16e98a3` |

The later descriptor producers repeat those formulas. N's authored postcomp
CAPB words remain zero; N changes only the earlier window TMO surface-load bit.
For a zero CAPB, entries=1 and the distinct `+4` value=2; stride=256 satisfies
the constructor's initial extent check. Buffer count remains one. Instance
count validity and actual pointer values are not recovered here.

## Guard and next bounded hypothesis

Constructor `16f03c0` reads `+0x18` at `16f04c2`. SFCLOAD clear requires count1
at `16f04cc` (satisfied by the static OLUT initialization), then sends each
pointer slot to `16f0536`, which requires zero. A nonzero pointer fails at
`16f0545→16f05f2`. SFCLOAD set instead permits up to two buffers and requires
the active instance/buffer slots nonzero at `16f04ef–16f050f`, with unused slots
still zero. The caller fills the active pointer slots before `16e98d9`, using
owner backing pointers plus the saved offset. This is a concrete mismatch
candidate if the allocated backing is nonnull; it does not prove the live inner
failure. Allocation failure after these guards remains another possibility.

The constructor's success byte is tested at `16e98ed`; failure branches to
`16e9962`, whose assertion return is `16e9967`. An isolated future diagnostic
could change only generated `POSTCOMP_HEAD_HDR_CAPB_OLUT_SFCLOAD_TRUE` relative
to N, for advertised heads, retaining immutable refusal of **all** display
methods. No DIRECT/size/capacity changes are suggested. That experiment has
**not** been implemented or run, and cannot establish active output-LUT support.

Operational obligations are separately described in the unreviewed
[production audit](LUT_PRODUCTION.md). Resume from this note and the parent
handoff; do not mistake this source hypothesis for a captured capability value
or a production feature.
