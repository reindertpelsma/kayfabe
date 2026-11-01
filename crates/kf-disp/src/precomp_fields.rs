//! Field-level policy for the implemented virtual pipeline. Physical observations
//! never widen its contract. No residual host-backed field is approved today.
use crate::caps::Missing;
use crate::capsderive::Die;
use crate::class::{ClassTable, Row, get, put};
use std::collections::BTreeMap;

pub(crate) fn membership(row: &kf_chip::display::DisplayRow, die: Die) -> Result<(), Missing> {
    let own = kf_chip::display::display_for(die.architecture, die.implementation)
        .ok_or_else(|| Missing("PRECOMP: no public die/display membership".into()))?;
    if own.ip_version != row.ip_version || own.classes != row.classes {
        return Err(Missing(
            "PRECOMP: class/IP does not belong to this die".into(),
        ));
    }
    Ok(())
}

fn authored(register: &str, field: &str) -> bool {
    match register {
        "CAPA" => matches!(
            field,
            "CSC00_PRESENT"
                | "CSC01_PRESENT"
                | "CSC0LUT_PRESENT"
                | "CSC10_PRESENT"
                | "CSC11_PRESENT"
                | "CSC1LUT_PRESENT"
                | "SCLR_PRESENT"
                | "TMO_PRESENT"
        ),
        "CAPB" => matches!(
            field,
            "ILUT_LOGSZ" | "ILUT_LOGNR" | "ILUT_DIRECT" | "ILUT_SFCLOAD"
        ),
        // These two existing declarations are INFERRED virtual precision rules.
        // They are not physical generation constants or a sufficient Windows fix.
        "CAPC" => matches!(field, "CSC00_PRECISION" | "CSC01_PRECISION"),
        "CAPD" => matches!(field, "TMO_LOGSZ" | "TMO_LOGNR" | "TMO_SFCLOAD"),
        _ => false,
    }
}

pub(crate) fn apply(
    table: &ClassTable,
    row: &kf_chip::display::DisplayRow,
    die: Die,
    words: &mut BTreeMap<u32, u32>,
) -> Result<(), Missing> {
    membership(row, die)?;
    if row.windows == 0 || row.windows > 32 {
        return Err(Missing(
            "PRECOMP: virtual window count outside public bound".into(),
        ));
    }
    for kind in ['A', 'B', 'C', 'D', 'E', 'F'] {
        let register = format!("CAP{kind}");
        let full = format!("PRECOMP_WIN_PIPE_HDR_{register}");
        let prefix = format!("{full}_");
        let bound = table
            .v(row.classes.caps, &format!("{full}__SIZE_1"))
            .filter(|n| row.windows <= *n && *n <= 32)
            .ok_or_else(|| Missing(format!("PRECOMP public array bound: {full}")))?;
        let fields = table.rows_under(row.classes.caps, &prefix);
        for window in 0..bound {
            let offset = table
                .a(row.classes.caps, &full, window)
                .filter(|o| o.is_multiple_of(4) && *o < crate::caps::PAGE as u32)
                .ok_or_else(|| Missing(format!("PRECOMP public address: {full}")))?;
            let prior = words.get(&offset).copied().unwrap_or_default();
            let mut value = 0;
            for (name, layout) in &fields {
                let Row::F(hi, lo) = *layout else {
                    continue;
                };
                if hi >= 32 || lo > hi {
                    return Err(Missing(format!("PRECOMP public field bound: {name}")));
                }
                let field = name
                    .strip_prefix(&prefix)
                    .ok_or_else(|| Missing(name.clone()))?;
                // Unresolved, unsupported and unassigned bits are unclaimed.
                // Keep the virtual authored value only after named-field policy.
                if window < row.windows && authored(&register, field) {
                    value = put(value, (hi, lo), get(prior, (hi, lo)));
                }
            }
            words.insert(offset, value);
        }
    }
    Ok(())
}
