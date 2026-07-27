//! A scanner for the narrow, rigidly regular C subset NVIDIA's FINN generator
//! emits into the SDK / `generated/` headers.
//!
//! Everything it cannot understand is an **error**, never a skip and never a
//! guess. That is the whole point: the L11 bug class (`abi_struct_truncation`,
//! `nvos64_abi_fix`, the `cuCtxCreate`-401 missing alloc size) is what happens
//! when a human silently gets one field wrong, and a parser that shrugs at an
//! unfamiliar declaration reproduces it faithfully.

use crate::ctype::RawField;
use std::collections::BTreeMap;
use std::fmt;

/// Everything the scanner refuses to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// A `typedef struct` block that never closes.
    UnterminatedStruct {
        /// Line the typedef opened on.
        line: usize,
    },
    /// A declaration inside a struct body the scanner cannot decompose.
    BadDeclaration {
        /// The raw text.
        text: String,
        /// 1-based line.
        line: usize,
    },
    /// An array length that is neither a literal nor a known `#define`.
    UnknownArrayLength {
        /// The bracket contents.
        expr: String,
        /// 1-based line.
        line: usize,
    },
    /// A nested aggregate inside a struct body — the scanner deliberately does
    /// not flatten these, because flattening changes the emitted Rust shape.
    NestedAggregate {
        /// 1-based line.
        line: usize,
    },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnterminatedStruct { line } => {
                write!(f, "unterminated struct opened at line {line}")
            }
            Self::BadDeclaration { text, line } => {
                write!(f, "line {line}: cannot parse declaration `{text}`")
            }
            Self::UnknownArrayLength { expr, line } => {
                write!(
                    f,
                    "line {line}: array length `{expr}` is not a literal or known #define"
                )
            }
            Self::NestedAggregate { line } => {
                write!(
                    f,
                    "line {line}: nested struct/union body — not supported, add it explicitly"
                )
            }
        }
    }
}

/// Replace comments with spaces, preserving every newline so line numbers stay
/// exact.
#[must_use]
pub fn strip_comments(src: &str) -> String {
    let b: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == '/' && i + 1 < b.len() && b[i + 1] == '/' {
            while i < b.len() && b[i] != '\n' {
                i += 1;
            }
        } else if b[i] == '/' && i + 1 < b.len() && b[i + 1] == '*' {
            i += 2;
            while i < b.len() && !(b[i] == '*' && i + 1 < b.len() && b[i + 1] == '/') {
                if b[i] == '\n' {
                    out.push('\n');
                }
                i += 1;
            }
            i = (i + 2).min(b.len());
            out.push(' ');
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

/// A raw `typedef struct`/`typedef union` body located in a header.
#[derive(Debug, Clone)]
pub struct RawAggregate {
    /// 1-based line of the `typedef` keyword.
    pub line: usize,
    /// `true` for `typedef union`.
    pub is_union: bool,
    /// The declarations between the braces, already comment-stripped.
    pub body: String,
    /// 1-based line the body starts on.
    pub body_line: usize,
}

/// Locate every `typedef struct { … } NAME;` / `typedef union { … } NAME;` in a
/// comment-stripped header.
///
/// Duplicates are **kept**, not resolved: a header may legitimately define two
/// `#if` variants of a struct we do not care about, but if the *requested*
/// struct is one of them the caller must refuse (`main.rs`'s `build_struct`
/// does, naming the count). Resolving a `#if` here would be an invisible coin
/// flip.
///
/// # Errors
///
/// [`ParseError::UnterminatedStruct`].
pub fn find_aggregates(clean: &str) -> Result<BTreeMap<String, Vec<RawAggregate>>, ParseError> {
    let chars: Vec<char> = clean.chars().collect();
    let mut line_of = Vec::with_capacity(chars.len() + 1);
    let mut line = 1usize;
    for &c in &chars {
        line_of.push(line);
        if c == '\n' {
            line += 1;
        }
    }
    line_of.push(line);

    let mut out: BTreeMap<String, Vec<RawAggregate>> = BTreeMap::new();
    let text: String = chars.iter().collect();

    let mut search_from = 0usize;
    while let Some(rel) = find_typedef_aggregate(&text[search_from..]) {
        let kw_start = search_from + rel.0;
        let is_union = rel.1;
        // Find the opening brace after the keyword.
        let Some(open_rel) = text[kw_start..].find('{') else {
            return Err(ParseError::UnterminatedStruct {
                line: line_of[kw_start],
            });
        };
        let open = kw_start + open_rel;
        // Match braces.
        let mut depth = 0usize;
        let mut close = None;
        for (i, c) in text[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(open + i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(close) = close else {
            return Err(ParseError::UnterminatedStruct {
                line: line_of[kw_start],
            });
        };
        // The typedef name runs from just past the '}' to the ';'.
        let Some(semi_rel) = text[close..].find(';') else {
            return Err(ParseError::UnterminatedStruct {
                line: line_of[kw_start],
            });
        };
        let semi = close + semi_rel;
        let name = text[close + 1..semi].trim().to_string();
        if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            let agg = RawAggregate {
                line: line_of[kw_start],
                is_union,
                body: text[open + 1..close].to_string(),
                body_line: line_of[open],
            };
            out.entry(name).or_default().push(agg);
        }
        search_from = semi + 1;
    }
    Ok(out)
}

/// Byte offset of the next `typedef struct`/`typedef union` keyword, plus
/// whether it is a union.
fn find_typedef_aggregate(hay: &str) -> Option<(usize, bool)> {
    let mut at = 0usize;
    loop {
        let rel = hay[at..].find("typedef")?;
        let pos = at + rel;
        let rest = hay[pos + "typedef".len()..].trim_start();
        if let Some(r) = rest.strip_prefix("struct")
            && r.starts_with([' ', '\t', '\n', '{'])
        {
            return Some((pos, false));
        }
        if let Some(r) = rest.strip_prefix("union")
            && r.starts_with([' ', '\t', '\n', '{'])
        {
            return Some((pos, true));
        }
        at = pos + "typedef".len();
    }
}

/// What [`parse_fields`] recovers from a struct body: the scalar fields, plus a
/// trailing flexible-array member as `(field name, element type spelling)`.
pub type ParsedBody = (Vec<RawField>, Option<(String, String)>);

/// Split a struct body into fields.
///
/// Returns the scalar fields plus, if the last declaration was `T name[]`, the
/// flexible-array member's `(name, element_type_spelling)`.
///
/// # Errors
///
/// Any [`ParseError`] variant; in particular a declaration this scanner does not
/// recognise is [`ParseError::BadDeclaration`] rather than a skip.
pub fn parse_fields(
    body: &str,
    body_line: usize,
    defines: &BTreeMap<String, usize>,
) -> Result<ParsedBody, ParseError> {
    let mut fields = Vec::new();
    let mut flexible = None;

    let mut line = body_line;
    for raw_decl in body.split(';') {
        let decl_start_line = line;
        line += raw_decl.chars().filter(|&c| c == '\n').count();
        let decl = raw_decl.trim();
        if decl.is_empty() {
            continue;
        }
        if decl.contains('{') || decl.contains('}') {
            return Err(ParseError::NestedAggregate {
                line: decl_start_line,
            });
        }
        // The line number of the declaration itself is the first non-blank line
        // of the fragment.
        let lead_blank = raw_decl
            .chars()
            .take_while(|&c| c.is_whitespace())
            .filter(|&c| c == '\n')
            .count();
        let decl_line = decl_start_line + lead_blank;

        let (core, explicit_align) = strip_alignment(decl);
        let core = core.trim();

        // Split off an array suffix.
        let (core, array_expr) = match core.find('[') {
            Some(open) => {
                let close = core.rfind(']').ok_or_else(|| ParseError::BadDeclaration {
                    text: decl.to_string(),
                    line: decl_line,
                })?;
                if close < open {
                    return Err(ParseError::BadDeclaration {
                        text: decl.to_string(),
                        line: decl_line,
                    });
                }
                (
                    core[..open].trim(),
                    Some(core[open + 1..close].trim().to_string()),
                )
            }
            None => (core, None),
        };

        let mut toks: Vec<&str> = core.split_whitespace().collect();
        if toks.len() < 2 {
            return Err(ParseError::BadDeclaration {
                text: decl.to_string(),
                line: decl_line,
            });
        }
        let name = toks.pop().expect("len >= 2").to_string();
        let ty = toks.join(" ");
        if name.starts_with('*') || ty.contains('*') {
            return Err(ParseError::BadDeclaration {
                text: decl.to_string(),
                line: decl_line,
            });
        }

        match array_expr.as_deref() {
            Some("") => {
                // Flexible array member — must be last; `split(';')` guarantees
                // ordering, and any later field would simply be laid out after
                // it, which C forbids, so reject that explicitly.
                if flexible.is_some() {
                    return Err(ParseError::BadDeclaration {
                        text: decl.to_string(),
                        line: decl_line,
                    });
                }
                flexible = Some((name, ty));
            }
            Some(expr) => {
                if flexible.is_some() {
                    return Err(ParseError::BadDeclaration {
                        text: decl.to_string(),
                        line: decl_line,
                    });
                }
                let n =
                    resolve_len(expr, defines).ok_or_else(|| ParseError::UnknownArrayLength {
                        expr: expr.to_string(),
                        line: decl_line,
                    })?;
                fields.push(RawField {
                    ty,
                    name,
                    array_len: Some(n),
                    explicit_align,
                    line: decl_line,
                });
            }
            None => {
                if flexible.is_some() {
                    return Err(ParseError::BadDeclaration {
                        text: decl.to_string(),
                        line: decl_line,
                    });
                }
                fields.push(RawField {
                    ty,
                    name,
                    array_len: None,
                    explicit_align,
                    line: decl_line,
                });
            }
        }
    }
    Ok((fields, flexible))
}

/// Peel `NV_DECLARE_ALIGNED(<decl>, N)` and a trailing `NV_ALIGN_BYTES(N)`.
fn strip_alignment(decl: &str) -> (String, Option<usize>) {
    let d = decl.trim();
    if let Some(rest) = d.strip_prefix("NV_DECLARE_ALIGNED")
        && let Some(open) = rest.find('(')
        && let Some(close) = rest.rfind(')')
        && close > open
    {
        let inner = &rest[open + 1..close];
        if let Some(comma) = inner.rfind(',') {
            let align = inner[comma + 1..].trim().parse::<usize>().ok();
            return (inner[..comma].trim().to_string(), align);
        }
    }
    if let Some(at) = d.find("NV_ALIGN_BYTES")
        && let Some(open) = d[at..].find('(')
        && let Some(close) = d[at..].find(')')
        && close > open
    {
        let align = d[at + open + 1..at + close].trim().parse::<usize>().ok();
        let mut core = String::from(&d[..at]);
        core.push_str(&d[at + close + 1..]);
        return (core, align);
    }
    (d.to_string(), None)
}

/// An array length is a decimal/hex literal, a `#define` we were given, or an
/// error. No arithmetic — if NVIDIA ever writes `[N + 1]` the generator stops.
fn resolve_len(expr: &str, defines: &BTreeMap<String, usize>) -> Option<usize> {
    let e = expr.trim().trim_end_matches(['U', 'u']);
    if let Some(hex) = e.strip_prefix("0x").or_else(|| e.strip_prefix("0X")) {
        return usize::from_str_radix(hex, 16).ok();
    }
    if let Ok(n) = e.parse::<usize>() {
        return Some(n);
    }
    defines.get(e).copied()
}

/// Scan `#define NAME (0x1234U)` / `#define NAME 100U` style object-like macros
/// with an integral value. Anything else is ignored (it is not an error to have
/// macros we do not use).
#[must_use]
pub fn scan_defines(clean: &str) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for raw in clean.lines() {
        let l = raw.trim();
        let Some(rest) = l.strip_prefix("#define") else {
            continue;
        };
        let rest = rest.trim();
        let mut it = rest.splitn(2, char::is_whitespace);
        let Some(name) = it.next() else { continue };
        if name.contains('(') {
            continue; // function-like macro
        }
        let Some(val) = it.next() else { continue };
        let val = val.trim().trim_start_matches('(').trim();
        let val = val
            .split([')', ' ', '\t'])
            .next()
            .unwrap_or("")
            .trim_end_matches(['U', 'u']);
        let parsed = if let Some(h) = val.strip_prefix("0x").or_else(|| val.strip_prefix("0X")) {
            usize::from_str_radix(h, 16).ok()
        } else {
            val.parse::<usize>().ok()
        };
        if let Some(v) = parsed {
            out.insert(name.to_string(), v);
        }
    }
    out
}

/// Scan an X-macro list line-by-line: `PREFIX(arg, …, VALUE)` where the last
/// argument is an integral literal and there are exactly `arity` arguments.
///
/// `rpc_global_enums.h` uses two forms — `X(RM, ALLOC_ROOT, 2)` for RPC function
/// IDs (arity 3, decimal) and `E(GSP_INIT_DONE, 0x1001)` for event IDs (arity 2,
/// hex). Returns `(name, value)` where `name` is the second-to-last argument.
///
/// Malformed lines are skipped because the same file carries the X-macro
/// *definition* boilerplate; the caller asserts on the expected count so a
/// silently-empty scan cannot pass for success.
#[must_use]
pub fn scan_macro_list(clean: &str, prefix: &str, arity: usize) -> Vec<(String, u32)> {
    let open = format!("{prefix}(");
    let mut out = Vec::new();
    for l in clean.lines() {
        let t = l.trim();
        let Some(rest) = t.strip_prefix(&open) else {
            continue;
        };
        let Some(close) = rest.find(')') else {
            continue;
        };
        let parts: Vec<&str> = rest[..close].split(',').map(str::trim).collect();
        if parts.len() != arity {
            continue;
        }
        let raw = parts[arity - 1].trim_end_matches(['U', 'u']);
        let v = if let Some(h) = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
            u32::from_str_radix(h, 16).ok()
        } else {
            raw.parse::<u32>().ok()
        };
        let Some(v) = v else { continue };
        out.push((parts[arity - 2].to_string(), v));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_comments_preserves_line_numbers() {
        let src = "a\n/* two\nline */b\n// eol\nc\n";
        let out = strip_comments(src);
        assert_eq!(out.lines().count(), src.lines().count());
        assert!(out.contains('b'));
        assert!(!out.contains("two"));
        assert!(!out.contains("eol"));
    }

    #[test]
    fn finds_a_typedef_struct_and_its_body() {
        let src = "typedef struct\n{\n    NvU32 a;\n    NvU64 b;\n} FOO;\n";
        let aggs = find_aggregates(&strip_comments(src)).expect("parses");
        let foo = &aggs.get("FOO").expect("FOO")[0];
        assert_eq!(foo.line, 1);
        assert!(!foo.is_union);
        assert!(foo.body.contains("NvU32 a"));
    }

    #[test]
    fn finds_a_named_typedef_struct() {
        let src = "typedef struct NV0000_ALLOC_PARAMETERS {\n NvHandle hClient;\n} NV0000_ALLOC_PARAMETERS;\n";
        let aggs = find_aggregates(&strip_comments(src)).expect("parses");
        assert_eq!(aggs.get("NV0000_ALLOC_PARAMETERS").map(Vec::len), Some(1));
    }

    /// Two `#if` arms of the same struct are BOTH recorded, so the caller can
    /// refuse; picking one here would be an invisible coin flip.
    #[test]
    fn duplicate_struct_is_recorded_twice_not_resolved() {
        let src = "typedef struct { NvU32 a; } FOO;\ntypedef struct { NvU64 a; } FOO;\n";
        let aggs = find_aggregates(&strip_comments(src)).expect("parses");
        assert_eq!(aggs.get("FOO").map(Vec::len), Some(2));
    }

    #[test]
    fn parses_both_alignment_spellings() {
        let (core, a) = strip_alignment("NvU64    limit NV_ALIGN_BYTES(8)");
        assert_eq!(
            core.split_whitespace().collect::<Vec<_>>(),
            ["NvU64", "limit"]
        );
        assert_eq!(a, Some(8));
        let (core, a) = strip_alignment("NV_DECLARE_ALIGNED(NvU64 physAddress, 8)");
        assert_eq!(core, "NvU64 physAddress");
        assert_eq!(a, Some(8));
        let (core, a) = strip_alignment("NvU32 flags");
        assert_eq!(core, "NvU32 flags");
        assert_eq!(a, None);
    }

    #[test]
    fn parses_an_array_with_a_define_length() {
        let mut d = BTreeMap::new();
        d.insert("NV_PROC_NAME_MAX_LENGTH".to_string(), 100usize);
        let (fields, flex) =
            parse_fields(" char processName[NV_PROC_NAME_MAX_LENGTH];\n", 1, &d).expect("parses");
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].array_len, Some(100));
        assert!(flex.is_none());
    }

    #[test]
    fn an_unknown_array_length_is_refused() {
        let d = BTreeMap::new();
        assert_eq!(
            parse_fields(" char n[SOMETHING];", 4, &d),
            Err(ParseError::UnknownArrayLength {
                expr: "SOMETHING".into(),
                line: 4
            })
        );
    }

    #[test]
    fn a_flexible_array_member_is_reported_separately() {
        let d = BTreeMap::new();
        let (fields, flex) =
            parse_fields(" NvU32 a;\n rpc_generic_union rpc_message_data[];\n", 1, &d)
                .expect("parses");
        assert_eq!(fields.len(), 1);
        assert_eq!(
            flex,
            Some(("rpc_message_data".into(), "rpc_generic_union".into()))
        );
    }

    #[test]
    fn a_field_after_a_flexible_array_member_is_refused() {
        let d = BTreeMap::new();
        assert!(matches!(
            parse_fields(" NvU32 a[];\n NvU32 b;\n", 1, &d),
            Err(ParseError::BadDeclaration { .. })
        ));
    }

    #[test]
    fn a_pointer_declaration_is_refused() {
        let d = BTreeMap::new();
        assert!(matches!(
            parse_fields(" void *p;", 1, &d),
            Err(ParseError::BadDeclaration { .. })
        ));
    }

    #[test]
    fn scan_defines_reads_both_literal_spellings() {
        let d = scan_defines(
            "#define A (0x80U)\n#define B 100U\n#define C(x) (x)\n#define D \"str\"\n",
        );
        assert_eq!(d.get("A"), Some(&0x80));
        assert_eq!(d.get("B"), Some(&100));
        assert_eq!(d.get("C"), None, "function-like macros are not constants");
        assert_eq!(d.get("D"), None, "a string is not an integral constant");
    }

    #[test]
    fn scan_macro_list_reads_both_rpc_enum_forms() {
        let fns = scan_macro_list(
            "    X(RM, ALLOC_ROOT,     2)\n    X(RM, FREE, 10)\n#   define X(a,b,c)\n",
            "X",
            3,
        );
        assert_eq!(fns, vec![("ALLOC_ROOT".into(), 2), ("FREE".into(), 10)]);
        let evs = scan_macro_list(
            "    E(GSP_INIT_DONE,   0x1001)\n    E(POST_EVENT, 0x1003)\n",
            "E",
            2,
        );
        assert_eq!(
            evs,
            vec![
                ("GSP_INIT_DONE".into(), 0x1001),
                ("POST_EVENT".into(), 0x1003)
            ]
        );
    }

    /// The wrong arity is skipped, not mis-read: an `E(…)` line must never be
    /// scanned as an `X(…)`.
    #[test]
    fn scan_macro_list_ignores_the_wrong_arity() {
        assert!(scan_macro_list("    E(GSP_INIT_DONE, 0x1001)\n", "E", 3).is_empty());
        assert!(scan_macro_list("    X(RM, FREE, 10)\n", "X", 2).is_empty());
    }
}
