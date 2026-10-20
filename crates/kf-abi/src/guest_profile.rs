//! Per-device compatibility selection. No environment or process-wide state.
//! Guest identity still controls admission; this profile never supplies an ownership witness.

/// The explicit QEMU guest OS property; absence preserves the Linux configuration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GuestProfile {
    /// Existing Linux behavior.
    #[default]
    Linux,
    /// The approved Windows compatibility behavior, using derived display caps.
    Windows,
}

/// Legacy selector names, retained only for the migration/effective-config table.
pub const P19_NAMES: [&str; 19] = [
    "KF3_WIN_USER_CHANNELS_PASSTHROUGH",
    "KF3_WIN_TWIN_DEFAPI_OBJECT",
    "KF3_KERNEL_GR_CE",
    "KF3_KERNEL_GR_WORK",
    "KF3_TRANSLATED_CE_RELAY",
    "KF3_DEFERRED_API",
    "KF3_PREEMPT_BIND_PROBE",
    "KF3_DISPLAY_IMP_ENABLE",
    "KF3_DISPLAY_CTRL_PROBE",
    "KF3_ZCULL_BIND_PROBE",
    "KF3_SW_RUNLIST_HOST_OWNED",
    "KF3_ASYNC_PREEMPT",
    "KF3_GFX_POOL_PROBE",
    "KF3_SW_RUNLIST_PROBE",
    "KF3_MEMORY_LIST_PROBE",
    "KF3_DISPLAY_SDR_COLOR",
    "KF3_DISPLAY_CAPS_PROBE",
    "KF3_DISPLAY_LUT_MIRROR",
    "KF3_DISPLAY_ILUT_OFFSET_256",
];

/// Immutable compatibility settings carried by one device into its construction boundaries.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settings {
    /// Replaces `KF3_WIN_USER_CHANNELS_PASSTHROUGH`; selected only by the explicit Windows profile.
    pub win_user_channels_passthrough: bool,
    /// Replaces `KF3_WIN_TWIN_DEFAPI_OBJECT`; selected only by the explicit Windows profile.
    pub win_twin_defapi_object: bool,
    /// Replaces `KF3_KERNEL_GR_CE`; selected only by the explicit Windows profile.
    pub kernel_gr_ce: bool,
    /// Replaces `KF3_KERNEL_GR_WORK`; selected only by the explicit Windows profile.
    pub kernel_gr_work: bool,
    /// Replaces `KF3_TRANSLATED_CE_RELAY`; selected only by the explicit Windows profile.
    pub translated_ce_relay: bool,
    /// Replaces `KF3_DEFERRED_API`; selected only by the explicit Windows profile.
    pub deferred_api: bool,
    /// Replaces `KF3_PREEMPT_BIND_PROBE`; selected only by the explicit Windows profile.
    pub preempt_bind_probe: bool,
    /// Replaces `KF3_DISPLAY_IMP_ENABLE`; selected only by the explicit Windows profile.
    pub display_imp_enable: bool,
    /// Replaces `KF3_DISPLAY_CTRL_PROBE`; selected only by the explicit Windows profile.
    pub display_ctrl_probe: bool,
    /// Replaces `KF3_ZCULL_BIND_PROBE`; selected only by the explicit Windows profile.
    pub zcull_bind_probe: bool,
    /// Replaces `KF3_SW_RUNLIST_HOST_OWNED`; selected only by the explicit Windows profile.
    pub sw_runlist_host_owned: bool,
    /// Replaces `KF3_ASYNC_PREEMPT`; selected only by the explicit Windows profile.
    pub async_preempt: bool,
    /// Replaces `KF3_GFX_POOL_PROBE`; selected only by the explicit Windows profile.
    pub gfx_pool_probe: bool,
    /// Replaces `KF3_SW_RUNLIST_PROBE`; selected only by the explicit Windows profile.
    pub sw_runlist_probe: bool,
    /// Replaces `KF3_MEMORY_LIST_PROBE`; selected only by the explicit Windows profile.
    pub memory_list_probe: bool,
    /// Replaces `KF3_DISPLAY_SDR_COLOR`; selected only by the explicit Windows profile.
    pub display_sdr_color: bool,
    /// Retired `KF3_DISPLAY_CAPS_PROBE` selector; always false in both profiles.
    pub display_caps_probe: bool,
    /// Replaces `KF3_DISPLAY_LUT_MIRROR`; selected only by the explicit Windows profile.
    pub display_lut_mirror: bool,
    /// Replaces `KF3_DISPLAY_ILUT_OFFSET_256`; selected only by the explicit Windows profile.
    pub display_ilut_offset_256: bool,
    /// Use public rules and exact-die measurement rows instead of a captured caps page.
    pub derived_caps: bool,
}

impl GuestProfile {
    /// Parse a property before opening host devices. Unspecified means Linux.
    ///
    /// # Errors
    /// Only the exact strings `linux` and `windows` are accepted.
    pub fn parse(value: Option<&str>) -> Result<Self, &'static str> {
        match value {
            None | Some("linux") => Ok(Self::Linux),
            Some("windows") => Ok(Self::Windows),
            _ => Err("guest-os must be linux or windows (default linux)"),
        }
    }

    /// Effective compatibility configuration, independent of environment variables.
    #[must_use]
    pub const fn settings(self) -> Settings {
        let windows = matches!(self, Self::Windows);
        Settings {
            win_user_channels_passthrough: windows,
            win_twin_defapi_object: windows,
            kernel_gr_ce: windows,
            kernel_gr_work: windows,
            translated_ce_relay: windows,
            deferred_api: windows,
            preempt_bind_probe: windows,
            display_imp_enable: windows,
            display_ctrl_probe: windows,
            zcull_bind_probe: windows,
            sw_runlist_host_owned: windows,
            async_preempt: windows,
            gfx_pool_probe: windows,
            sw_runlist_probe: windows,
            memory_list_probe: windows,
            display_sdr_color: windows,
            display_caps_probe: false,
            display_lut_mirror: windows,
            display_ilut_offset_256: windows,
            derived_caps: windows,
        }
    }
}

impl Settings {
    /// Cold-path migration/configuration dump lookup; diagnostics are not boot settings.
    #[must_use]
    pub fn value(self, name: &str) -> Option<bool> {
        match name {
            "KF3_WIN_USER_CHANNELS_PASSTHROUGH" => Some(self.win_user_channels_passthrough),
            "KF3_WIN_TWIN_DEFAPI_OBJECT" => Some(self.win_twin_defapi_object),
            "KF3_KERNEL_GR_CE" => Some(self.kernel_gr_ce),
            "KF3_KERNEL_GR_WORK" => Some(self.kernel_gr_work),
            "KF3_TRANSLATED_CE_RELAY" => Some(self.translated_ce_relay),
            "KF3_DEFERRED_API" => Some(self.deferred_api),
            "KF3_PREEMPT_BIND_PROBE" => Some(self.preempt_bind_probe),
            "KF3_DISPLAY_IMP_ENABLE" => Some(self.display_imp_enable),
            "KF3_DISPLAY_CTRL_PROBE" => Some(self.display_ctrl_probe),
            "KF3_ZCULL_BIND_PROBE" => Some(self.zcull_bind_probe),
            "KF3_SW_RUNLIST_HOST_OWNED" => Some(self.sw_runlist_host_owned),
            "KF3_ASYNC_PREEMPT" => Some(self.async_preempt),
            "KF3_GFX_POOL_PROBE" => Some(self.gfx_pool_probe),
            "KF3_SW_RUNLIST_PROBE" => Some(self.sw_runlist_probe),
            "KF3_MEMORY_LIST_PROBE" => Some(self.memory_list_probe),
            "KF3_DISPLAY_SDR_COLOR" => Some(self.display_sdr_color),
            "KF3_DISPLAY_CAPS_PROBE" => Some(self.display_caps_probe),
            "KF3_DISPLAY_LUT_MIRROR" => Some(self.display_lut_mirror),
            "KF3_DISPLAY_ILUT_OFFSET_256" => Some(self.display_ilut_offset_256),
            _ => None,
        }
    }
}
