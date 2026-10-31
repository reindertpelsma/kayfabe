//! Native-versus-guest mode.
//!
//! The client cannot reliably tell where it runs (a nested Vast box is a guest that looks native), so
//! the harness says so with `--mode`. An unset flag is [`Mode::Unknown`], printed as such, and the
//! ratio computation refuses a log whose mode is unknown (a mislabelled run would silently poison a
//! calibration).

/// Where this process runs, as the harness declared it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// On the machine that owns the GPU (the "bare metal" lane of `bare_metal_suite.sh`).
    Native,
    /// Inside the kayfabe guest, over the emulated device.
    Guest,
    /// Not declared.
    Unknown,
}

impl Mode {
    /// Parse a `--mode` value. `bare` and `native` are the same mode (the suites say bare, the design
    /// says native); anything else is refused.
    ///
    /// # Errors
    /// The refused text.
    pub fn parse(s: &str) -> Result<Mode, String> {
        match s {
            "bare" | "native" => Ok(Mode::Native),
            "guest" => Ok(Mode::Guest),
            other => Err(format!("mode must be bare|native|guest, got {other:?}")),
        }
    }

    /// The word printed in the cell line (`bare`, `guest`, `unknown`).
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Mode::Native => "bare",
            Mode::Guest => "guest",
            Mode::Unknown => "unknown",
        }
    }

    /// Inverse of [`Mode::word`] for log parsing; unknown text is `None`.
    #[must_use]
    pub fn from_word(s: &str) -> Option<Mode> {
        match s {
            "bare" => Some(Mode::Native),
            "guest" => Some(Mode::Guest),
            "unknown" => Some(Mode::Unknown),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_words_round_trip() {
        assert_eq!(Mode::parse("native"), Ok(Mode::Native));
        assert_eq!(Mode::parse("bare"), Ok(Mode::Native));
        assert_eq!(Mode::parse("guest"), Ok(Mode::Guest));
        for m in [Mode::Native, Mode::Guest, Mode::Unknown] {
            assert_eq!(Mode::from_word(m.word()), Some(m));
        }
        assert!(Mode::parse("Guest").is_err());
        assert!(Mode::parse("").is_err());
        assert!(
            Mode::parse("unknown").is_err(),
            "unknown is a default, not a choice"
        );
        assert_eq!(Mode::from_word("host"), None);
    }
}
