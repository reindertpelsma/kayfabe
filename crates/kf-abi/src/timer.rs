//! Native timer view: compiled host/guest ABI cells, never a captured die offset.
use crate::{DriverVersion, generated::matrix as m};

/// Unprivileged control querying the host's timer register offset.
pub const REGISTER_OFFSET: u32 =
    m::CTRL_CMDS_NV2080_CTRL_CMD_TIMER_GET_REGISTER_OFFSET.everywhere_u32();

/// The SDK register view, independently compiled at each driver tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimerLayout {
    /// Byte length of the source-defined register view.
    pub register_bytes: u64,
    /// Offset of the low 32-bit timestamp word within the view.
    pub time_low: u64,
    /// Offset of the high 32-bit timestamp word within the view.
    pub time_high: u64,
}

/// Only a single ordinary 4 KiB register page with two naturally aligned 32-bit words is
/// supported. A new or absent layout is refused, never borrowed from a nearby driver.
pub fn layout(version: DriverVersion) -> Option<TimerLayout> {
    let l = m::NV01TIMERMAP.at(version).ok()??;
    let low = l.field("PTimerTime0")?;
    let high = l.field("PTimerTime1")?;
    if l.size == 0
        || l.size > 4096
        || [low, high].iter().any(|f| {
            f.size != 4
                || f.elem != 0
                || f.off % 4 != 0
                || f.off.checked_add(4).is_none_or(|end| end > l.size)
        })
        || low.off == high.off
    {
        return None;
    }
    Some(TimerLayout {
        register_bytes: u64::from(l.size),
        time_low: u64::from(low.off),
        time_high: u64::from(high.off),
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn timer_layout_is_measured_at_every_tag_and_unmeasured_refuses() {
        for &version in crate::generated::matrix::MEASURED {
            let l = super::layout(version).unwrap();
            assert_eq!(
                (l.register_bytes, l.time_low, l.time_high),
                (0x414, 0x400, 0x410)
            );
        }
        assert!(
            super::layout(crate::DriverVersion {
                major: 580,
                minor: 65,
                patch: 7
            })
            .is_none()
        );
    }
}
