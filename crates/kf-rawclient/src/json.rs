//! A tiny JSON object writer for the one-line machine-readable result. No dependency, no parser: the
//! compare step reads the `key=value` lines, which are the stable interface; the JSON is a convenience
//! copy for a tool that wants one record.

/// Builds `{"k":v,...}`. Keys are trusted constants; string values are escaped.
#[derive(Debug)]
pub struct Obj {
    buf: String,
    first: bool,
}

impl Default for Obj {
    fn default() -> Self {
        Self::new()
    }
}

impl Obj {
    /// An empty object.
    #[must_use]
    pub fn new() -> Self {
        Obj {
            buf: String::from("{"),
            first: true,
        }
    }

    fn key(&mut self, k: &str) {
        if !self.first {
            self.buf.push(',');
        }
        self.first = false;
        self.buf.push('"');
        self.buf.push_str(k);
        self.buf.push_str("\":");
    }

    /// A string member.
    pub fn str(&mut self, k: &str, v: &str) -> &mut Self {
        self.key(k);
        self.buf.push('"');
        for c in v.chars() {
            match c {
                '"' => self.buf.push_str("\\\""),
                '\\' => self.buf.push_str("\\\\"),
                c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                    self.buf.push_str(&format!("\\u{:04x}", c as u32));
                }
                c => self.buf.push(c),
            }
        }
        self.buf.push('"');
        self
    }

    /// An unsigned integer member.
    pub fn u64(&mut self, k: &str, v: u64) -> &mut Self {
        self.key(k);
        self.buf.push_str(&v.to_string());
        self
    }

    /// A boolean member.
    pub fn bool(&mut self, k: &str, v: bool) -> &mut Self {
        self.key(k);
        self.buf.push_str(if v { "true" } else { "false" });
        self
    }

    /// A float member; `null` for NaN or infinity (JSON has neither).
    pub fn f64(&mut self, k: &str, v: f64) -> &mut Self {
        self.key(k);
        if v.is_finite() {
            self.buf.push_str(&format!("{v:.6}"));
        } else {
            self.buf.push_str("null");
        }
        self
    }

    /// A member whose value is already valid JSON (a nested [`Obj::finish`] or an array).
    pub fn raw(&mut self, k: &str, json: &str) -> &mut Self {
        self.key(k);
        self.buf.push_str(json);
        self
    }

    /// The finished object text.
    #[must_use]
    pub fn finish(&self) -> String {
        format!("{}}}", self.buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_valid_json_with_escapes() {
        let mut o = Obj::new();
        o.str("a", "x\"y\\z\n\u{1}")
            .u64("n", 7)
            .bool("ok", true)
            .f64("r", 1.5)
            .f64("bad", f64::NAN)
            .raw("sub", "[1,2]");
        assert_eq!(
            o.finish(),
            "{\"a\":\"x\\\"y\\\\z\\u000a\\u0001\",\"n\":7,\"ok\":true,\"r\":1.500000,\"bad\":null,\"sub\":[1,2]}"
        );
        assert_eq!(Obj::new().finish(), "{}");
    }
}
