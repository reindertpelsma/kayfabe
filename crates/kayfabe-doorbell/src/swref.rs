//! The swref descriptor parser — P1's *"that generator does not exist today"*.
//!
//! ## ⊘ Why this is not a C parser
//!
//! P1: *"Its input is **not parseable as C**: `NV_CTRL_VF_DOORBELL_VECTOR 11:0` and
//! `NV_VIRTUAL_FUNCTION 0x0003FFFF:0x00030000` are not C expressions, so libclang yields
//! nothing. It needs a **token-level `hi:lo` + access-code parser**."*
//!
//! ⇒ A `#define` here carries one of three shapes, and the trailing comment is the access code:
//!
//! ```text
//! #define NV_PBUS_FOO                          0x00001700 /* RW--V */   -- an offset
//! #define NV_CTRL_VF_DOORBELL_VECTOR                 11:0 /* -WXUF */   -- a bit range
//! #define NV_VIRTUAL_FUNCTION      0x0003FFFF:0x00030000 /* RW--D */   -- an aperture
//! ```
//!
//! ## The access code, derived from the corpus rather than assumed
//!
//! `[measured w823]` **24 042** coded defines across `swref/published`, **all exactly 5
//! characters**, with these per-position alphabets:
//!
//! | position | alphabet | meaning used here |
//! |---|---|---|
//! | 1 | `- C R` | **readable** iff `R` or `C`. ⊘ `C` is *constant* — it is on `VENDOR_ID` and on value defines, and it **is** readable |
//! | 2 | `- W` | **writable** iff `W` |
//! | 3 | `- A B C D E H I X` | not used here |
//! | 4 | `- U V` | not used here |
//! | 5 | `C D F G L M T V` | not used here |
//!
//! ## ★★★ Measured against the whole corpus, because a parser only ever agrees with itself
//!
//! `[w823, 416 headers]` an **independent** predicate (grep for a 5-char code) counts **24 042**
//! coded defines. The parser produces **24 004** — 99.84 % — and the 38 it declines are
//! structurally unparseable, not missed:
//!
//! | | n |
//! |---|---|
//! | parsed | **24 004** — 15 410 offsets, 8 381 bit ranges, 127 apertures, 86 structure-bit fields |
//! | ⊘ value-less **group headers** (`#define NV_MMU_PTE /* ----G */`) | 22 |
//! | ⊘ parameterised macros with a free `(i)` | 12 + 4 |
//!
//! ⊘⊘⊘ **The cross-check is what found the two real defects**, and neither was visible from the
//! parser's own tests:
//! 1. **Over-count by 1 975.** Accepting any 5-character first token made `/* Fermi and later */`
//!    parse as an access code, yielding a silently unreadable/unwritable descriptor. ⇒ Validate
//!    against the alphabet, not the length.
//! 2. **Under-count by 324.** Bit positions are **arithmetic** here — `(0*32+31-3):(0*32+4)` —
//!    and 102 of them are offsets into a *structure*, not a register. ⇒ `NV_RAMUSERD_GP_GET` is
//!    `(34*32+31):(34*32+0)` = **word 34 = byte 0x88**, the USERD geometry the doorbell plane
//!    needs. Dropping them would have been *the missing table row never denies*.
//!
//! ⚠ Positions 3–5 are deliberately **not** interpreted. §5.5 measures that `write_semantics`
//! **cannot** be recovered from these codes — *"the interrupt-pending register (write-1-to-clear)
//! and its enable-set port carry the SAME code; the invalidate trigger reads as plain
//! read-write"* — so inventing a meaning for them would be a guess dressed as generation.

/// What a `#define`'s value turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Value {
    /// `0x00001700`
    Offset(u32),
    /// `11:0`
    BitRange { hi: u8, lo: u8 },
    /// `0x0003FFFF:0x00030000`
    Aperture { hi: u32, lo: u32 },
    /// ★★★ A bit range into a **structure**, not a register: `(34*32+31):(34*32+0)`.
    ///
    /// `[measured w823]` 102 defines take this form — `NV_RAMIN_*` (the instance block) and
    /// `NV_RAMUSERD_*` (the channel cursor block). ⊘ They exceed 64 bits, so treating them as a
    /// register bit range would be wrong; they are **offsets in bits from the start of a
    /// structure**.
    ///
    /// ★ And they are load-bearing rather than trivia: `NV_RAMUSERD_GP_GET` at
    /// `(34*32+31):(34*32+0)` is word 34 ⇒ **byte offset 0x88**, which is exactly the USERD
    /// geometry the doorbell plane needs. [`Value::byte_offset`] returns it.
    StructBits { hi: u32, lo: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Descriptor {
    pub name: String,
    pub value: Value,
    /// Generated. §5.5: *"Read/write-only-ness is generated."*
    pub readable: bool,
    pub writable: bool,
}

/// Parse one line. Returns `None` for anything that is not a coded `#define` — ⊘ deliberately
/// permissive, because the headers contain prose, guards and uncoded defines, and a parser that
/// refused them would refuse the file.
pub fn parse_line(line: &str) -> Option<Descriptor> {
    let rest = line.trim().strip_prefix("#define ")?;
    let (head, tail) = rest.split_once("/*")?;
    let code = tail.trim_start().split_whitespace().next()?;
    // ⊘⊘⊘ **LENGTH ALONE IS NOT ENOUGH, AND THE FIRST VERSION OF THIS PARSER PROVED IT.**
    // `[measured w823]` accepting any 5-character first token made `/* Fermi and later */` parse
    // as an access code — `F` is not a read bit, so the descriptor came out unreadable and
    // unwritable, silently. The parser over-counted the corpus by **1 975** rows (26 017 against
    // grep's 24 042), which is how it was caught: **a parser must be cross-checked against an
    // independent count, or it only ever agrees with itself.**
    // ⇒ Validate against the per-position alphabet derived FROM the corpus (see the module docs).
    if !is_access_code(code) {
        return None;
    }
    let mut parts = head.split_whitespace();
    let name = parts.next()?.to_string();
    let value_tok = parts.next()?;
    let value = parse_value(value_tok)?;
    let b = code.as_bytes();
    Some(Descriptor {
        name,
        value,
        readable: b[0] == b'R' || b[0] == b'C',
        writable: b[1] == b'W',
    })
}

/// The per-position alphabet, measured over all 24 042 coded defines in `swref/`.
/// ⚠ A character outside these sets means the corpus has changed — the line is skipped rather
/// than guessed, and a systematic change will show up as a drop in the survey count.
fn is_access_code(c: &str) -> bool {
    let b = c.as_bytes();
    b.len() == 5
        && matches!(b[0], b'-' | b'C' | b'R')
        && matches!(b[1], b'-' | b'W')
        && matches!(b[2], b'-' | b'A' | b'B' | b'C' | b'D' | b'E' | b'H' | b'I' | b'X')
        && matches!(b[3], b'-' | b'U' | b'V')
        && matches!(b[4], b'C' | b'D' | b'F' | b'G' | b'L' | b'M' | b'T' | b'V')
}

/// ⊘⊘ **Bit positions are ARITHMETIC in this corpus, and skipping them loses real registers.**
/// `[measured w823]` the cross-check against grep left a residual of **324 lines**, and they are
/// forms like `(0*32+31-3):(0*32+4)`, `(35-3):8` and `35:(36-3)` — genuine bit ranges written as
/// expressions over the word index. ⚠ 1.3 % of the corpus silently absent is the
/// *missing table row never denies* failure, so they are evaluated rather than dropped.
///
/// A deliberately tiny evaluator: unsigned `+ - *` with parentheses, left-to-right within a
/// precedence level. ⊘ It is NOT a C expression evaluator — it handles exactly the shapes the
/// corpus contains, and anything else returns `None` and is skipped.
fn eval_expr(t: &str) -> Option<i64> {
    fn atom(b: &[u8], i: &mut usize) -> Option<i64> {
        while *i < b.len() && b[*i] == b' ' { *i += 1 }
        if *i < b.len() && b[*i] == b'(' {
            *i += 1;
            let v = add(b, i)?;
            if *i >= b.len() || b[*i] != b')' { return None }
            *i += 1;
            return Some(v);
        }
        let st = *i;
        while *i < b.len() && b[*i].is_ascii_digit() { *i += 1 }
        if st == *i { return None }
        std::str::from_utf8(&b[st..*i]).ok()?.parse().ok()
    }
    fn mul(b: &[u8], i: &mut usize) -> Option<i64> {
        let mut v = atom(b, i)?;
        while *i < b.len() && b[*i] == b'*' { *i += 1; v *= atom(b, i)?; }
        Some(v)
    }
    fn add(b: &[u8], i: &mut usize) -> Option<i64> {
        let mut v = mul(b, i)?;
        while *i < b.len() && (b[*i] == b'+' || b[*i] == b'-') {
            let op = b[*i]; *i += 1;
            let r = mul(b, i)?;
            v = if op == b'+' { v + r } else { v - r };
        }
        Some(v)
    }
    let b = t.as_bytes();
    let mut i = 0;
    let v = add(b, &mut i)?;
    if i != b.len() { return None }
    Some(v)
}

/// Split on the `:` that separates hi from lo, ignoring any inside parentheses.
fn split_range(t: &str) -> Option<(&str, &str)> {
    let mut depth = 0i32;
    for (i, c) in t.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ':' if depth == 0 => return Some((&t[..i], &t[i + 1..])),
            _ => {}
        }
    }
    None
}

fn parse_value(t: &str) -> Option<Value> {
    if let Some((a, b)) = split_range(t) {
        if let (Some(a16), Some(b16)) = (a.strip_prefix("0x"), b.strip_prefix("0x")) {
            return Some(Value::Aperture {
                hi: u32::from_str_radix(a16, 16).ok()?,
                lo: u32::from_str_radix(b16, 16).ok()?,
            });
        }
        let (hi, lo) = (eval_expr(a)?, eval_expr(b)?);
        if hi < 0 || lo < 0 { return None }
        // ⊘ A range beyond one 64-bit word is not a register field — it is a structure offset.
        // Distinguishing them is the point; squeezing both into one variant would silently
        // truncate USERD's word-34 cursor into a nonsense bit index.
        return Some(if hi < 64 && lo < 64 {
            Value::BitRange { hi: hi as u8, lo: lo as u8 }
        } else {
            Value::StructBits { hi: hi as u32, lo: lo as u32 }
        });
    }
    if let Some(h) = t.strip_prefix("0x") {
        return Some(Value::Offset(u32::from_str_radix(h, 16).ok()?));
    }
    // Decimal offsets appear too (e.g. a bare `0`).
    Some(Value::Offset(t.parse::<u32>().ok()?))
}

impl Value {
    /// The byte offset a structure field starts at, for [`Value::StructBits`].
    pub fn byte_offset(&self) -> Option<u32> {
        match self {
            Value::StructBits { lo, .. } => Some(lo / 8),
            Value::Offset(o) => Some(*o),
            _ => None,
        }
    }
}

pub fn parse_header(text: &str) -> Vec<Descriptor> {
    text.lines().filter_map(parse_line).collect()
}

/// ★★★ THE OVERLAY CROSS-CHECK — §5.5's build gate.
///
/// > *"The overlay is small, but it is hand-maintained, and **it must fail the build when a
/// > register in the generated set has no entry**."*
///
/// ⇒ Every name the hand-maintained `write_semantics` overlay mentions **must exist in the
/// generated set**. A vendor rename then fails the build instead of silently dropping the entry
/// — which is exactly how *a capture-derived table expires as a vendor regression*.
///
/// ⚠ Note the direction: we do **not** demand an overlay entry for all 24 042 registers. Only
/// that every overlay name is real.
pub fn overlay_names_all_exist(generated: &[Descriptor], overlay: &[&str]) -> Result<(), Vec<String>> {
    let missing: Vec<String> = overlay
        .iter()
        .filter(|n| !generated.iter().any(|d| d.name == **n))
        .map(|n| (*n).to_string())
        .collect();
    if missing.is_empty() { Ok(()) } else { Err(missing) }
}
