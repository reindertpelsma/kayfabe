//! ★ The virtual monitor's EDID — authored by kayfabe, never captured from a real panel.
//!
//! The guest's NVKMS learns what a connector can show only from an EDID (`V3_DISPLAY.md` §4.7). The
//! monitor behind our virtual connector is ours, so its EDID is too: a VESA E-EDID 1.4 base block
//! whose first detailed timing is the preferred mode (the size the operator or the QEMU window
//! asked for) and whose range-limits descriptor bounds what NVKMS may validate against it.
//!
//! Layout references: VESA E-EDID Release A, Revision 2 (the 1.4 base block); VESA CVT 1.2 for the
//! reduced-blanking timings a non-CEA size gets. Nothing here is NVIDIA-specific.

/// Bytes in the EDID base block.
pub const EDID_BLOCK: usize = 128;

/// One display timing, in pixels / lines and kHz of pixel clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// Active width.
    pub h_active: u32,
    /// Horizontal front porch.
    pub h_front: u32,
    /// Horizontal sync width.
    pub h_sync: u32,
    /// Horizontal back porch.
    pub h_back: u32,
    /// Active height.
    pub v_active: u32,
    /// Vertical front porch.
    pub v_front: u32,
    /// Vertical sync width.
    pub v_sync: u32,
    /// Vertical back porch.
    pub v_back: u32,
    /// Pixel clock in kHz.
    pub pixel_khz: u32,
    /// Horizontal sync polarity positive.
    pub h_pos: bool,
    /// Vertical sync polarity positive.
    pub v_pos: bool,
}

impl Timing {
    /// Total width (active + blanking).
    #[must_use]
    pub fn h_total(&self) -> u32 {
        self.h_active + self.h_front + self.h_sync + self.h_back
    }
    /// Total height (active + blanking).
    #[must_use]
    pub fn v_total(&self) -> u32 {
        self.v_active + self.v_front + self.v_sync + self.v_back
    }
    /// Refresh rate in millihertz (the rate a vblank timer for this mode runs at).
    #[must_use]
    pub fn refresh_mhz(&self) -> u64 {
        let tot = u64::from(self.h_total()) * u64::from(self.v_total());
        if tot == 0 {
            return 0;
        }
        u64::from(self.pixel_khz) * 1_000_000 / tot
    }

    /// CEA-861 VIC 16: 1920×1080 @ 60 Hz, 148.5 MHz — the default preferred mode.
    #[must_use]
    pub fn cea_1080p60() -> Timing {
        Timing {
            h_active: 1920,
            h_front: 88,
            h_sync: 44,
            h_back: 148,
            v_active: 1080,
            v_front: 4,
            v_sync: 5,
            v_back: 36,
            pixel_khz: 148_500,
            h_pos: true,
            v_pos: true,
        }
    }

    /// ★ VESA CVT 1.2 **reduced blanking (v1)** timing for `w`×`h` at `hz` — what a size that is
    /// not a CEA mode (a QEMU window) is presented as. Returns `None` for sizes outside what an
    /// EDID detailed timing can carry (12-bit active, 16-bit pixel clock in 10 kHz units).
    #[must_use]
    pub fn cvt_rb(w: u32, h: u32, hz: u32) -> Option<Timing> {
        if !(64..=4095).contains(&w) || !(64..=4095).contains(&h) || !(24..=120).contains(&hz) {
            return None;
        }
        // CVT RB v1 constants.
        const RB_H_BLANK: u32 = 160;
        const RB_H_SYNC: u32 = 32;
        const RB_MIN_V_BLANK_US: f64 = 460.0;
        const RB_V_FPORCH: u32 = 3;
        const MIN_V_BPORCH: u32 = 6;
        const CLOCK_STEP_KHZ: f64 = 250.0;
        let w = w & !7; // CVT character cell = 8
        let v_sync = match (w * 1000).checked_div(h)? {
            // aspect → vsync width (CVT table)
            1333 => 4,        // 4:3
            1777 | 1778 => 5, // 16:9
            1600 => 6,        // 16:10
            1250 => 7,        // 5:4
            _ => 10,
        };
        let h_period_est = ((1_000_000.0 / f64::from(hz)) - RB_MIN_V_BLANK_US) / f64::from(h);
        let vbi_lines = (RB_MIN_V_BLANK_US / h_period_est).floor() as u32 + 1;
        let v_blank = vbi_lines.max(RB_V_FPORCH + v_sync + MIN_V_BPORCH);
        let v_total = h + v_blank;
        let h_total = w + RB_H_BLANK;
        let pixel_khz =
            ((f64::from(hz) * f64::from(v_total) * f64::from(h_total) / 1000.0 / CLOCK_STEP_KHZ)
                .floor()
                * CLOCK_STEP_KHZ) as u32;
        if pixel_khz / 10 > 0xffff {
            return None;
        }
        Some(Timing {
            h_active: w,
            h_front: 48,
            h_sync: RB_H_SYNC,
            h_back: RB_H_BLANK - 48 - RB_H_SYNC,
            v_active: h,
            v_front: RB_V_FPORCH,
            v_sync,
            v_back: v_blank - RB_V_FPORCH - v_sync,
            pixel_khz,
            h_pos: true,
            v_pos: false,
        })
    }
}

/// Which digital interface the EDID declares (E-EDID 1.4 byte 20, bits 3:0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interface {
    /// DVI (TMDS, no audio, no InfoFrames).
    Dvi,
    /// HDMI-a (TMDS).
    HdmiA,
    /// DisplayPort.
    DisplayPort,
}

/// ★ What the virtual monitor is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Monitor {
    /// The preferred (first detailed) timing.
    pub preferred: Timing,
    /// The interface the EDID declares.
    pub interface: Interface,
    /// The monitor name descriptor (≤ 13 ASCII bytes are used).
    pub name: String,
    /// Serial number (distinguishes several heads' monitors).
    pub serial: u32,
    /// Upper bound on the pixel clock NVKMS may validate against this monitor, kHz.
    pub max_pixel_khz: u32,
    /// ★ `display-max-fps` (`crate::pace`): the cap, in Hz — the range-limits descriptor's maximum
    /// vertical rate (75 unset, D5: today's byte), and below 60 no 60 Hz mode is listed at all.
    pub cap_hz: u32,
}

impl Monitor {
    /// The default virtual monitor: 1920×1080@60 on DVI, "kayfabe".
    #[must_use]
    pub fn default_1080p() -> Monitor {
        Monitor {
            preferred: Timing::cea_1080p60(),
            interface: Interface::Dvi,
            name: "kayfabe".to_string(),
            serial: 1,
            max_pixel_khz: 165_000,
            cap_hz: crate::pace::UNSET_CAP_HZ,
        }
    }

    /// ★ The monitor for the `display-max-fps` property `cfg_hz` (0 unset): unset, exactly
    /// [`Monitor::default_1080p`] (D5 — the EDID is byte-identical to the one before the property);
    /// set, a 1920x1080 window at that rate ([`Monitor::for_window`]) whose EDID caps at it.
    #[must_use]
    pub fn configured(cfg_hz: u32) -> Monitor {
        if cfg_hz == 0 {
            Monitor::default_1080p()
        } else {
            Monitor::for_window(1920, 1080, 0, 165_000, cfg_hz)
        }
    }

    /// ★ Display step 3c: the monitor for a host window of `w` x `h` at `mhz` millihertz (0:
    /// unknown) behind a connector whose pixel clock is limited to `max_pixel_khz` (`PCLK_LIMIT`;
    /// DVI single-link is 165 000), under the `display-max-fps` property `cfg_hz` (0 unset). The
    /// size is clamped to 640..=3840 x 480..=2160; the refresh is [`crate::pace::preferred_hz`]
    /// (60 with neither, never above the cap); 1920x1080@60 is the CEA mode, anything else CVT
    /// reduced blanking.
    ///
    /// ★ ABOVE 60 Hz THE RATE YIELDS, NOT THE SIZE (2026-10-04, `display-max-fps`): the highest
    /// rate from the target down to 61 at which the requested size fits the clock is taken (1080p
    /// tops out at 71 Hz, 164 750 kHz). Only when none fits does today's rule run — at 60 Hz, a
    /// size whose clock exceeds the limit is scaled down at the SAME aspect ratio until it fits
    /// (the broker scales the rest). ⊘ Before, a 72–75 Hz host window became a 1800x1012 monitor.
    /// At 60 Hz and below the sizes are exactly the old rule's. The EDID carries 1920x1080@60 as
    /// its second mode when the cap allows 60.
    #[must_use]
    pub fn for_window(w: u32, h: u32, mhz: u32, max_pixel_khz: u32, cfg_hz: u32) -> Monitor {
        let target = crate::pace::preferred_hz(cfg_hz, mhz);
        let (w, h) = (w.clamp(640, 3840), h.clamp(480, 2160));
        let at = |w: u32, h: u32, hz: u32| {
            if (w & !7, h, hz) == (1920, 1080, 60) {
                Some(Timing::cea_1080p60())
            } else {
                Timing::cvt_rb(w, h, hz)
            }
        };
        let fast = (61..=target)
            .rev()
            .find_map(|hz| at(w, h, hz).filter(|t| t.pixel_khz <= max_pixel_khz));
        let timing = match fast {
            Some(t) => t,
            None => {
                let hz = target.min(60);
                let (mut w, mut h) = (w, h);
                loop {
                    match at(w, h, hz) {
                        Some(t) if t.pixel_khz <= max_pixel_khz || w <= 640 || h <= 480 => break t,
                        _ => {
                            // 15/16 per step keeps the aspect ratio within a pixel
                            w = (w * 15 / 16).max(640);
                            h = (h * 15 / 16).max(480);
                        }
                    }
                }
            }
        };
        Monitor {
            preferred: timing,
            interface: Interface::Dvi,
            name: "kayfabe".to_string(),
            serial: 1,
            max_pixel_khz,
            cap_hz: crate::pace::cap_hz(cfg_hz),
        }
    }

    /// ★ `display-max-fps` (§8.16): the FNV-1a 64 of [`Monitor::edid`] — logged at realize and at
    /// every re-author, so a box run can grade "the unset EDID is byte-identical" from the log alone
    /// (`0xc9dc_bb39_4c28_b1c7` is `default_1080p`'s, pinned by a test). `None` when no EDID can be
    /// authored.
    #[must_use]
    pub fn edid_fnv(&self) -> Option<u64> {
        self.edid().ok().map(|e| {
            e.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
                (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
            })
        })
    }

    /// ★ The EDID base block (128 bytes, checksummed).
    ///
    /// # Errors
    /// A preferred timing that a detailed timing descriptor cannot express.
    pub fn edid(&self) -> Result<[u8; EDID_BLOCK], String> {
        let mut e = [0u8; EDID_BLOCK];
        e[0..8].copy_from_slice(&[0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00]);
        // Manufacturer "KFB": 5-bit letters, 'A' = 1, big-endian word.
        let id = |c: u8| u16::from(c - b'A' + 1);
        let mfg = (id(b'K') << 10) | (id(b'F') << 5) | id(b'B');
        e[8..10].copy_from_slice(&mfg.to_be_bytes());
        e[10..12].copy_from_slice(&0x0001u16.to_le_bytes()); // product code
        e[12..16].copy_from_slice(&self.serial.to_le_bytes());
        e[16] = 39; // week
        e[17] = 36; // year: 2026 - 1990
        e[18] = 1; // EDID 1.4
        e[19] = 4;
        let iface = match self.interface {
            Interface::Dvi => 0x1,
            Interface::HdmiA => 0x2,
            Interface::DisplayPort => 0x5,
        };
        e[20] = 0x80 | (0b010 << 4) | iface; // digital, 8 bpc
        // physical size from a 96-DPI assumption (cm), clamped to a byte
        e[21] =
            u8::try_from((self.preferred.h_active * 254 / 960).clamp(1, 255)).unwrap_or(255) / 10;
        e[22] =
            u8::try_from((self.preferred.v_active * 254 / 960).clamp(1, 255)).unwrap_or(255) / 10;
        e[23] = 120; // gamma 2.2
        e[24] = 0x06; // RGB 4:4:4; sRGB default; preferred timing is native
        // sRGB chromaticity (E-EDID 1.4 §3.7, the standard sRGB encoding)
        e[25..35].copy_from_slice(&[0xee, 0x91, 0xa3, 0x54, 0x4c, 0x99, 0x26, 0x0f, 0x50, 0x54]);
        // ★ `display-max-fps` below 60: no 60 Hz mode is listed — NVKMS keeps an EDID-listed mode
        // even when the EDID's own range excludes it (`ogkm-580: nvkms-modepool.c:1472-1490`), so
        // the guest would believe it runs at 60 under a slower tick
        let sixty = self.cap_hz >= 60;
        if sixty {
            e[35] = 0x21; // established: 640x480@60, 800x600@60
            e[36] = 0x08; // 1024x768@60
        }
        e[37] = 0x00;
        // standard timings: 1280x1024@60, 1600x900@60, 1680x1050@60; rest unused
        let mut std_timings: [[u8; 2]; 8] = [[1, 1]; 8];
        if sixty {
            std_timings[..3].copy_from_slice(&[[0x81, 0x80], [0xa9, 0xc0], [0xb3, 0x00]]);
        }
        for (i, s) in std_timings.iter().enumerate() {
            e[38 + 2 * i..40 + 2 * i].copy_from_slice(s);
        }
        e[54..72].copy_from_slice(&dtd(&self.preferred)?);
        // range limits: 24..cap Hz vertical (75 unset), 15..160 kHz horizontal, max pixel clock
        // (10 MHz units)
        let max_10mhz = u8::try_from(self.max_pixel_khz.div_ceil(10_000)).unwrap_or(255);
        let max_v = u8::try_from(self.cap_hz.clamp(24, 75)).unwrap_or(75);
        e[72..90].copy_from_slice(&[
            0, 0, 0, 0xfd, 0, 24, max_v, 15, 160, max_10mhz, 0x00, 0x0a, 0x20, 0x20, 0x20, 0x20,
            0x20, 0x20,
        ]);
        let mut name = [0x20u8; 13];
        let n = self.name.as_bytes();
        let used = n.len().min(12);
        name[..used].copy_from_slice(&n[..used]);
        name[used] = 0x0a;
        e[90..95].copy_from_slice(&[0, 0, 0, 0xfc, 0]);
        e[95..108].copy_from_slice(&name);
        // ★ 3c: a preferred mode other than 1080p60 keeps 1080p60 as the second detailed timing (a
        // compositor can always fall back to it); otherwise the dummy descriptor
        let cea = Timing::cea_1080p60();
        if sixty && self.preferred != cea && cea.pixel_khz <= self.max_pixel_khz {
            e[108..126].copy_from_slice(&dtd(&cea)?);
        } else {
            e[108..126].copy_from_slice(&[0, 0, 0, 0x10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        }
        e[126] = 0; // no extension blocks
        let sum = e[..127].iter().fold(0u8, |a, b| a.wrapping_add(*b));
        e[127] = 0u8.wrapping_sub(sum);
        Ok(e)
    }
}

/// An 18-byte detailed timing descriptor.
fn dtd(t: &Timing) -> Result<[u8; 18], String> {
    let clk = t.pixel_khz / 10;
    let h_blank = t.h_front + t.h_sync + t.h_back;
    let v_blank = t.v_front + t.v_sync + t.v_back;
    let fits = clk > 0
        && clk <= 0xffff
        && t.h_active <= 0xfff
        && h_blank <= 0xfff
        && t.v_active <= 0xfff
        && v_blank <= 0xfff
        && t.h_front <= 0x3ff
        && t.h_sync <= 0x3ff
        && t.v_front <= 0x3f
        && t.v_sync <= 0x3f;
    if !fits {
        return Err(format!(
            "timing {t:?} does not fit a detailed timing descriptor"
        ));
    }
    let b = |v: u32| (v & 0xff) as u8;
    let mut d = [0u8; 18];
    d[0..2].copy_from_slice(&(clk as u16).to_le_bytes());
    d[2] = b(t.h_active);
    d[3] = b(h_blank);
    d[4] = (((t.h_active >> 8) & 0xf) << 4) as u8 | ((h_blank >> 8) & 0xf) as u8;
    d[5] = b(t.v_active);
    d[6] = b(v_blank);
    d[7] = (((t.v_active >> 8) & 0xf) << 4) as u8 | ((v_blank >> 8) & 0xf) as u8;
    d[8] = b(t.h_front);
    d[9] = b(t.h_sync);
    d[10] = (((t.v_front & 0xf) << 4) | (t.v_sync & 0xf)) as u8;
    d[11] = ((((t.h_front >> 8) & 3) << 6)
        | (((t.h_sync >> 8) & 3) << 4)
        | (((t.v_front >> 4) & 3) << 2)
        | ((t.v_sync >> 4) & 3)) as u8;
    // image size in mm (96 DPI)
    let hmm = t.h_active * 254 / 960;
    let vmm = t.v_active * 254 / 960;
    d[12] = b(hmm);
    d[13] = b(vmm);
    d[14] = (((hmm >> 8) & 0xf) << 4) as u8 | ((vmm >> 8) & 0xf) as u8;
    // digital separate sync, polarities
    d[17] = 0x18 | if t.v_pos { 0x04 } else { 0 } | if t.h_pos { 0x02 } else { 0 };
    Ok(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★ The default EDID is a valid 1.4 base block: header, checksum, the preferred timing decodes
    /// back to CEA 1080p60, and the refresh the vblank timer will use is 60 Hz.
    #[test]
    fn the_default_edid_is_valid_and_its_preferred_timing_round_trips() {
        let m = Monitor::default_1080p();
        let e = m.edid().expect("edid");
        assert_eq!(&e[0..8], &[0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00]);
        assert_eq!(e.iter().fold(0u8, |a, b| a.wrapping_add(*b)), 0, "checksum");
        assert_eq!((e[18], e[19]), (1, 4));
        assert_eq!(e[20] & 0x80, 0x80, "digital");
        // DTD 1 at 54: 148.5 MHz, 1920 + 280, 1080 + 45
        assert_eq!(u16::from_le_bytes([e[54], e[55]]), 14850);
        assert_eq!(u32::from(e[56]) | (u32::from(e[58] >> 4) << 8), 1920);
        assert_eq!(u32::from(e[57]) | (u32::from(e[58] & 0xf) << 8), 280);
        assert_eq!(u32::from(e[59]) | (u32::from(e[61] >> 4) << 8), 1080);
        assert_eq!(u32::from(e[60]) | (u32::from(e[61] & 0xf) << 8), 45);
        assert_eq!(m.preferred.refresh_mhz(), 60_000);
        assert_eq!(&e[95..102], b"kayfabe");
    }

    /// CVT-RB sizes for common window sizes are ~60 Hz, fit a descriptor, and are refused out of range.
    #[test]
    fn cvt_rb_timings_are_about_60hz_and_bounded() {
        for (w, h) in [
            (1280, 720),
            (1920, 1080),
            (2560, 1440),
            (1366, 768),
            (3840, 2160),
        ] {
            let t = Timing::cvt_rb(w, h, 60).expect("cvt");
            let r = t.refresh_mhz();
            assert!((59_000..=61_000).contains(&r), "{w}x{h}: {r} mHz");
            assert!(dtd(&t).is_ok(), "{w}x{h} fits a DTD");
            assert_eq!(t.h_active, w & !7);
        }
        assert!(Timing::cvt_rb(8192, 8192, 60).is_none());
        assert!(Timing::cvt_rb(1920, 1080, 500).is_none());
    }

    /// ★ 3c: a window becomes a monitor — clamped, CVT-RB (or CEA at 1080p60), fitted under the
    /// connector's 165 MHz at the same aspect ratio, with 1080p60 as the second mode and a valid
    /// checksum.
    #[test]
    fn a_window_becomes_a_monitor_fitted_under_the_connector() {
        let m = Monitor::for_window(1600, 900, 0, 165_000, 0);
        assert_eq!((m.preferred.h_active, m.preferred.v_active), (1600, 900));
        assert!((59_000..=61_000).contains(&m.preferred.refresh_mhz()));
        let e = m.edid().unwrap();
        assert_eq!(e.iter().fold(0u8, |a, b| a.wrapping_add(*b)), 0, "checksum");
        assert_eq!(
            u16::from_le_bytes([e[108], e[109]]),
            14850,
            "the second DTD is CEA 1080p60"
        );
        assert_eq!(
            Monitor::for_window(1920, 1080, 60_000, 165_000, 0).preferred,
            Timing::cea_1080p60()
        );
        let d = Monitor::for_window(1920, 1080, 60_000, 165_000, 0)
            .edid()
            .unwrap();
        assert_eq!(d[111], 0x10, "1080p60 preferred: no duplicate second DTD");
        // too small and too large are clamped; the refresh is clamped to the EDID's range
        let s = Monitor::for_window(100, 100, 240_000, 165_000, 0);
        assert_eq!((s.preferred.h_active, s.preferred.v_active), (640, 480));
        assert!(s.preferred.refresh_mhz() <= 76_000);
        // 2560x1440 does not fit 165 MHz: scaled down at 16:9 until it does
        let b = Monitor::for_window(2560, 1440, 60_000, 165_000, 0);
        assert!(b.preferred.pixel_khz <= 165_000, "{:?}", b.preferred);
        let (w, h) = (b.preferred.h_active, b.preferred.v_active);
        assert!(w < 2560 && h < 1440);
        let aspect = f64::from(w) / f64::from(h);
        assert!((aspect - 16.0 / 9.0).abs() < 0.02, "{w}x{h}");
        assert!(b.edid().is_ok());
    }

    /// FNV-1a 64 of an EDID (the fingerprint the golden table below pins).
    fn fnv(e: &[u8]) -> u64 {
        e.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
        })
    }

    /// ★ D5: with `display-max-fps` unset the EDID is BYTE-IDENTICAL to the one authored before the
    /// property existed — the golden array was printed by `Monitor::default_1080p().edid()` at
    /// `82f98f42` (v3-broker), the commit `v3-maxfps` was cut from — and so is every 3c window at 60
    /// Hz and below. (Mutations: a range maximum written as the preferred rate or 60, the 60 Hz
    /// extras gated on the preferred mode instead of the cap.)
    #[test]
    fn the_unset_edid_is_byte_identical() {
        const GOLDEN: [u8; EDID_BLOCK] = [
            0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00, 0x2c, 0xc2, 0x01, 0x00, 0x01, 0x00,
            0x00, 0x00, 0x27, 0x24, 0x01, 0x04, 0xa1, 0x19, 0x19, 0x78, 0x06, 0xee, 0x91, 0xa3,
            0x54, 0x4c, 0x99, 0x26, 0x0f, 0x50, 0x54, 0x21, 0x08, 0x00, 0x81, 0x80, 0xa9, 0xc0,
            0xb3, 0x00, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x02, 0x3a,
            0x80, 0x18, 0x71, 0x38, 0x2d, 0x40, 0x58, 0x2c, 0x45, 0x00, 0xfc, 0x1d, 0x11, 0x00,
            0x00, 0x1e, 0x00, 0x00, 0x00, 0xfd, 0x00, 0x18, 0x4b, 0x0f, 0xa0, 0x11, 0x00, 0x0a,
            0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x00, 0x00, 0x00, 0xfc, 0x00, 0x6b, 0x61, 0x79,
            0x66, 0x61, 0x62, 0x65, 0x0a, 0x20, 0x20, 0x20, 0x20, 0x20, 0x00, 0x00, 0x00, 0x10,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x83,
        ];
        assert_eq!(Monitor::default_1080p().edid().unwrap(), GOLDEN);
        assert_eq!(fnv(&GOLDEN), 0xc9dc_bb39_4c28_b1c7);
        assert_eq!(
            Monitor::default_1080p().edid_fnv(),
            Some(fnv(&GOLDEN)),
            "the logged value"
        );
        assert_eq!(Monitor::configured(0).edid().unwrap(), GOLDEN);
        assert_eq!(Monitor::configured(0), Monitor::default_1080p());
        // (w, h, mHz, FNV-1a 64 of the EDID `Monitor::for_window` authored at 82f98f42)
        for (w, h, mhz, old) in [
            (1600, 900, 0, 0x8c18_c535_f16f_a497_u64),
            (1920, 1080, 60000, 0xc9dc_bb39_4c28_b1c7),
            (1920, 1080, 50000, 0x2768_ab22_3ba3_c9e1),
            (2560, 1440, 60000, 0x4791_bf0b_dcef_38c3),
            (2560, 1440, 30000, 0xcbd7_3c1e_83e7_7bb3),
            (1024, 695, 60000, 0x1499_385d_0eff_6d99),
            (1366, 768, 59940, 0x843f_aee7_a215_aed3),
            (3840, 2160, 24000, 0xab6c_aa01_3781_babd),
            (800, 600, 48000, 0x649a_cfbc_73d0_7131),
        ] {
            let e = Monitor::for_window(w, h, mhz, 165_000, 0).edid().unwrap();
            assert_eq!(fnv(&e), old, "{w}x{h} at {mhz} mHz");
        }
    }

    /// ★ Below a 60 Hz cap NO 60 Hz mode is listed — no established timing (bytes 35-37), no
    /// standard timing, no second CEA DTD — and the range maximum (byte 78) is the cap; the
    /// preferred mode runs at the cap. (Mutation: keeping the extras lets NVKMS validate a 60 Hz
    /// mode the tick then runs at 30.)
    #[test]
    fn a_cap_below_60_lists_no_60hz_mode() {
        let m = Monitor::configured(30);
        let e = m.edid().unwrap();
        assert_eq!(&e[35..38], &[0, 0, 0]);
        assert!(e[38..54].chunks(2).all(|c| c == [1, 1]), "{:?}", &e[38..54]);
        assert_eq!(&e[108..112], &[0, 0, 0, 0x10], "the dummy descriptor");
        assert_eq!(e[77], 24);
        assert_eq!(e[78], 30, "the range maximum is the cap");
        assert_eq!(e.iter().fold(0u8, |a, b| a.wrapping_add(*b)), 0, "checksum");
        assert_eq!((m.preferred.h_active, m.preferred.v_active), (1920, 1080));
        assert_eq!(m.preferred.pixel_khz, 68_250);
        assert_eq!(m.preferred.refresh_mhz(), 29_938);
        // the period the engine will derive from it (F5: logged in ns, 33 401 904)
        let ns = u64::from(m.preferred.h_total()) * u64::from(m.preferred.v_total()) * 1_000_000
            / u64::from(m.preferred.pixel_khz);
        assert_eq!(ns, 33_401_904);
        // at a 60 Hz cap they are all there, and the preferred mode is CEA 1080p60
        let s = Monitor::configured(60);
        let e = s.edid().unwrap();
        assert_eq!(s.preferred, Timing::cea_1080p60());
        assert_eq!(&e[35..37], &[0x21, 0x08]);
        assert_eq!(e[78], 60);
        let mut g = Monitor::default_1080p().edid().unwrap();
        g[78] = 60;
        g[127] = g[127].wrapping_add(15);
        assert_eq!(
            e, g,
            "only the range maximum and the checksum differ from the default"
        );
        assert_eq!(Monitor::configured(50).preferred.pixel_khz, 115_000);
    }

    /// ★ Above 60 Hz the RATE yields, never the size: 1920x1080 asked at 75 Hz is 71 Hz at
    /// 164 750 kHz (72 would be 167 250); 2560x1440 at 75 Hz fits no rate above 60, so it shrinks
    /// at 60 Hz exactly as before. (Mutation: the old loop gives 1800x1012 at 75 Hz.)
    #[test]
    fn above_60hz_the_rate_yields_not_the_size() {
        assert_eq!(Timing::cvt_rb(1920, 1080, 71).unwrap().pixel_khz, 164_750);
        assert_eq!(Timing::cvt_rb(1920, 1080, 72).unwrap().pixel_khz, 167_250);
        for (mhz, cfg) in [(75_000, 0), (144_000, 0), (0, 75), (120_000, 72)] {
            let m = Monitor::for_window(1920, 1080, mhz, 165_000, cfg);
            assert_eq!(
                (m.preferred.h_active, m.preferred.v_active),
                (1920, 1080),
                "{mhz} {cfg}"
            );
            assert_eq!(m.preferred.pixel_khz, 164_750, "{mhz} {cfg}");
            assert!((70_000..72_000).contains(&m.preferred.refresh_mhz()));
        }
        let b = Monitor::for_window(2560, 1440, 75_000, 165_000, 0);
        assert_eq!(b, Monitor::for_window(2560, 1440, 60_000, 165_000, 0));
        assert!(b.preferred.h_active < 2560);
        // a small window keeps its rate
        let s = Monitor::for_window(1280, 720, 75_000, 165_000, 0);
        assert_eq!((s.preferred.h_active, s.preferred.v_active), (1280, 720));
        assert!((74_000..76_000).contains(&s.preferred.refresh_mhz()));
    }

    /// ★ Every EDID kayfabe authors honours its cap: swept over every property value and host rate
    /// at a few sizes — the preferred mode runs at most at the cap, under the pixel clock, every DTD's
    /// refresh is at most the cap (the 1.01 tolerance NVKMS applies is not leaned on), the range
    /// maximum IS the cap and the checksum holds.
    #[test]
    fn every_authored_edid_honours_its_cap() {
        let dtd_mhz = |d: &[u8]| -> Option<u64> {
            let clk = u64::from(u16::from_le_bytes([d[0], d[1]])) * 10;
            if clk == 0 {
                return None;
            }
            let ha = u64::from(d[2]) | (u64::from(d[4] >> 4) << 8);
            let hb = u64::from(d[3]) | (u64::from(d[4] & 0xf) << 8);
            let va = u64::from(d[5]) | (u64::from(d[7] >> 4) << 8);
            let vb = u64::from(d[6]) | (u64::from(d[7] & 0xf) << 8);
            Some(clk * 1_000_000 / ((ha + hb) * (va + vb)))
        };
        let mut cfgs = vec![0];
        cfgs.extend(24..=75);
        for cfg in cfgs {
            let cap = crate::pace::cap_hz(cfg);
            for host in [
                0u32, 24_000, 30_000, 50_000, 59_940, 60_000, 75_000, 120_000, 240_000,
            ] {
                for (w, h) in [(1920, 1080), (1280, 720), (2560, 1440), (640, 480)] {
                    let m = Monitor::for_window(w, h, host, 165_000, cfg);
                    let e = m.edid().unwrap();
                    let what = format!("cfg {cfg} host {host} {w}x{h}");
                    assert_eq!(e.iter().fold(0u8, |a, b| a.wrapping_add(*b)), 0, "{what}");
                    assert_eq!(u32::from(e[78]), cap, "{what}");
                    assert!(m.preferred.pixel_khz <= 165_000, "{what}");
                    // ⊘ only the HIGH end is the cap's: CVT's 250 kHz clock step puts a small mode
                    // asked at 24 Hz below 24 (640x480 at 23.45, as before the property), and
                    // NVKMS keeps an EDID-listed mode against its own range anyway
                    // (`ogkm-580: nvkms-modepool.c:1472-1490`)
                    let r = m.preferred.refresh_mhz();
                    assert!(r > 23_000 && r <= u64::from(cap) * 1000, "{what}: {r}");
                    for at in [54, 108] {
                        if let Some(mhz) = dtd_mhz(&e[at..at + 18]) {
                            assert!(
                                mhz <= u64::from(cap) * 1000,
                                "{what}: DTD at {at} {mhz} mHz"
                            );
                        }
                    }
                    if cap < 60 {
                        assert_eq!(&e[35..38], &[0, 0, 0], "{what}");
                    }
                }
            }
        }
    }
}
