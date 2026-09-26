//! ★ `NV0080_CTRL_CMD_MSENC_GET_CAPS_V2` (`0x801b02`) / `NV0080_CTRL_CMD_BSP_GET_CAPS_V2`
//! (`0x801c02`) — the video engines' capability tables, answered from the HOST.
//!
//! Both are `NON_PRIVILEGED | ROUTE_TO_PHYSICAL` (flags `0x50148`, `ogkm-580:
//! g_device_nvoc.c:933-951`), so a GSP-client guest sends them to us and a GSP answers them. The
//! tables are properties of the silicon + firmware, so the device asks the host the same question
//! at realize (unprivileged, on OUR device object) and serves the host's table: nothing
//! hand-written, per die. `[measured vvid 2026-09-26, GA106/580.159.04]` bare-metal
//! `libnvidia-encode` asks `0x801c02` (`capsTbl = 01 00 00 00 16 00 00 00`, instance 0) and the
//! legacy `0x801b01` before allocating its encoder.
//!
//! Layouts (`ogkm-580: ctrl0080msenc.h:85-88`, `ctrl0080bsp.h:107-110`):
//! `{NvU8 capsTbl[5]; NvU32 instanceId /* ignored */}` = 12 bytes and
//! `{NvU8 capsTbl[8]; NvU32 instanceId}` = 12 bytes.

/// `NV0080_CTRL_CMD_MSENC_GET_CAPS_V2`.
pub const MSENC_GET_CAPS_V2: u32 = 0x0080_1b02;
/// `NV0080_CTRL_CMD_BSP_GET_CAPS_V2`.
pub const BSP_GET_CAPS_V2: u32 = 0x0080_1c02;
/// `NV0080_CTRL_MSENC_CAPS_TBL_SIZE`.
pub const MSENC_CAPS_TBL_SIZE: usize = 5;
/// `NV0080_CTRL_BSP_CAPS_TBL_SIZE`.
pub const BSP_CAPS_TBL_SIZE: usize = 8;
/// Both params structs are 12 bytes (`instanceId` at offset 8).
pub const PARAMS_SIZE: usize = 12;
/// Offset of `instanceId`.
pub const INSTANCE_OFF: usize = 8;

/// One host answer: the control, the instance asked, the caps table bytes the host returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapsAnswer {
    /// [`MSENC_GET_CAPS_V2`] or [`BSP_GET_CAPS_V2`].
    pub cmd: u32,
    /// `instanceId` asked.
    pub instance: u32,
    /// `capsTbl`, as long as the control's table.
    pub caps: Vec<u8>,
}

/// The table length of `cmd`, or `None` for another control.
#[must_use]
pub fn caps_len(cmd: u32) -> Option<usize> {
    match cmd {
        MSENC_GET_CAPS_V2 => Some(MSENC_CAPS_TBL_SIZE),
        BSP_GET_CAPS_V2 => Some(BSP_CAPS_TBL_SIZE),
        _ => None,
    }
}

/// The request this port authors: a zero table and `instance`.
#[must_use]
pub fn host_request(instance: u32) -> [u8; PARAMS_SIZE] {
    let mut p = [0u8; PARAMS_SIZE];
    p[INSTANCE_OFF..].copy_from_slice(&instance.to_le_bytes());
    p
}

/// Why a guest request was not answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapsRefusal {
    /// Not one of the two controls.
    NotACapsControl(u32),
    /// Not the 12-byte struct.
    Size(usize),
    /// No host answer for this instance (an engine the device does not advertise).
    Instance(u32),
}

/// ★ Answer a guest's caps request in place: the host's table, the guest's own `instanceId`.
/// ⊘ `MSENC`'s `instanceId` is documented *ignored*, so any instance takes the instance-0 answer.
///
/// # Errors
/// [`CapsRefusal`].
pub fn answer(answers: &[CapsAnswer], cmd: u32, params: &mut [u8]) -> Result<(), CapsRefusal> {
    let n = caps_len(cmd).ok_or(CapsRefusal::NotACapsControl(cmd))?;
    if params.len() != PARAMS_SIZE {
        return Err(CapsRefusal::Size(params.len()));
    }
    let inst = u32::from_le_bytes([params[8], params[9], params[10], params[11]]);
    let want = if cmd == MSENC_GET_CAPS_V2 { 0 } else { inst };
    let a = answers.iter().find(|a| a.cmd == cmd && a.instance == want).ok_or(CapsRefusal::Instance(inst))?;
    params[..n].copy_from_slice(&a.caps[..n]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_measured_bsp_table_is_served_for_its_instance_only() {
        let a = CapsAnswer { cmd: BSP_GET_CAPS_V2, instance: 0, caps: vec![1, 0, 0, 0, 0x16, 0, 0, 0] };
        let mut p = host_request(0).to_vec();
        answer(std::slice::from_ref(&a), BSP_GET_CAPS_V2, &mut p).expect("instance 0");
        assert_eq!(p, [1, 0, 0, 0, 0x16, 0, 0, 0, 0, 0, 0, 0]);
        let mut p1 = host_request(1).to_vec();
        assert_eq!(answer(std::slice::from_ref(&a), BSP_GET_CAPS_V2, &mut p1), Err(CapsRefusal::Instance(1)));
        assert_eq!(answer(&[a], 0x2080_0000, &mut p1), Err(CapsRefusal::NotACapsControl(0x2080_0000)));
    }

    #[test]
    fn msenc_ignores_the_instance_and_keeps_the_guests_padding() {
        let a = CapsAnswer { cmd: MSENC_GET_CAPS_V2, instance: 0, caps: vec![9, 8, 7, 6, 5] };
        let mut p = host_request(3).to_vec();
        p[5] = 0xee;
        answer(&[a], MSENC_GET_CAPS_V2, &mut p).expect("ignored instance");
        assert_eq!(p, [9, 8, 7, 6, 5, 0xee, 0, 0, 3, 0, 0, 0]);
    }
}
