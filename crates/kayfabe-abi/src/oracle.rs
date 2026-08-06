//! ★★★ The C artifact's captured control table, and the one move it does **not** support:
//! reading a row it recorded **empty** as *"hardware answers zero"*.
//!
//! # The defect this module exists to make unrepeatable
//!
//! `nvidia-gpu-passthrough/src/qemu/mode2_initctrl_ga106.h` is labelled *"GA106 init
//! `GSP_RM_CONTROL` responses (real, captured from host)"* and is the closest thing this
//! port has to an answer sheet. Each row is
//! `{ cmd, status, psize, dlen, ctl_<cmd>[] }`, where `psize` is the reply's size **on the
//! wire** and `dlen` is *how many bytes the recorder kept*.
//!
//! **56 rows. 11 of them — 19.6% — carry `dlen = 0`.** For those the `ctl_` array is
//! literally `{ }`, and the obvious reading is that the reply body was `psize` bytes of
//! nothing. That reading built a wall: `0x20802a08`'s empty row decodes to a fault-method
//! buffer size of **0**, which is a buffer RM allocates and *hardware DMAs into*
//! ([`crate::fmbsize`]).
//!
//! ★★ It survived four rungs because it was **cited**. `crates/kayfabe-device/src/sweep.rs`
//! is gated on every triage row naming a source, and the row named `C:
//! mode2_initctrl_ga106.h:6233 = 0x20802a08` — exactly as demanded. ⊘ **Citing the oracle is not the
//! oracle being right.** A citation gate checks a claim is *sourced*; it can never check
//! that the source says what the claim says it says.
//!
//! # ★★★ The class, MEASURED — all eleven, not a sample
//!
//! `[measured]` 2026-08-01 on an NVIDIA GeForce RTX 3060 (GA106),
//! `GPU-e28d7776-e4f9-704b-d392-d46f187343f8`, vast instance `46494693`, host driver open
//! **580.159.04** — `research_clones/ogkm-580.159.04` (tag `b81d58e`) rebuilt with a chunked
//! whole-body print at `rpcRmApiControl_GSP`'s reply, `RmInitAdapter` driven by opening
//! `/dev/nvidia0`, and widened with `nvidia-smi -q`, which is what reaches `0x20800a4c`.
//! Whole bodies at `traces/real_ga106/rpc_bodies_real_ga106.txt`.
//!
//! | control | C's row | real GA106 | verdict |
//! |---|---|---|---|
//! | `0x20802a06` | `psize 4, dlen 0` | `10 00 00 00` | **contradicted** |
//! | `0x2080017e` | `psize 8, dlen 0` | `00 00 00 02 00 00 00 00` | **contradicted** |
//! | `0x20800af3` | `psize 2, dlen 0` | `01 01` | **contradicted** |
//! | `0x20802a08` | `psize 4, dlen 0` | `00 50 00 00` | **contradicted** |
//! | `0xa06f0103` | `psize 3, dlen 0` | `01 00 00` | **contradicted** |
//! | `0xa06f0104` | `psize 4, dlen 0` | `0b 00 00 00` | **contradicted** |
//! | `0x20800a4b` | `psize 4, dlen 0` | `00 00 01 04` | **contradicted** |
//! | `0x20800aac` | `psize 4, dlen 0` | `00 00 01 00` | **contradicted** |
//! | `0x20800a6c` | `psize 4, dlen 0` | `31 00 00 00` **and** `11 00 00 00` | **contradicted**, and an `[IN]` **echo** |
//! | `0x20800a4c` | `psize 4, dlen 0` | `00 00 00 00` | ⚠ coincides |
//! | `0x20800a70` | `psize 0, dlen 0` | `<empty>` | ⚠ coincides |
//!
//! **Nine of eleven are wrong.** And the two that are not are the important ones:
//!
//! - `0x20800a70` has `psize = 0`. There is no body to have failed to capture, so the row is
//!   consistent for a reason a reader *can* check without hardware — and that reason is the
//!   whole predicate below.
//! - `0x20800a4c` (`INTERNAL_GPU_GET_SMC_MODE`) genuinely answers zero on this part, because
//!   SMC is disabled. ★★★ **Nothing about the row says so.** It is byte-identical in the
//!   capture to the nine that are wrong. It is the strongest available demonstration that an
//!   empty capture is evidence of *nothing*: here it happens to coincide with the truth, and
//!   a policy of believing it would have been right once and wrong nine times for the same
//!   reason.
//!
//! # The rule, and why it is a predicate rather than a list
//!
//! [`captured_row_evidence`] refuses **`psize > 0 && dlen == 0`** — not "these eleven ids".
//! A list would be a smaller true statement that goes stale the moment a 57th row appears
//! (memory: *gates quantified over a LIST*). The list in [`EMPTY_CAPTURE_ROWS`] exists for a
//! different job: it carries what hardware actually said, so that a citation of one of these
//! rows can be made to carry **content** rather than an address.
//!
//! # ⊘ What this does NOT say
//!
//! - ⊘ **It does not demote the C artifact.** Rows it *did* capture are corroborated by the
//!   same transcript, byte for byte and including all 3712 bytes of `0x20800a2a`
//!   ([`crate::grinfo`]). It remains the only implementation a real driver has accepted end
//!   to end.
//! - ⊘ **It is one part, one driver version, one boot.**
//!
//! # ★★★ WIDENED, 2026-08-01 — the truncated class is BIGGER than the empty one
//!
//! The paragraph that stood here said this module *"says nothing about TRUNCATED rows"*.
//! That was true and it was a live hole: a truncated row **passes** a `dlen == 0` test,
//! because it *has* a body. It therefore decoded cleanly and the uncaptured tail arrived as
//! zeros — the exact failure `0x20802a08` cost four rungs, with the one signal that had
//! caught it (an empty array) removed.
//!
//! `[measured]` 2026-08-01, the whole table at `nvidia-gpu-passthrough` rev `8baf4f2`
//! parsed row by row:
//!
//! | class | rows | share |
//! |---|---|---|
//! | complete (`dlen >= psize`) | **29** | 51.8% |
//! | **truncated (`0 < dlen < psize`)** | **16** | **28.6%** |
//! | empty (`dlen == 0`) | 11 | 19.6% |
//!
//! **Only half the table is a complete capture, and the truncated class is larger than the
//! empty one this module was built for.** [`captured_row_evidence`] now refuses
//! `dlen < psize`, not `dlen == 0`.
//!
//! ## ★★★ The hypothesis that had to die first: trailing-zero trimming
//!
//! ⚠ Widening was not obviously correct. The C's consumer states in-line that *"any
//! captured tail beyond `cr->dlen` is zero"* (`C: src/qemu/nvkvm_gpu_emul.c:3421-3422`),
//! and [`crate::l2evict`] records that this module's own decoder was once read as
//! trimming trailing zeros. **If `dlen` were trailing-zero-trimmed then truncation would
//! not be a defect at all** — the tail really would be zeros, zero-filling would be
//! correct, and widening the predicate would refuse sixteen sound rows.
//!
//! It is refuted, from the **raw bytes of the header** rather than from any decoder — the
//! rule against letting the thing under test be its own observer applies here more than
//! anywhere. A trailing-zero trimmer cannot leave a zero byte at the end of what it kept.
//! **All sixteen kept-prefixes end in zero bytes**; `0x20800a40`'s ends in 15 833 of them
//! and `0x20800a22`'s in 12 053 ([`TruncatedRow::trailing_zeros_kept`]). So `dlen` is not a
//! trim, the tail is **unknown**, and the C's in-line sentence is an assumption it labels
//! as one (*"fine where the meaningful prefix is numEntries + entries"*).
//!
//! ★ What `dlen` actually is, visible in the two largest rows: `0x20800a22` kept
//! 16 376 = 16 384 − 8 and `0x20800a40` kept 16 384 — **one GSP message-queue element**.
//! The recorder did not follow continuation elements. That is the same defect the C repo
//! later fixed for its replay recorder (`GSP-D6`, "make the C's SKIPPED continuation
//! elements OBSERVABLE"), and this table is its fossil.
//!
//! ## ⚠ Why the refusal is per-FIELD and not per-row
//!
//! ⊘ A truncated row is **not** worthless: its first `dlen` bytes came off real hardware
//! and are as good as any complete row's. [`crate::grstatic`] already argues exactly this
//! for `0x20800a22` — *"engine 0 occupies the first 4 324 bytes, so nothing that is read is
//! missing"* — and that argument is **sound**. What it was not, was checkable. Prose cannot
//! fail a build.
//!
//! [`field_is_captured`] is that argument as a predicate. A caller reading a field out of a
//! truncated row states the offset and gets a decision; a caller reading past `dlen` is
//! fabricating and now has to say so by name.
//!
//! ## ⊘ What the widening does NOT establish
//!
//! - ⊘ **No hardware body has been taken for any of the sixteen.** This is a statement about
//!   *the capture*, not about what the missing bytes contain — every one of them may yet
//!   turn out to be zeros.
//! - ⊘ **It does not say the nine referenced rows are wrong.** It says their tails are
//!   unevidenced. Where a module reads only the prefix (`grstatic`) or derives the reply
//!   outright (`deviceinfo`), nothing about the value changes.
//!
//! # ★★★ THE CONSEQUENCE, MEASURED — nothing this port reads is missing
//!
//! The paragraph above closed with *"no hardware body has been taken"*, and that was the
//! whole of what had been established: **the CAPTURE was measured, the CONSEQUENCE was
//! not.** Which rows are short says nothing about whether anything in this tree *reads* the
//! part that is short — and that second question is the one that cost four rungs on
//! `0x20802a08`, where a consumer silently took zero-fill for an answer.
//!
//! `[measured]` 2026-08-02, by locating every byte blob this repository embeds from the C
//! artifact's table inside the table's own arrays and reporting the offset range each one
//! occupies. ⚠ **The read was taken at that repo's HEAD `8001cb5`**, not at the `8baf4f2`
//! the tables above are attributed to. They are the same measurement:
//! `git diff 8baf4f2 8001cb5 -- src/qemu/mode2_initctrl_ga106.h` is **empty**, and
//! `8baf4f2` is an ancestor of `8001cb5`. Recorded rather than smoothed over, because a
//! verification binds to CONTENT at a REVISION and the two revisions are not the same word.
//!
//! | row | `kept`/`psize` | the deepest byte this port relies on | inside? |
//! |---|---|---|---|
//! | `0x20800a22` | 16376/34592 | 16376 — the whole kept prefix, as a fixture | ✔ |
//! | `0x20800a1f` | 176/184 | 176 — the whole kept prefix, as a fixture | ✔ |
//! | `0x20800a9f` | 176/184 | 176 — the whole kept prefix, as a fixture | ✔ |
//! | `0x20800a41` | 8200/8204 | 4108 — `profilingRangesSize`; the embedded prefix ends at 1277 | ✔ |
//! | `0x20801112` | 3208/3212 | 1112 — header + all 11 entries the capture declares | ✔ |
//! | `0x20800a40` | 16384/24580 | 580 — header + all 12 entries the capture declares | ✔ |
//! | `0x208001b0` | 1280/1284 | 164 — count + all 8 falcons the capture declares | ✔ |
//! | `0x20800301` | 16/20 | 0 — the reply is re-encoded from the request | ✔ |
//! | `0x20802a0f` | 16/28 | 0 — refused; no byte of it is decoded | ✔ |
//!
//! ⇒ **No read in this tree lands at or beyond `dlen`.** ★ That is a *result*, not a
//! reassurance: the four-byte-short rows are short in their **last array element**
//! (`profilingRanges[4092..4096]`, `constructedFalconsTable[63]`'s last word,
//! `entries[31]`'s last four name bytes), and every count field that bounds those arrays
//! sits far inside the prefix. The two badly-short rows are short **past the only engine
//! anything reads**.
//!
//! ⊘ **One place does read past `kept`, and it says so.** `tests/gvaspace_pdes.rs` pads the
//! 176-byte `0x20800a9f` fixture to the struct's 184 bytes with **this port's own zeros**
//! and round-trips the result; the eight bytes it adds are `levels[5]`'s `aperture` and
//! `pageShift`, which a publication with `numLevelsToCopy = 4` does not use. The zeros are
//! declared in that file and asserted to be its own — they are not evidence.
//!
//! ## [`CAPTURE_RELIANCE`] is that result as a checkable statement
//!
//! Prose cannot fail a build, and this argument is *nine* prose arguments. Each row of
//! [`CAPTURE_RELIANCE`] states the deepest byte of a truncated row that this port's
//! argument rests on; `tests/truncated_row_reads.rs` checks every one against
//! [`field_is_captured`] **and derives the set of rows that need one** by scanning
//! `crates/` rather than by trusting the list's length. A tenth truncated row acquiring a
//! consumer turns that test red; so does a consumer reaching deeper into one that already
//! has an entry.
//!
//! ★ The derivation found a row the hand-written list did not have: `0x20800b03`
//! (`kept` 8192 of `psize` 16352) was referenced from `kayfabe_device::sweep` — by a
//! **miscited line number**. That is the next section.
//!
//! # ★★ Three miscitations, and two of them landed on TRUNCATED rows
//!
//! `[measured]` 2026-08-02 by resolving every `mode2_initctrl_ga106.h:<line>` citation in
//! the tree against the committed census's own `c_line` column:
//!
//! | claim | cited | that line is | should have been |
//! |---|---|---|---|
//! | `0x20800a3d`, *"32 bytes … 128 on this part"* | `:6246` | `0x20800a38` (24/24) | `:6228` |
//! | `0x20800a48`, *"`01 00 00 00 00 00 00 00`"* | `:6252` | **`0x20800a40` (16384/24580)** | `:6230` |
//! | `0x20800a32`, *"1664 bytes"* | `:6226` | **`0x20800b03` (8192/16352)** | `:6231` |
//!
//! Every one of the three **values** is right; only the address is wrong. ★★★ Which is the
//! memory *citing the oracle is not the oracle being right* arriving from the other
//! direction: there the citation was to a row that said nothing, here it is to the wrong
//! row entirely, and in both cases the gate that demands a `C:` citation was satisfied. A
//! citation gate checks that a claim is *sourced*; only resolving the address checks that
//! the source is the one the claim means. `tests/truncated_row_reads.rs` resolves them.
//!
//! ⊘ **The gate that does this is scoped and says so:** it can only judge a citation that
//! names its own control nearby (an id, or a `ctl_<cmd>` array name). A citation carrying
//! neither is *unattributable*, and the test counts those rather than passing them
//! silently.

/// One row of `mode2_initctrl_ga106.h` whose reply body was **never captured**.
///
/// ★ `real` is the point of the struct. Without it a reader who wants to know what this
/// control answers has to go to hardware; with it the citation carries the content, which
/// is exactly what the `C:`-citation gate cannot check on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmptyCaptureRow {
    /// The control id.
    pub cmd: u32,
    /// The reply size on the wire, from the C's own row.
    pub psize: usize,
    /// The line of `src/qemu/mode2_initctrl_ga106.h` the row sits on.
    pub c_line: u32,
    /// ★★★ What a **real GA106** answered — `[measured]` 2026-08-01, see the module docs.
    /// Empty only for `0x20800a70`, whose `psize` is genuinely zero.
    pub real: &'static [u8],
    /// `true` when the empty capture happens to agree with hardware anyway. ⚠ Two rows, and
    /// neither is distinguishable from the nine wrong ones *by anything in the capture*.
    pub coincides: bool,
}

/// Every `dlen = 0` row of the C artifact's GA106 table, with what hardware said.
///
/// ⊘ This is **not** the rule — [`captured_row_evidence`] is. This is the evidence, kept so
/// that a triage row citing one of these can quote a value instead of an address.
pub const EMPTY_CAPTURE_ROWS: &[EmptyCaptureRow] = &[
    EmptyCaptureRow {
        cmd: 0x2080_2a06,
        psize: 4,
        c_line: 6213,
        real: &[0x10, 0x00, 0x00, 0x00],
        coincides: false,
    },
    EmptyCaptureRow {
        cmd: 0x2080_017e,
        psize: 8,
        c_line: 6215,
        real: &[0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00],
        coincides: false,
    },
    EmptyCaptureRow {
        cmd: 0x2080_0af3,
        psize: 2,
        c_line: 6216,
        real: &[0x01, 0x01],
        coincides: false,
    },
    EmptyCaptureRow {
        cmd: 0x2080_2a08,
        psize: 4,
        c_line: 6233,
        real: &[0x00, 0x50, 0x00, 0x00],
        coincides: false,
    },
    EmptyCaptureRow {
        cmd: 0xa06f_0103,
        psize: 3,
        c_line: 6234,
        real: &[0x01, 0x00, 0x00],
        coincides: false,
    },
    EmptyCaptureRow {
        cmd: 0xa06f_0104,
        psize: 4,
        c_line: 6236,
        real: &[0x0b, 0x00, 0x00, 0x00],
        coincides: false,
    },
    // ⚠ Reached only by a CLIENT query (`nvidia-smi -q` → `NV2080_CTRL_GPU_INFO_INDEX_-
    // GPU_SMC_MODE`, `ogkm-580: subdevice_ctrl_gpu_kernel.c:232-266`), never by
    // `RmInitAdapter`. It answers zero because SMC is disabled on this part — a fact about
    // the silicon, not about the capture.
    EmptyCaptureRow {
        cmd: 0x2080_0a4c,
        psize: 4,
        c_line: 6243,
        real: &[0x00, 0x00, 0x00, 0x00],
        coincides: true,
    },
    // ⚠ `psize = 0`: the only row in the table for which "empty" is checkable without
    // hardware, and the reason the predicate keys on `psize` rather than on an id list.
    EmptyCaptureRow {
        cmd: 0x2080_0a70,
        psize: 0,
        c_line: 6244,
        real: &[],
        coincides: true,
    },
    // ⚠ NOT A CONSTANT, and not a state either: `flags` is the struct's only field and it
    // is `[IN]` (`ogkm-580: ctrl2080internal.h:2149-2158`), so the reply is an **echo of the
    // request**. `0x31` is `ALL | CLEAN | WAIT_FB_PULL` and `0x11` is `ALL | CLEAN` — the
    // two words `kbusVerifyBar2_GM107`'s call sites pass. ★★★ Even a *correct* single
    // capture of this row would therefore have been a statement about one caller, and the
    // empty row it actually carries was read for four rungs as "hardware answers zero".
    // See `crate::l2evict`, where that reading is corrected.
    EmptyCaptureRow {
        cmd: 0x2080_0a6c,
        psize: 4,
        c_line: 6245,
        real: &[0x31, 0x00, 0x00, 0x00],
        coincides: false,
    },
    EmptyCaptureRow {
        cmd: 0x2080_0a4b,
        psize: 4,
        c_line: 6257,
        real: &[0x00, 0x00, 0x01, 0x04],
        coincides: false,
    },
    EmptyCaptureRow {
        cmd: 0x2080_0aac,
        psize: 4,
        c_line: 6258,
        real: &[0x00, 0x00, 0x01, 0x00],
        coincides: false,
    },
];

/// One row of `mode2_initctrl_ga106.h` whose reply body was captured only in **part**.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TruncatedRow {
    /// The control id.
    pub cmd: u32,
    /// The reply size on the wire, from the C's own row.
    pub psize: usize,
    /// Bytes the recorder kept. Always `0 < kept < psize`.
    pub kept: usize,
    /// The line of `src/qemu/mode2_initctrl_ga106.h` the row sits on.
    pub c_line: u32,
    /// ★★★ How many bytes of the **kept** prefix are themselves zero, counted from its end.
    ///
    /// This field exists to kill one specific hypothesis, and it is the reason this table
    /// carries a number nobody reads at runtime. If `dlen` were *trailing-zero-trimmed* —
    /// which is how the C's own decoder was once read — then a truncated row would not be a
    /// defect at all: the tail really would be zeros and zero-filling it would be correct.
    /// **A trimmer cannot leave a zero byte at the end.** Every one of the sixteen rows has
    /// this greater than zero, and `0x20800a40` has 15 833.
    pub trailing_zeros_kept: usize,
}

/// ★★★ Every `0 < dlen < psize` row of the C artifact's GA106 table.
///
/// `[measured]` 2026-08-01 by parsing the table at `nvidia-gpu-passthrough` rev `8baf4f2`
/// — 56 rows: **29 complete, 16 truncated, 11 empty**. ⊘ The truncated class is *larger*
/// than the empty class that [`BodyNeverCaptured`](OracleRowError::BodyNeverCaptured) was
/// built for, and only half the table is a complete capture.
///
/// ⚠ **No hardware body has been taken for any of these.** That is #159 and it is not done.
/// This table says *which rows are short and by how much*, which is a statement about the
/// capture; it says nothing about what the missing bytes contain.
///
/// ★ The mechanism behind the two largest is visible in the numbers: `0x20800a22` kept
/// 16 376 = 16 384 − 8 and `0x20800a40` kept 16 384 — one GSP message-queue element's worth.
/// The recorder did not follow continuation elements, so the capture stops at an element
/// boundary rather than at a field boundary.
pub const TRUNCATED_ROWS: &[TruncatedRow] = &[
    TruncatedRow {
        cmd: 0x2080_0a49,
        psize: 24,
        kept: 16,
        c_line: 6208,
        trailing_zeros_kept: 5,
    },
    TruncatedRow {
        cmd: 0x2080_2a0f,
        psize: 28,
        kept: 16,
        c_line: 6212,
        trailing_zeros_kept: 15,
    },
    TruncatedRow {
        cmd: 0x2080_0a9f,
        psize: 184,
        kept: 176,
        c_line: 6217,
        trailing_zeros_kept: 43,
    },
    TruncatedRow {
        cmd: 0x2080_0a1f,
        psize: 184,
        kept: 176,
        c_line: 6218,
        trailing_zeros_kept: 153,
    },
    TruncatedRow {
        cmd: 0x2080_0a22,
        psize: 34592,
        kept: 16376,
        c_line: 6221,
        trailing_zeros_kept: 12053,
    },
    TruncatedRow {
        cmd: 0x2080_0a34,
        psize: 72,
        kept: 64,
        c_line: 6225,
        trailing_zeros_kept: 55,
    },
    TruncatedRow {
        cmd: 0x2080_0b03,
        psize: 16352,
        kept: 8192,
        c_line: 6226,
        trailing_zeros_kept: 8119,
    },
    TruncatedRow {
        cmd: 0x2080_0301,
        psize: 20,
        kept: 16,
        c_line: 6235,
        trailing_zeros_kept: 11,
    },
    TruncatedRow {
        cmd: 0x0073_0107,
        psize: 12,
        kept: 8,
        c_line: 6240,
        trailing_zeros_kept: 2,
    },
    TruncatedRow {
        cmd: 0x2080_0a41,
        psize: 8204,
        kept: 8200,
        c_line: 6249,
        trailing_zeros_kept: 6924,
    },
    TruncatedRow {
        cmd: 0x2080_01b0,
        psize: 1284,
        kept: 1280,
        c_line: 6250,
        trailing_zeros_kept: 1117,
    },
    TruncatedRow {
        cmd: 0x2080_0a40,
        psize: 24580,
        kept: 16384,
        c_line: 6252,
        trailing_zeros_kept: 15833,
    },
    TruncatedRow {
        cmd: 0x2080_1112,
        psize: 3212,
        kept: 3208,
        c_line: 6253,
        trailing_zeros_kept: 2104,
    },
    TruncatedRow {
        cmd: 0x2080_0a01,
        psize: 36,
        kept: 32,
        c_line: 6261,
        trailing_zeros_kept: 2,
    },
    TruncatedRow {
        cmd: 0x2080_0ac6,
        psize: 4104,
        kept: 4072,
        c_line: 6262,
        trailing_zeros_kept: 4071,
    },
    TruncatedRow {
        cmd: 0x2080_0adf,
        psize: 8388,
        kept: 8368,
        c_line: 6263,
        trailing_zeros_kept: 502,
    },
];

/// The row for `cmd`, if the C artifact captured only part of its body.
#[must_use]
pub fn truncated_row(cmd: u32) -> Option<&'static TruncatedRow> {
    TRUNCATED_ROWS.iter().find(|r| r.cmd == cmd)
}

/// ★★★ One statement of the form *"this port's argument about truncated row `cmd` rests on
/// the capture's bytes `[0, read_end)` and on nothing after them"*.
///
/// ⊘ It is **not** a claim that those bytes are correct — [`field_is_captured`]'s docs say
/// why that is a narrower thing than it looks. It is the claim that they came off the
/// recorder rather than out of zero-fill, which is the only property the capture can settle
/// about itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureReliance {
    /// The truncated control this is about. Must appear in [`TRUNCATED_ROWS`].
    pub cmd: u32,
    /// The deepest byte offset of the capture that this port's argument rests on, exclusive.
    ///
    /// ★ **Zero is a real answer**, and two rows carry it: a control this port refuses, and
    /// one whose reply it re-encodes from the guest's own request. Neither decodes a byte of
    /// the capture, so neither can be hurt by a short one.
    pub read_end: usize,
    /// The files whose argument this is. Each is checked to exist and to mention `cmd`, so a
    /// site cannot be aspirational.
    pub sites: &'static [&'static str],
    /// How `read_end` was arrived at. ⚠ A number without this is a guess with a decimal
    /// point.
    pub why: &'static str,
}

/// ★★★ Every truncated row this tree references, with the extent of it that is load-bearing.
///
/// ⚠ **The list is not the gate.** `tests/truncated_row_reads.rs` derives the set of
/// truncated rows referenced anywhere under `crates/` and requires it to equal the set here
/// — so shortening this list turns that test red rather than quietly shrinking the universe
/// it quantifies over (memory: *gates quantified over a LIST*). Deriving it is what surfaced
/// `0x20800b03`, which was in the universe only because of a miscited line number.
///
/// `[measured]` 2026-08-02 — see the module docs for the offsets each row was measured at.
pub const CAPTURE_RELIANCE: &[CaptureReliance] = &[
    CaptureReliance {
        cmd: 0x2080_0a22,
        read_end: 16376,
        sites: &[
            "crates/kayfabe-abi/src/grstatic.rs",
            "crates/kayfabe-abi/tests/gr_static_info.rs",
        ],
        why: "the fixture `ga106_ctl_20800a22.bin` IS the kept prefix, and \
              `assert_prefix_matches_for` compares exactly that many bytes — the one site \
              that already went through `field_is_captured`. Only `engineCaps[0]`'s 4324 \
              bytes are ever read past the test",
    },
    CaptureReliance {
        cmd: 0x2080_0a1f,
        read_end: 176,
        sites: &[
            "crates/kayfabe-abi/src/grstatic.rs",
            "crates/kayfabe-abi/tests/gr_static_info.rs",
        ],
        why: "the fixture `ga106_ctl_20800a1f.bin` IS the kept prefix; `GA106_GR_CAPS` \
              transcribes `engineCaps[0].capsTbl`, the first 23 bytes",
    },
    CaptureReliance {
        cmd: 0x2080_0a9f,
        read_end: 176,
        sites: &["crates/kayfabe-abi/tests/gvaspace_pdes.rs"],
        why: "the fixture `ga106_ctl_20800a9f.bin` IS the kept prefix. ⊘ That test then \
              pads it to the struct's 184 bytes with ITS OWN zeros and round-trips them; \
              those eight bytes are `levels[5]`'s aperture and pageShift, they are declared \
              in that file as this port's rather than silicon's, and they are NOT counted \
              here",
    },
    CaptureReliance {
        cmd: 0x2080_0a41,
        read_end: 4108,
        sites: &[
            "crates/kayfabe-abi/src/regaccessmap.rs",
            "crates/kayfabe-device/tests/user_register_access_map.rs",
        ],
        why: "`profilingRangesSize` sits at 4104 and the module quotes its value; the \
              embedded `ORACLE_PREFIX` is the first 1277 bytes — the two size words plus \
              the whole 1269-byte gzip member. The four uncaptured bytes are \
              `profilingRanges[4092..4096]`, which `profilingRangesSize = 0` puts out of \
              reach of any reader",
    },
    CaptureReliance {
        cmd: 0x2080_1112,
        read_end: 1112,
        sites: &["crates/kayfabe-device/tests/init_tables.rs"],
        why: "12-byte header plus all 11 entries the capture's own `numEntries` declares, \
              at 100 bytes each. `ORACLE_ENGINE_ENTRIES` embeds six of those eleven — \
              entries 0, 4, 5, 6, 7 and 10 — so its deepest byte is the end of entry 10. \
              The four uncaptured bytes are the last of `entries[31]`'s `engineName`",
    },
    CaptureReliance {
        cmd: 0x2080_0a40,
        read_end: 580,
        sites: &[
            "crates/kayfabe-abi/src/deviceinfo.rs",
            "crates/kayfabe-device/tests/internal_device_info_table.rs",
        ],
        why: "4-byte header plus all 12 entries the capture's own `numEntries` declares, at \
              48 bytes each. `ORACLE_FIVE` embeds five of the twelve, ending at 388. ★ This \
              is the row whose 8196-byte shortfall `deviceinfo`'s header calls a fail-open — \
              and the reason it is harmless is exactly that 580 is far inside 16384",
    },
    CaptureReliance {
        cmd: 0x2080_01b0,
        read_end: 164,
        sites: &[
            "crates/kayfabe-abi/src/falconinfo.rs",
            "crates/kayfabe-device/tests/constructed_falcon_info.rs",
        ],
        why: "4-byte count plus all 8 falcons the capture's own `numConstructedFalcons` \
              declares, at 20 bytes each; `ORACLE_PREFIX` embeds the 163 non-zero bytes of \
              exactly that. The four uncaptured bytes are the last word of \
              `constructedFalconsTable[63]`, an entry the count puts out of reach",
    },
    CaptureReliance {
        cmd: 0x2080_0301,
        read_end: 0,
        sites: &["crates/kayfabe-abi/src/eventnotify.rs"],
        why: "NOT A READ. Every member is `[IN]` and this port re-encodes the reply from the \
              DECODED REQUEST rather than from any capture — see that module's `⊘ The reply \
              is a PURE STATUS` section. The four uncaptured bytes are `info16` and its pad; \
              nothing here has ever read them, so the truncation cannot reach this port",
    },
    CaptureReliance {
        cmd: 0x2080_2a0f,
        read_end: 0,
        sites: &["crates/kayfabe-device/src/sweep.rs"],
        why: "NOT A READ. `RefusalHalts`: the copy-engine topology triple is refused whole, \
              so no byte of this row is decoded. ⊘ If it is ever served, this row must be \
              re-decided — the five fields `kceGetPceConfigForLceType` copies out span 20 \
              bytes of a 28-byte reply and only 16 were kept",
    },
];

/// The reliance statement for `cmd`, if this tree has one.
#[must_use]
pub fn capture_reliance(cmd: u32) -> Option<&'static CaptureReliance> {
    CAPTURE_RELIANCE.iter().find(|r| r.cmd == cmd)
}

/// Rows of `mode2_initctrl_ga106.h` whose capture is **complete** (`dlen >= psize`),
/// `[measured]` 2026-08-01 at `nvidia-gpu-passthrough` rev `8baf4f2`.
pub const COMPLETE_ROWS_TOTAL: usize = 29;

/// The number of rows in `mode2_initctrl_ga106.h`, `[measured]` by counting the table at
/// `nvidia-gpu-passthrough` rev `8baf4f2` on 2026-08-01. Pinned so that
/// [`EMPTY_CAPTURE_ROWS`]'s share is a stated fraction rather than a bare count.
pub const CAPTURED_ROWS_TOTAL: usize = 56;

/// What a captured row is evidence **of**.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapturedEvidence {
    /// The reply is genuinely empty, because the control's `psize` is zero. Checkable from
    /// the row alone.
    NoBodyExists,
    /// ★ The recorder kept **the whole body** — `dlen >= psize`. This is the only variant
    /// that licenses reading the row as a reply.
    ///
    /// ⊘ Renamed from `Body { kept, psize }` when the predicate was widened to cover
    /// truncation. The old name could hold `kept < psize`, and every reader of it was
    /// trusting a body that was partly zero-fill; the rename is deliberate churn, so that
    /// each site had to be re-decided rather than silently inheriting the weaker meaning.
    Complete {
        /// Bytes the reply had on the wire, all of which were kept.
        psize: usize,
    },
}

/// Why a captured row cannot be read as a reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OracleRowError {
    /// ★★★ `psize > 0` and `dlen == 0`: the reply had a body and **the recorder kept none
    /// of it**. The row is a record that a control was asked, and nothing more.
    ///
    /// ⊘ It is emphatically *not* `Ok(vec![0; psize])`. Nine of the eleven rows in this
    /// state are contradicted by hardware, and the two that are not are indistinguishable
    /// from the nine — so a zero-filled body is a fabricated reply wearing the oracle's
    /// name.
    BodyNeverCaptured {
        /// The control whose body is missing.
        cmd: u32,
        /// How many bytes the reply had that were not kept.
        psize: usize,
    },
    /// ★★★ `0 < dlen < psize`: the recorder kept a **prefix** and dropped the rest. The row
    /// is evidence for its first `kept` bytes and for **nothing** after them.
    ///
    /// ⊘ This is the variant that closes the hole [`BodyNeverCaptured`] left open. A
    /// truncated row *has* a body, so it decoded cleanly and the uncaptured tail arrived as
    /// zeros — silently, and wearing the oracle's name. **Sixteen of the C table's 56 rows
    /// are in this state, a larger class than the eleven empty ones**, and the tail is not
    /// zero-trimmed: all sixteen kept-bodies themselves *end* in zero bytes, one of them in
    /// 15 833 of them, which no trailing-zero trimmer could emit ([`TRUNCATED_ROWS`]).
    ///
    /// ⚠ Refusing the row wholesale would be too strong: a field that lies entirely inside
    /// the kept prefix is as well-evidenced as any full row. [`field_is_captured`] is how a
    /// caller says which one it is, per field, instead of per row.
    BodyTruncated {
        /// The control.
        cmd: u32,
        /// Bytes the recorder kept — the row is evidence for exactly these.
        kept: usize,
        /// Bytes the reply had on the wire.
        psize: usize,
    },
    /// `dlen > psize` — the row is internally inconsistent and describes no reply.
    KeptMoreThanExists {
        /// The control.
        cmd: u32,
        /// Bytes claimed kept.
        dlen: usize,
        /// Bytes the reply is said to have.
        psize: usize,
    },
}

/// ★★★ **The rule.** Decide what a row of the C artifact's captured table is evidence of.
///
/// This is the named refusal the four-rung `0x20802a08` defect needed and did not have. It
/// is a predicate on `(psize, dlen)`, not a lookup in [`EMPTY_CAPTURE_ROWS`], so a row added
/// to the table tomorrow is covered tomorrow.
///
/// # Errors
/// [`OracleRowError::BodyNeverCaptured`] when `psize > 0 && dlen == 0` — an empty capture is
/// evidence of nothing, never evidence of emptiness.
/// [`OracleRowError::BodyTruncated`] when `0 < dlen < psize` — the row is evidence for its
/// first `dlen` bytes and for nothing after them.
/// [`OracleRowError::KeptMoreThanExists`] when `dlen > psize`.
pub const fn captured_row_evidence(
    cmd: u32,
    psize: usize,
    dlen: usize,
) -> Result<CapturedEvidence, OracleRowError> {
    if dlen > psize {
        return Err(OracleRowError::KeptMoreThanExists { cmd, dlen, psize });
    }
    if psize == 0 {
        return Ok(CapturedEvidence::NoBodyExists);
    }
    if dlen == 0 {
        return Err(OracleRowError::BodyNeverCaptured { cmd, psize });
    }
    if dlen < psize {
        return Err(OracleRowError::BodyTruncated {
            cmd,
            kept: dlen,
            psize,
        });
    }
    Ok(CapturedEvidence::Complete { psize })
}

/// ★★★ Whether the field at `[off, off + len)` lies entirely inside what the recorder kept.
///
/// This is the move a blanket row-level refusal would take away. `crate::grstatic` already
/// makes exactly this argument in prose for `0x20800a22` — *"engine 0 occupies the first
/// 4 324 bytes, so nothing that is read is missing"* — and prose cannot fail a build. A
/// caller reading a field out of a truncated row states its offset here and gets a decision
/// instead of a zero.
///
/// ⊘ `true` is **not** a claim the bytes are correct; it is the narrower claim that they
/// came from the capture rather than from zero-fill. A wrong offset inside the prefix is
/// still wrong, and that is what the per-module encoders and their fixtures are for.
#[must_use]
pub const fn field_is_captured(off: usize, len: usize, dlen: usize) -> bool {
    match off.checked_add(len) {
        Some(end) => end <= dlen,
        // An offset+length that overflows `usize` cannot be inside any capture.
        None => false,
    }
}

/// The row for `cmd`, if the C artifact recorded it with no body at all.
#[must_use]
pub fn empty_capture_row(cmd: u32) -> Option<&'static EmptyCaptureRow> {
    EMPTY_CAPTURE_ROWS.iter().find(|r| r.cmd == cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_eleven_are_all_here_and_the_share_is_stated() {
        assert_eq!(EMPTY_CAPTURE_ROWS.len(), 11);
        assert_eq!(CAPTURED_ROWS_TOTAL, 56);
        // 11/56 — stated as the multiplication so the fraction cannot drift from the count.
        assert!(EMPTY_CAPTURE_ROWS.len() * 5 < CAPTURED_ROWS_TOTAL);
        let mut ids: Vec<u32> = EMPTY_CAPTURE_ROWS.iter().map(|r| r.cmd).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), EMPTY_CAPTURE_ROWS.len(), "an id appears once");
    }

    #[test]
    fn every_empty_capture_row_is_refused_by_name_unless_no_body_exists() {
        // ★★ Quantified over the whole list, so a row added without a decision fails here.
        for r in EMPTY_CAPTURE_ROWS {
            let got = captured_row_evidence(r.cmd, r.psize, 0);
            if r.psize == 0 {
                assert_eq!(got, Ok(CapturedEvidence::NoBodyExists), "{:#x}", r.cmd);
            } else {
                assert_eq!(
                    got,
                    Err(OracleRowError::BodyNeverCaptured {
                        cmd: r.cmd,
                        psize: r.psize
                    }),
                    "{:#x} must be UNMEASURED, not zero",
                    r.cmd
                );
            }
        }
    }

    #[test]
    fn the_refusal_is_not_the_only_witness_that_zeros_would_have_been_wrong() {
        // ★★★ Deliberately does NOT go through `captured_row_evidence`. If the only
        // evidence that "the empty row is wrong" were the function that refuses empty rows,
        // then "refused" and "answered zeros" would be indistinguishable and a mutation that
        // deleted the refusal would survive. This inspects the bodies `[measured]`
        // 2026-08-01 on a real RTX 3060 directly — the ones
        // `tests/real_ga106_bodies.rs::every_claimed_hardware_body_is_the_body_in_the_trace`
        // pins against `traces/real_ga106/rpc_bodies_real_ga106.txt`.
        let contradicted: Vec<u32> = EMPTY_CAPTURE_ROWS
            .iter()
            .filter(|r| !r.real.iter().all(|&b| b == 0) || r.real.is_empty() && r.psize > 0)
            .map(|r| r.cmd)
            .collect();
        assert_eq!(
            contradicted.len(),
            9,
            "nine of the eleven empty rows are contradicted by a non-zero hardware body: {contradicted:#x?}"
        );
        // …and the two survivors are exactly the ones flagged as coincidences, each for a
        // reason recorded in the module docs.
        let coinciding: Vec<u32> = EMPTY_CAPTURE_ROWS
            .iter()
            .filter(|r| r.coincides)
            .map(|r| r.cmd)
            .collect();
        assert_eq!(coinciding, vec![0x2080_0a4c, 0x2080_0a70]);
        for r in EMPTY_CAPTURE_ROWS {
            assert_eq!(
                r.coincides,
                r.real.iter().all(|&b| b == 0),
                "{:#x}: `coincides` must be derivable from the measured body, not asserted",
                r.cmd
            );
            assert_eq!(r.real.len(), r.psize, "{:#x}: body is psize bytes", r.cmd);
        }
    }

    #[test]
    fn a_truncated_row_is_refused_by_name_and_carries_its_usable_prefix() {
        // ★★★ This test previously asserted the OPPOSITE — that `0x20800a22` decodes to
        // `Ok(Body { kept: 16376, psize: 34592 })`. That `Ok` was the hole: every caller of
        // it received a 34 592-byte body of which 18 216 bytes were zero-fill wearing the
        // oracle's name. The inversion is the finding, not a test being made to pass.
        assert_eq!(
            captured_row_evidence(0x2080_0a22, 34592, 16376),
            Err(OracleRowError::BodyTruncated {
                cmd: 0x2080_0a22,
                kept: 16376,
                psize: 34592
            })
        );
    }

    #[test]
    fn all_sixteen_truncated_rows_are_refused_and_none_is_a_zero_trim() {
        // ★★ Quantified over the whole list — a row added without a decision fails here.
        assert_eq!(TRUNCATED_ROWS.len(), 16);
        for r in TRUNCATED_ROWS {
            assert!(0 < r.kept && r.kept < r.psize, "{:#x} is truncated", r.cmd);
            assert_eq!(
                captured_row_evidence(r.cmd, r.psize, r.kept),
                Err(OracleRowError::BodyTruncated {
                    cmd: r.cmd,
                    kept: r.kept,
                    psize: r.psize
                }),
                "{:#x} must be refused, not zero-filled",
                r.cmd
            );
            // ★★★ The refutation of trailing-zero trimming, asserted per row rather than
            // narrated once. A trimmer cannot leave a zero byte at the end of what it kept,
            // so a single row with `trailing_zeros_kept == 0` would mean the trim hypothesis
            // is live again for that row and the refusal above needs re-arguing.
            assert!(
                r.trailing_zeros_kept > 0,
                "{:#x}: kept prefix ends in a non-zero byte — `dlen` could be a zero-trim \
                 here, and if it is, refusing this row is wrong",
                r.cmd
            );
        }
        // The three classes partition the table — stated as the sum so no class can be
        // widened by quietly shrinking another.
        assert_eq!(
            COMPLETE_ROWS_TOTAL + TRUNCATED_ROWS.len() + EMPTY_CAPTURE_ROWS.len(),
            CAPTURED_ROWS_TOTAL
        );
        // ⊘ …and the truncated class is the LARGER of the two defective ones, which is the
        // reason this widening was not optional.
        assert!(TRUNCATED_ROWS.len() > EMPTY_CAPTURE_ROWS.len());
        // No row may be in two classes at once.
        for r in TRUNCATED_ROWS {
            assert!(empty_capture_row(r.cmd).is_none(), "{:#x}", r.cmd);
        }
    }

    #[test]
    fn a_field_inside_the_kept_prefix_is_captured_and_one_past_it_is_not() {
        // `crate::grstatic`'s prose argument, mechanized: engine 0 of `0x20800a22` occupies
        // the first 4 324 bytes and the recorder kept 16 376, so reading it is sound…
        assert!(field_is_captured(0, 4324, 16376));
        // …while the eight-engine array's last engine is not there at all.
        assert!(!field_is_captured(30268, 4324, 16376));
        // Boundary: the last kept byte is in, one past it is out.
        assert!(field_is_captured(16375, 1, 16376));
        assert!(!field_is_captured(16376, 1, 16376));
        // A zero-length field at the boundary is trivially inside; past it is not.
        assert!(field_is_captured(16376, 0, 16376));
        assert!(!field_is_captured(16377, 0, 16376));
        // ⊘ An overflowing offset+len is not inside any capture, and must not wrap into one.
        assert!(!field_is_captured(usize::MAX, 1, 16376));
    }

    #[test]
    fn nothing_this_port_relies_on_reaches_past_what_the_recorder_kept() {
        // ★★★ The module docs' consequence table, as one assertion per row. This is the
        // check `0x20802a08` never had: there the consumer read a field the capture did not
        // contain and got a zero for it, and no test could have gone red because the
        // argument that it was fine existed only in prose.
        //
        // Each `read_end` was `[measured]` 2026-08-02 by locating this repository's embedded
        // capture bytes inside the C table's own arrays at `nvidia-gpu-passthrough` HEAD
        // `8001cb5`, whose copy of that header is byte-identical to rev `8baf4f2` — the
        // offsets are in this module's header. The universe of rows that
        // need one is derived rather than listed, by `tests/truncated_row_reads.rs`.
        assert!(!CAPTURE_RELIANCE.is_empty());
        for r in CAPTURE_RELIANCE {
            let row = truncated_row(r.cmd).unwrap_or_else(|| {
                panic!(
                    "{:#010x} has a reliance statement but is not a truncated row — either \
                     the census moved or this entry is about the wrong control",
                    r.cmd
                )
            });
            assert!(
                field_is_captured(0, r.read_end, row.kept),
                "{:#010x}: this port relies on bytes [0,{}) but the recorder kept only {} \
                 of {} — the bytes past {} are zero-fill wearing the oracle's name. {}",
                r.cmd,
                r.read_end,
                row.kept,
                row.psize,
                row.kept,
                r.why
            );
            // ⊘ And a reliance may not be a bare number: `read_end` without a derivation is
            // the shape the empty rows were read in for four rungs.
            assert!(
                r.why.len() > 40,
                "{:#010x}: `why` must say how `read_end` was arrived at",
                r.cmd
            );
            assert!(!r.sites.is_empty(), "{:#010x}: no site", r.cmd);
        }
        let mut ids: Vec<u32> = CAPTURE_RELIANCE.iter().map(|r| r.cmd).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), CAPTURE_RELIANCE.len(), "an id appears once");
        for r in CAPTURE_RELIANCE {
            assert_eq!(capture_reliance(r.cmd), Some(r));
        }
        assert_eq!(
            capture_reliance(0x2080_0a2a),
            None,
            "a complete row has none"
        );
    }

    #[test]
    fn a_reliance_that_reaches_one_byte_past_the_capture_is_refused() {
        // ★★ The gate above is only worth its line count if it can FAIL, and a gate never
        // seen to fail is not evidence (memory: *suspect the instrument first*). This is the
        // same predicate applied to the same rows shifted by one byte: every one must flip.
        for r in CAPTURE_RELIANCE {
            let row = truncated_row(r.cmd).expect("a truncated row");
            assert!(
                !field_is_captured(0, row.kept + 1, row.kept),
                "{:#010x}: the predicate must refuse one byte past the prefix",
                r.cmd
            );
            // …and the boundary itself is in, so the check is not vacuously strict.
            assert!(field_is_captured(0, row.kept, row.kept));
        }
    }

    #[test]
    fn a_full_row_is_evidence_and_0x20800a2a_is_one() {
        // ★ The other half of the class result: `0x20800a2a` carries `dlen == psize` and is
        // byte-identical to hardware for all 3712 bytes. See `crate::grinfo`.
        assert_eq!(
            captured_row_evidence(0x2080_0a2a, 3712, 3712),
            Ok(CapturedEvidence::Complete { psize: 3712 })
        );
        assert!(empty_capture_row(0x2080_0a2a).is_none());
        assert!(truncated_row(0x2080_0a2a).is_none());
    }

    #[test]
    fn an_inconsistent_row_describes_no_reply() {
        assert_eq!(
            captured_row_evidence(0xdead_beef, 4, 8),
            Err(OracleRowError::KeptMoreThanExists {
                cmd: 0xdead_beef,
                dlen: 8,
                psize: 4
            })
        );
    }
}
