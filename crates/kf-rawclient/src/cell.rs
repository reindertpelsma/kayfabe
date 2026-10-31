//! The `RAWCLIENT_CELL` identity line.
//!
//! ```text
//! RAWCLIENT_CELL die=<name|arch:impl> family=<Family|unknown> mmu=<Ver2|Ver3|unknown> drv=<tag> kmod=<open|proprietary|unknown> mode=<bare|guest|unknown> rev=<sha|unknown>
//! ```
//!
//! Every field is the host's own answer or the harness's declaration: the die from `MC_GET_ARCH_INFO`
//! through `kf_chip::DieId`, the family and GMMU format through the `kf-chip` rows, the driver from the
//! RM session. Nothing is read from `nvidia-smi` and no die is listed here. The revision is declared by
//! the harness in `KF_REV` (the suites already hold it as `REV`); a missing one prints `unknown` and
//! the compare step refuses to compare such logs ("a bench claim carries its source revision").

use crate::caps::HostFacts;
use crate::mode::Mode;
use crate::outcome::token;

/// The identity of a run: the facts plus the harness's declarations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    /// Die name, or `arch:impl` in hex when ogkm names none.
    pub die: String,
    /// Family name or `unknown`.
    pub family: String,
    /// GMMU format or `unknown`.
    pub mmu: String,
    /// Driver tag.
    pub drv: String,
    /// Kernel module flavour.
    pub kmod: String,
    /// Declared mode.
    pub mode: Mode,
    /// Source revision.
    pub rev: String,
}

impl Cell {
    /// Build the cell from host facts, the declared mode and the declared revision.
    #[must_use]
    pub fn from_facts(facts: &HostFacts, mode: Mode, rev: Option<&str>) -> Cell {
        let die = facts.die();
        Cell {
            die: die.name().map_or_else(
                || format!("{:#x}:{:#x}", die.architecture, die.implementation),
                str::to_owned,
            ),
            family: facts
                .family()
                .map_or_else(|| "unknown".to_owned(), |f| format!("{f:?}")),
            mmu: facts
                .mmu()
                .map_or_else(|| "unknown".to_owned(), |m| format!("{m:?}")),
            drv: token(&facts.driver),
            kmod: token(&facts.kmod),
            mode,
            rev: rev.map_or_else(|| "unknown".to_owned(), token),
        }
    }

    /// The printed line.
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "RAWCLIENT_CELL die={} family={} mmu={} drv={} kmod={} mode={} rev={}",
            self.die,
            self.family,
            self.mmu,
            self.drv,
            self.kmod,
            self.mode.word(),
            self.rev
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caps::test_facts;

    #[test]
    fn the_die_comes_from_the_host_words_through_kf_chip() {
        let c = Cell::from_facts(&test_facts(), Mode::Guest, Some("abc123"));
        assert_eq!(
            c.line(),
            "RAWCLIENT_CELL die=GA106 family=Ampere mmu=Ver2 drv=580.159.04 kmod=open mode=guest rev=abc123"
        );
    }

    #[test]
    fn an_unnamed_die_and_unknown_family_print_what_was_answered() {
        let f = HostFacts {
            architecture: 0x1234,
            implementation: 0x9,
            ..test_facts()
        };
        let c = Cell::from_facts(&f, Mode::Unknown, None);
        assert_eq!(
            c.line(),
            "RAWCLIENT_CELL die=0x1234:0x9 family=unknown mmu=unknown drv=580.159.04 kmod=open mode=unknown rev=unknown"
        );
        // A known architecture with an implementation ogkm does not name: family known, die unnamed.
        let f = HostFacts {
            implementation: 0x1f,
            ..test_facts()
        };
        let c = Cell::from_facts(&f, Mode::Native, Some("r"));
        assert!(
            c.line().contains("die=0x170:0x1f family=Ampere mmu=Ver2 "),
            "{}",
            c.line()
        );
        assert!(c.line().contains("mode=bare"));
    }

    #[test]
    fn hostile_text_cannot_split_the_line() {
        let f = HostFacts {
            driver: "580 x=y\nRUNG_a=PASS".into(),
            kmod: "a b".into(),
            ..test_facts()
        };
        let c = Cell::from_facts(&f, Mode::Native, Some("sha with space"));
        let l = c.line();
        assert!(!l.contains('\n'));
        assert_eq!(l.split(' ').count(), 8, "{l}");
    }
}
