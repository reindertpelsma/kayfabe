//! Immutable same-die PRECOMP snapshot, obtained at realize, never on a vCPU.
use crate::capsderive::Die;
use crate::class::{ClassTable, Row};
use crate::regs::Regs;

const PREFIX: &str = "PRECOMP_WIN_PIPE_HDR_CAP";
const TYPES: [char; 6] = ['A', 'B', 'C', 'D', 'E', 'F'];

/// Bounded host facts, tied to the selected die, class and public layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPrecomp {
    die: Die,
    class: u32,
    windows: u32,
    layout: Vec<(String, Row)>,
    words: Vec<(u32, u32)>,
}

/// Form the plan from public host headers and read it once, without guest inputs.
///
/// # Errors
/// Missing public vocabulary, invalid dimensions or failed/incomplete host read.
pub fn query(
    table: &ClassTable,
    regs: &Regs,
    row: &kf_chip::display::DisplayRow,
    die: Die,
    mut read: impl FnMut(&[u32]) -> Result<Vec<u32>, String>,
) -> Result<HostPrecomp, String> {
    crate::precomp_fields::membership(row, die).map_err(|e| e.0)?;
    if row.windows == 0 || row.windows > 32 {
        return Err("PRECOMP host window count outside public bound".into());
    }
    let (end, base) = regs
        .f("NV_PDISP_FE_SW")
        .ok_or("PRECOMP host aperture is not public")?;
    let mut offsets = Vec::new();
    let mut addresses = Vec::new();
    for window in 0..row.windows {
        for kind in TYPES {
            let name = format!("{PREFIX}{kind}");
            if table
                .v(row.classes.caps, &format!("{name}__SIZE_1"))
                .is_none_or(|n| window >= n)
            {
                return Err(format!("PRECOMP host array bound missing: {name}"));
            }
            let offset = table
                .a(row.classes.caps, &name, window)
                .ok_or_else(|| format!("PRECOMP host register missing: {name}"))?;
            let address = base
                .checked_add(u64::from(offset))
                .filter(|a| a.checked_add(3).is_some_and(|last| last <= end))
                .and_then(|a| u32::try_from(a).ok())
                .ok_or("PRECOMP host address outside aperture")?;
            if addresses.contains(&address) || !address.is_multiple_of(4) {
                return Err("PRECOMP host addresses overlap or are unaligned".into());
            }
            offsets.push(offset);
            addresses.push(address);
        }
    }
    let values = read(&addresses)?;
    if values.len() != addresses.len()
        || values.iter().all(|v| *v == 0)
        || values.iter().all(|v| *v == u32::MAX)
    {
        return Err("PRECOMP host reply is incomplete, empty or inaccessible".into());
    }
    Ok(HostPrecomp {
        die,
        class: row.classes.caps,
        windows: row.windows,
        layout: table.rows_under(row.classes.caps, PREFIX),
        words: offsets.into_iter().zip(values).collect(),
    })
}

impl HostPrecomp {
    /// Bounded physical observations for the exact die/class/topology/layout.
    /// These facts are evidence, not permission to advertise a virtual feature.
    #[must_use]
    pub fn observations_for(
        &self,
        table: &ClassTable,
        row: &kf_chip::display::DisplayRow,
        die: Die,
    ) -> Option<&[(u32, u32)]> {
        self.words_for(table, row, die)
    }

    pub(crate) fn words_for(
        &self,
        table: &ClassTable,
        row: &kf_chip::display::DisplayRow,
        die: Die,
    ) -> Option<&[(u32, u32)]> {
        (self.die == die
            && self.class == row.classes.caps
            && self.windows == row.windows
            && self.layout == table.rows_under(row.classes.caps, PREFIX))
        .then_some(self.words.as_slice())
    }
}
