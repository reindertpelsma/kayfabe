// @generated from ogkm 535.309.01..615.71.09 (30 tags) + served.json by tools/api_boundaries/ingress_policy.py; do not edit.
// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// The ogkm facts are NVIDIA's (MIT, see THIRD_PARTY_NOTICES.md); the policy and the rule rows are
// kayfabe's (tools/api_boundaries/ingress_rules.tsv, docs/design/V3_GUEST_INGRESS_TABLE.md).
// Regenerate: tools/api_boundaries/ingress_policy.py extract ... && ingress_policy.py emit ...

use crate::DriverVersion;
use crate::capability::{Basis, IngressRow, Judgement, Refusal, Run, Verdict};

/// The guest driver tags this table covers (the driver matrix's tags), ascending.
pub const TAGS: &[DriverVersion] = &[
    DriverVersion { major: 535, minor: 309, patch: 1 },
    DriverVersion { major: 545, minor: 23, patch: 8 },
    DriverVersion { major: 550, minor: 40, patch: 7 },
    DriverVersion { major: 550, minor: 54, patch: 14 },
    DriverVersion { major: 550, minor: 90, patch: 7 },
    DriverVersion { major: 555, minor: 42, patch: 2 },
    DriverVersion { major: 560, minor: 28, patch: 3 },
    DriverVersion { major: 565, minor: 57, patch: 1 },
    DriverVersion { major: 570, minor: 86, patch: 15 },
    DriverVersion { major: 570, minor: 124, patch: 6 },
    DriverVersion { major: 570, minor: 148, patch: 8 },
    DriverVersion { major: 575, minor: 51, patch: 2 },
    DriverVersion { major: 575, minor: 51, patch: 3 },
    DriverVersion { major: 575, minor: 57, patch: 8 },
    DriverVersion { major: 575, minor: 64, patch: 5 },
    DriverVersion { major: 580, minor: 65, patch: 6 },
    DriverVersion { major: 580, minor: 82, patch: 7 },
    DriverVersion { major: 580, minor: 94, patch: 2 },
    DriverVersion { major: 580, minor: 95, patch: 5 },
    DriverVersion { major: 580, minor: 105, patch: 8 },
    DriverVersion { major: 580, minor: 126, patch: 9 },
    DriverVersion { major: 580, minor: 159, patch: 4 },
    DriverVersion { major: 580, minor: 173, patch: 2 },
    DriverVersion { major: 580, minor: 178, patch: 4 },
    DriverVersion { major: 590, minor: 48, patch: 1 },
    DriverVersion { major: 595, minor: 84, patch: 0 },
    DriverVersion { major: 595, minor: 91, patch: 7 },
    DriverVersion { major: 610, minor: 43, patch: 2 },
    DriverVersion { major: 610, minor: 57, patch: 4 },
    DriverVersion { major: 615, minor: 71, patch: 9 },
];

/// Every control kayfabe serves (or a rule row names), sorted by id; one run per span of equal verdicts.
pub static CONTROLS: &[IngressRow] = &[
    IngressRow {
        id: 0x00410103,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 0, name: "NV0041_CTRL_CMD_GET_SURFACE_PHYS_ATTR", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 1, last: 14, name: "NV0041_CTRL_CMD_GET_SURFACE_PHYS_ATTR", verdict: Verdict::Refuse(Refusal::KernelOnly) },
            Run { first: 15, last: 29, name: "NV0041_CTRL_CMD_GET_SURFACE_PHYS_ATTR", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x00730101,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0073_CTRL_CMD_SYSTEM_GET_CAPS_V2", verdict: Verdict::Allow(Basis::Empirical) },
        ],
    },
    IngressRow {
        id: 0x00730102,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0073_CTRL_CMD_SYSTEM_GET_NUM_HEADS", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::DisplayEngine)) },
        ],
    },
    IngressRow {
        id: 0x00730107,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0073_CTRL_CMD_SYSTEM_GET_SUPPORTED", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::DisplayEngine)) },
        ],
    },
    IngressRow {
        id: 0x00730108,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 4, name: "NV0073_CTRL_CMD_SYSTEM_GET_SCANLINE", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::DisplayEngine)) },
            Run { first: 5, last: 29, name: "NV0073_CTRL_CMD_SYSTEM_GET_CONNECT_STATE", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::DisplayEngine)) },
        ],
    },
    IngressRow {
        id: 0x00730109,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 4, name: "NV0073_CTRL_CMD_SYSTEM_GET_VBLANK_COUNTER", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::DisplayEngine)) },
            Run { first: 5, last: 29, name: "NV0073_CTRL_CMD_SYSTEM_GET_HOTPLUG_CONFIG", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::DisplayEngine)) },
        ],
    },
    IngressRow {
        id: 0x0073010a,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 4, name: "NV0073_CTRL_CMD_SYSTEM_GET_VBLANK_ENABLE", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::DisplayEngine)) },
            Run { first: 5, last: 29, name: "NV0073_CTRL_CMD_SYSTEM_GET_HOTPLUG_STATE", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::DisplayEngine)) },
        ],
    },
    IngressRow {
        id: 0x0073010b,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0073_CTRL_CMD_SYSTEM_GET_HEAD_ROUTING_MAP", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::DisplayEngine)) },
        ],
    },
    IngressRow {
        id: 0x0073010c,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0073_CTRL_CMD_SYSTEM_GET_ACTIVE", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::DisplayEngine)) },
        ],
    },
    IngressRow {
        id: 0x00730116,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0073_CTRL_CMD_SYSTEM_GET_INTERNAL_DISPLAYS", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::DisplayEngine)) },
        ],
    },
    IngressRow {
        id: 0x0073011d,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 4, name: "NV0073_CTRL_CMD_SYSTEM_GET_CONNECTOR_TABLE", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 5, last: 29, name: "NV0073_CTRL_CMD_SYSTEM_GET_CONNECTOR_TABLE", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x0073011e,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 4, name: "NV0073_CTRL_CMD_SYSTEM_GET_BOOT_DISPLAYS", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 5, last: 29, name: "NV0073_CTRL_CMD_SYSTEM_GET_BOOT_DISPLAYS", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x00730120,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 4, name: "NV0073_CTRL_CMD_SYSTEM_GET_SUPPORTED", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::DisplayEngine)) },
            Run { first: 5, last: 29, name: "NV0073_CTRL_CMD_SYSTEM_EXECUTE_ACPI_METHOD", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::DisplayEngine)) },
        ],
    },
    IngressRow {
        id: 0x00730122,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0073_CTRL_CMD_SYSTEM_GET_CONNECT_STATE", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::DisplayEngine)) },
        ],
    },
    IngressRow {
        id: 0x00730124,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV0073_CTRL_CMD_SYSTEM_GET_HOTPLUG_STATE", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::DisplayEngine)) },
        ],
    },
    IngressRow {
        id: 0x00730125,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV0073_CTRL_CMD_SYSTEM_GET_HEAD_ROUTING_MAP", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::DisplayEngine)) },
        ],
    },
    IngressRow {
        id: 0x00730126,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV0073_CTRL_CMD_SYSTEM_GET_ACTIVE", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::DisplayEngine)) },
        ],
    },
    IngressRow {
        id: 0x0073012c,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 4, name: "NV0073_CTRL_CMD_SYSTEM_VRR_DISPLAY_INFO", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 5, last: 29, name: "NV0073_CTRL_CMD_SYSTEM_VRR_DISPLAY_INFO", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x00730138,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV0073_CTRL_CMD_SYSTEM_GET_CAPS_V2", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::DisplayEngine)) },
        ],
    },
    IngressRow {
        id: 0x0073013d,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 4, name: "NV0073_CTRL_CMD_SYSTEM_QUERY_DISPLAY_IDS_WITH_MUX", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 5, last: 29, name: "NV0073_CTRL_CMD_SYSTEM_QUERY_DISPLAY_IDS_WITH_MUX", verdict: Verdict::Refuse(Refusal::KernelOnly) },
        ],
    },
    IngressRow {
        id: 0x0073014b,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 4, name: "NV0073_CTRL_CMD_SYSTEM_CHECK_SIDEBAND_I2C_SUPPORT", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 5, last: 29, name: "NV0073_CTRL_CMD_SYSTEM_CHECK_SIDEBAND_I2C_SUPPORT", verdict: Verdict::Refuse(Refusal::KernelOnly) },
        ],
    },
    IngressRow {
        id: 0x0073015b,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV0073_CTRL_CMD_SYSTEM_GET_INTERNAL_DISPLAYS", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::DisplayEngine)) },
        ],
    },
    IngressRow {
        id: 0x00730166,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 4, name: "NV0073_CTRL_CMD_SYSTEM_GET_BOOT_DISPLAYS", verdict: Verdict::Refuse(Refusal::Privileged) },
            Run { first: 5, last: 29, name: "NV0073_CTRL_CMD_SYSTEM_GET_BOOT_DISPLAYS", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
        ],
    },
    IngressRow {
        id: 0x00730240,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0073_CTRL_CMD_SPECIFIC_GET_TYPE", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x00730245,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0073_CTRL_CMD_SPECIFIC_GET_EDID_V2", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x00730246,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV0073_CTRL_CMD_SPECIFIC_SET_EDID_V2", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x00730250,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0073_CTRL_CMD_SPECIFIC_GET_CONNECTOR_DATA", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::DisplayEngine)) },
        ],
    },
    IngressRow {
        id: 0x00730273,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 14, name: "NV0073_CTRL_CMD_SPECIFIC_SET_HDMI_ENABLE", verdict: Verdict::Refuse(Refusal::KernelOnly) },
            Run { first: 15, last: 29, name: "NV0073_CTRL_CMD_SPECIFIC_SET_HDMI_ENABLE", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x00730280,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 24, name: "NV0073_CTRL_CMD_SPECIFIC_GET_HDCP_STATE", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 25, last: 29, name: "NV0073_CTRL_CMD_SPECIFIC_GET_HDCP_STATE", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x00730282,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 24, name: "NV0073_CTRL_CMD_SPECIFIC_HDCP_CTRL", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 25, last: 29, name: "NV0073_CTRL_CMD_SPECIFIC_HDCP_CTRL", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x00730285,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0073_CTRL_CMD_SPECIFIC_GET_ACPI_DOD_DISPLAY_PORT_ATTACHMENT", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x00730287,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0073_CTRL_CMD_SPECIFIC_GET_ALL_HEAD_MASK", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x00730288,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 14, name: "NV0073_CTRL_CMD_SPECIFIC_SET_OD_PACKET", verdict: Verdict::Refuse(Refusal::KernelOnly) },
            Run { first: 15, last: 29, name: "NV0073_CTRL_CMD_SPECIFIC_SET_OD_PACKET", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x0073028a,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0073_CTRL_CMD_SPECIFIC_GET_PCLK_LIMIT", verdict: Verdict::Refuse(Refusal::KernelOnly) },
        ],
    },
    IngressRow {
        id: 0x0073028b,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0073_CTRL_CMD_SPECIFIC_OR_GET_INFO", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x00730291,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV0073_CTRL_CMD_SPECIFIC_GET_BACKLIGHT_BRIGHTNESS", verdict: Verdict::Refuse(Refusal::KernelOnly) },
        ],
    },
    IngressRow {
        id: 0x00730293,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 14, name: "NV0073_CTRL_CMD_SPECIFIC_SET_HDMI_SINK_CAPS", verdict: Verdict::Refuse(Refusal::KernelOnly) },
            Run { first: 15, last: 29, name: "NV0073_CTRL_CMD_SPECIFIC_SET_HDMI_SINK_CAPS", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x007302a2,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0073_CTRL_CMD_SPECIFIC_GET_HDMI_GPU_CAPS", verdict: Verdict::Refuse(Refusal::KernelOnly) },
        ],
    },
    IngressRow {
        id: 0x007302a4,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0073_CTRL_CMD_SPECIFIC_DISPLAY_CHANGE", verdict: Verdict::Refuse(Refusal::KernelOnly) },
        ],
    },
    IngressRow {
        id: 0x007302a6,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 14, name: "NV0073_CTRL_CMD_SPECIFIC_GET_HDMI_SCDC_DATA", verdict: Verdict::Refuse(Refusal::KernelOnly) },
            Run { first: 15, last: 29, name: "NV0073_CTRL_CMD_SPECIFIC_GET_HDMI_SCDC_DATA", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x007302a7,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV0073_CTRL_CMD_SPECIFIC_IS_DIRECTMODE_DISPLAY", verdict: Verdict::Refuse(Refusal::KernelOnly) },
        ],
    },
    IngressRow {
        id: 0x007302ad,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV0073_CTRL_CMD_SPECIFIC_GET_VALID_HEAD_WINDOW_ASSIGNMENT", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::DisplayEngine)) },
        ],
    },
    IngressRow {
        id: 0x00730401,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0073_CTRL_CMD_INTERNAL_GET_HOTPLUG_UNPLUG_STATE", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x00730404,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 1, name: "NV0073_CTRL_CMD_INTERNAL_DFP_GET_DISP_MUX_STATUS", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 2, last: 29, name: "NV0073_CTRL_CMD_INTERNAL_DFP_GET_DISP_MUX_STATUS", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x00731140,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0073_CTRL_CMD_DFP_GET_INFO", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::DisplayEngine)) },
        ],
    },
    IngressRow {
        id: 0x00731142,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0073_CTRL_CMD_DFP_GET_DISPLAYPORT_DONGLE_INFO", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::DisplayEngine)) },
        ],
    },
    IngressRow {
        id: 0x00731144,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 14, name: "NV0073_CTRL_CMD_DFP_SET_ELD_AUDIO_CAPS", verdict: Verdict::Refuse(Refusal::KernelOnly) },
            Run { first: 15, last: 29, name: "NV0073_CTRL_CMD_DFP_SET_ELD_AUDIO_CAPS", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x00731152,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 14, name: "NV0073_CTRL_CMD_DFP_ASSIGN_SOR", verdict: Verdict::Refuse(Refusal::KernelOnly) },
            Run { first: 15, last: 29, name: "NV0073_CTRL_CMD_DFP_ASSIGN_SOR", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x00731153,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0073_CTRL_CMD_DFP_GET_PADLINK_MASK", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x00731341,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0073_CTRL_CMD_DP_AUXCH_CTRL", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x00731365,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0073_CTRL_CMD_DP_SET_MANUAL_DISPLAYPORT", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x00731369,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0073_CTRL_CMD_DP_GET_CAPS", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x00800106,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0080_CTRL_CMD_BIF_GET_PCIE_POWER_CONTROL_MASK", verdict: Verdict::Refuse(Refusal::KernelOnly) },
        ],
    },
    IngressRow {
        id: 0x0080028b,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "UNKNOWN_CONTROL_COMMAND_80028B", verdict: Verdict::Allow(Basis::Empirical) },
        ],
    },
    IngressRow {
        id: 0x0080028e,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV0080_CTRL_CMD_GPU_GET_VGX_CAPS", verdict: Verdict::Allow(Basis::Empirical) },
        ],
    },
    IngressRow {
        id: 0x00800292,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV0080_CTRL_CMD_GPU_GET_CLASSLIST_V2", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x00800294,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0080_CTRL_CMD_GPU_GET_BRAND_CAPS", verdict: Verdict::Allow(Basis::Empirical) },
        ],
    },
    IngressRow {
        id: 0x00801306,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0080_CTRL_CMD_FB_GET_COMPBIT_STORE_INFO", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::AnsweredInChain)) },
        ],
    },
    IngressRow {
        id: 0x00801401,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV0080_CTRL_CMD_HOST_GET_CAPS", verdict: Verdict::Allow(Basis::Empirical) },
        ],
    },
    IngressRow {
        id: 0x00801707,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0080_CTRL_CMD_FIFO_GET_ENGINE_CONTEXT_PROPERTIES", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0080170e,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV0080_CTRL_CMD_FIFO_GET_LATENCY_BUFFER_SIZE", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::AnsweredInChain)) },
        ],
    },
    IngressRow {
        id: 0x0080170f,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0080_CTRL_CMD_FIFO_SET_CHANNEL_PROPERTIES", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x00801714,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0080_CTRL_CMD_FIFO_IDLE_CHANNELS", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::AnsweredInChain)) },
        ],
    },
    IngressRow {
        id: 0x00801801,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0080_CTRL_CMD_DMA_GET_PTE_INFO", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::AnsweredInChain)) },
        ],
    },
    IngressRow {
        id: 0x00801809,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0080_CTRL_CMD_DMA_GET_PDE_INFO", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::AnsweredInChain)) },
        ],
    },
    IngressRow {
        id: 0x00801812,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0080_CTRL_DMA_SET_DEFAULT_VASPACE", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x00801813,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x00801814,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0080_CTRL_CMD_DMA_UNSET_PAGE_DIRECTORY", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x00801909,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 1, name: "NV0080_CTRL_CMD_PERF_CUDA_LIMIT_SET_CONTROL", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 2, last: 29, name: "NV0080_CTRL_CMD_PERF_CUDA_LIMIT_SET_CONTROL", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x00801b02,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 1, name: "NV0080_CTRL_CMD_NVENC_GET_CAPS_V2", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::AnsweredInChain)) },
            Run { first: 2, last: 26, name: "NV0080_CTRL_CMD_MSENC_GET_CAPS_V2", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::AnsweredInChain)) },
            Run { first: 27, last: 29, name: "NV0080_CTRL_CMD_NVENC_GET_CAPS_V2", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::AnsweredInChain)) },
        ],
    },
    IngressRow {
        id: 0x00801c02,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 26, name: "NV0080_CTRL_CMD_BSP_GET_CAPS_V2", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
            Run { first: 27, last: 29, name: "NV0080_CTRL_CMD_NVDEC_GET_CAPS_V2", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x00802004,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0080_CTRL_CMD_INTERNAL_PERF_CUDA_LIMIT_DISABLE", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x00802009,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0080_CTRL_CMD_INTERNAL_PERF_CUDA_LIMIT_SET_CONTROL", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x0080200a,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0080_CTRL_CMD_INTERNAL_MEMSYS_SET_ZBC_REFERENCED", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::AnsweredInChain)) },
        ],
    },
    IngressRow {
        id: 0x00e00102,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV00E0_CTRL_CMD_IMPORT_MEM", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x00f10003,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV00F1_CTRL_CMD_DISABLE_IMPORTERS", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x20800102,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GPU_GET_INFO_V2", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x20800110,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GPU_GET_NAME_STRING", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x20800111,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GPU_GET_SHORT_NAME_STRING", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x20800122,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GPU_EXEC_REG_OPS", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::RegisterAccess)) },
        ],
    },
    IngressRow {
        id: 0x2080012b,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GPU_PROMOTE_CTX", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x2080012c,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GPU_EVICT_CTX", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x2080012d,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GPU_INITIALIZE_CTX", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x2080012f,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GPU_QUERY_ECC_STATUS", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x2080013f,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GPU_GET_OEM_BOARD_INFO", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x2080014a,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GPU_GET_GID_INFO", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x2080014b,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GPU_GET_INFOROM_OBJECT_VERSION", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x20800156,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GPU_GET_INFOROM_IMAGE_VERSION", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x20800157,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GPU_QUERY_INFOROM_ECC_SUPPORT", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x2080015a,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GPU_GET_PHYSICAL_BRIDGE_VERSION_INFO", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x2080015f,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GPU_QUERY_SCRUBBER_STATUS", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x20800160,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GPU_GET_VPR_CAPS", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x20800168,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GPU_GET_PES_INFO", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::AnsweredInChain)) },
        ],
    },
    IngressRow {
        id: 0x20800169,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GPU_GET_OEM_INFO", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x20800170,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GPU_GET_ENGINES_V2", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x20800173,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GPU_QUERY_FUNCTION_STATUS", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x20800177,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GPU_REPORT_NON_REPLAYABLE_FAULT", verdict: Verdict::Refuse(Refusal::KernelOnly) },
        ],
    },
    IngressRow {
        id: 0x2080017e,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GPU_GET_VMMU_SEGMENT_SIZE", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::AnsweredInChain)) },
        ],
    },
    IngressRow {
        id: 0x208001a4,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GPU_GET_CHIP_DETAILS", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::AnsweredInChain)) },
        ],
    },
    IngressRow {
        id: 0x208001b0,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GPU_GET_CONSTRUCTED_FALCON_INFO", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::AnsweredInChain)) },
        ],
    },
    IngressRow {
        id: 0x20800301,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x20800404,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_TIMER_GET_REGISTER_OFFSET", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::RegisterAccess)) },
        ],
    },
    IngressRow {
        id: 0x20800808,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_BIOS_GET_SKU_INFO", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x2080080b,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_BIOS_GET_UEFI_SUPPORT", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x2080080e,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_BIOS_GET_NBSI_V2", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x20800810,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_BIOS_GET_INFO_V2", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x20800a00,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 24, name: "NV2080_CTRL_CMD_INTERNAL_BIF_GET_DATA", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 25, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_BIF_GET_DATA", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a01,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_DISPLAY_GET_STATIC_INFO", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a1c,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_MEMSYS_GET_STATIC_CONFIG", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a1d,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_UVM_REGISTER_ACCESS_CNTR_BUFFER", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a1f,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_CAPS", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a22,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_GLOBAL_SM_ORDER", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a26,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_FLOORSWEEPING_MASKS", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a2a,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_INFO", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a2c,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_ZCULL_INFO", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a2e,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_ROP_INFO", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a30,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_PPC_MASKS", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a32,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_CONTEXT_BUFFERS_INFO", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a34,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_SM_ISSUE_RATE_MODIFIER", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a36,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 28, name: "NV2080_CTRL_CMD_INTERNAL_GPU_GET_CHIP_INFO", verdict: Verdict::Refuse(Refusal::Internal) },
            Run { first: 29, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_GPU_SYNC_FABRIC_PROBE_INFO", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
        ],
    },
    IngressRow {
        id: 0x20800a37,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 2, name: "NV2080_CTRL_CMD_INTERNAL_GR_SET_FECS_TRACE_HW_ENABLE", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 3, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_GR_SET_FECS_TRACE_HW_ENABLE", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a38,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 2, name: "NV2080_CTRL_CMD_INTERNAL_GR_GET_FECS_TRACE_HW_ENABLE", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 3, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_GR_GET_FECS_TRACE_HW_ENABLE", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a39,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 2, name: "NV2080_CTRL_CMD_INTERNAL_GR_SET_FECS_TRACE_RD_OFFSET", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 3, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_GR_SET_FECS_TRACE_RD_OFFSET", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a3a,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 2, name: "NV2080_CTRL_CMD_INTERNAL_GR_SET_FECS_TRACE_WR_OFFSET", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 3, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_GR_SET_FECS_TRACE_WR_OFFSET", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a3d,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_FECS_RECORD_SIZE", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a3f,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_FECS_TRACE_DEFINES", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a40,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_GET_DEVICE_INFO_TABLE", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a41,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_GPU_GET_USER_REGISTER_ACCESS_MAP", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a48,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_PDB_PROPERTIES", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a49,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_DISPLAY_WRITE_INST_MEM", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a4b,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_DISPLAY_GET_IP_VERSION", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a4c,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_GPU_GET_SMC_MODE", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a58,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a59,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_GMMU_GET_STATIC_INFO", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a5c,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_INTR_GET_KERNEL_TABLE", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a5d,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_DISPLAY_GET_ACTIVE_DISPLAY_DEVICES", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a61,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_FIFO_GET_NUM_CHANNELS", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a69,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_MEMSYS_SET_ZBC_REFERENCED", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::AnsweredInChain)) },
        ],
    },
    IngressRow {
        id: 0x20800a6c,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_MEMSYS_L2_INVALIDATE_EVICT", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a70,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_BUS_FLUSH_WITH_SYSMEMBAR", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a76,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 1, name: "NV2080_CTRL_CMD_INTERNAL_DISPLAY_UNIX_CONSOLE", verdict: Verdict::Refuse(Refusal::Internal) },
            Run { first: 2, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_DISPLAY_PRE_UNIX_CONSOLE", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a77,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 1, name: "NV2080_CTRL_CMD_INTERNAL_DISPLAY_POST_RESTORE", verdict: Verdict::Refuse(Refusal::Internal) },
            Run { first: 2, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_DISPLAY_POST_UNIX_CONSOLE", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a80,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_PERF_GPU_BOOST_SYNC_GET_INFO", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a87,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 7, name: "NV2080_CTRL_CMD_INTERNAL_NVLINK_GET_NVLINK_DEVICE_INFO", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 8, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_NVLINK_GET_NVLINK_DEVICE_INFO", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a89,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 7, name: "NV2080_CTRL_CMD_INTERNAL_HSHUB_FIRST_LINK_PEER_ID", verdict: Verdict::Refuse(Refusal::Internal) },
            Run { first: 8, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_SEND_CMC_LIBOS_BUFFER_INFO", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
        ],
    },
    IngressRow {
        id: 0x20800a9a,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_PERF_BOOST_SET_2X", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a9b,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a9d,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_CLIENT_SHADOW_FAULT_BUFFER", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800a9f,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x20800aac,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_BIF_GET_STATIC_INFO", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800ab8,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_GET_PCIE_P2P_CAPS", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800ac2,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_FBSR_INIT", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800ac3,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 4, name: "NV2080_CTRL_CMD_INTERNAL_FBSR_SEND_REGION_INFO", verdict: Verdict::Refuse(Refusal::Internal) },
            Run { first: 5, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_FIFO_TOGGLE_ACTIVE_CHANNEL_SCHEDULING", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800ac4,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_GSYNC_GET_VERTICAL_ACTIVE_LINES", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800ac6,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_INIT_BRIGHTC_STATE_LOAD", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800adf,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_SET_STATIC_EDID_DATA", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800af0,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_DISPLAY_ACPI_SUBSYSTEM_ACTIVATED", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800af1,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_DISPLAY_PRE_MODESET", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800af2,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_DISPLAY_POST_MODESET", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800af3,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_CONF_COMPUTE_GET_STATIC_INFO", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800afe,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 0, name: "NV2080_CTRL_CMD_INTERNAL_INIT_USER_SHARED_DATA", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 1, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_INIT_USER_SHARED_DATA", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800aff,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 0, name: "NV2080_CTRL_CMD_INTERNAL_USER_SHARED_DATA_SET_DATA_POLL", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 1, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_USER_SHARED_DATA_SET_DATA_POLL", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800b03,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 10, name: "NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_SM_ISSUE_RATE_MODIFIER_V2", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 11, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_SM_ISSUE_RATE_MODIFIER_V2", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20800b05,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 14, name: "NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_SM_ISSUE_THROTTLE_CTRL", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 15, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_SM_ISSUE_THROTTLE_CTRL", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20801109,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_FIFO_GET_INFO", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::AnsweredInChain)) },
        ],
    },
    IngressRow {
        id: 0x2080110b,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_FIFO_DISABLE_CHANNELS", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x20801110,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 18, name: "NV2080_CTRL_CMD_FIFO_CONFIG_CTXSW_TIMEOUT", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 19, last: 29, name: "NV2080_CTRL_CMD_FIFO_CONFIG_CTXSW_TIMEOUT", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x20801112,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE", verdict: Verdict::Refuse(Refusal::KernelOnly) },
        ],
    },
    IngressRow {
        id: 0x20801115,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_FIFO_RUNLIST_SET_SCHED_POLICY", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x20801116,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_FIFO_UPDATE_CHANNEL_INFO", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x20801117,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_FIFO_DISABLE_USERMODE_CHANNELS", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x20801119,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_FIFO_GET_ALLOCATED_CHANNELS", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x20801206,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GR_GET_ZCULL_INFO", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x20801208,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GR_CTXSW_ZCULL_BIND", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x20801209,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GR_CTXSW_PM_BIND", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::AnsweredInChain)) },
        ],
    },
    IngressRow {
        id: 0x20801210,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x20801211,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GR_CTXSW_PREEMPTION_BIND", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::AnsweredInChain)) },
        ],
    },
    IngressRow {
        id: 0x20801219,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GR_GET_CTX_BUFFER_INFO", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x2080121b,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GR_GET_GLOBAL_SM_ORDER", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x2080121f,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GR_GFX_POOL_QUERY_SIZE", verdict: Verdict::Refuse(Refusal::KernelOnly) },
        ],
    },
    IngressRow {
        id: 0x20801220,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GR_GFX_POOL_INITIALIZE", verdict: Verdict::Refuse(Refusal::KernelOnly) },
        ],
    },
    IngressRow {
        id: 0x20801221,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GR_GFX_POOL_ADD_SLOTS", verdict: Verdict::Refuse(Refusal::KernelOnly) },
        ],
    },
    IngressRow {
        id: 0x20801222,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GR_GFX_POOL_REMOVE_SLOTS", verdict: Verdict::Refuse(Refusal::KernelOnly) },
        ],
    },
    IngressRow {
        id: 0x20801227,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GR_GET_CAPS_V2", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x20801228,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GR_GET_INFO_V2", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::AnsweredInChain)) },
        ],
    },
    IngressRow {
        id: 0x2080122a,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GR_GET_GPC_MASK", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x2080122b,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GR_GET_TPC_MASK", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x2080122d,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GR_GET_ENGINE_CONTEXT_PROPERTIES", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::AnsweredInChain)) },
        ],
    },
    IngressRow {
        id: 0x20801230,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GR_GET_SM_ISSUE_RATE_MODIFIER", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x20801232,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GR_GET_PHYS_GPC_MASK", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x20801234,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GR_GET_NUM_TPCS_FOR_GPC", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::AnsweredInChain)) },
        ],
    },
    IngressRow {
        id: 0x20801237,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GR_GET_ZCULL_MASK", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::AnsweredInChain)) },
        ],
    },
    IngressRow {
        id: 0x20801239,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GR_GET_GFX_GPC_AND_TPC_INFO", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::AnsweredInChain)) },
        ],
    },
    IngressRow {
        id: 0x20801303,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_FB_GET_INFO_V2", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x2080130e,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_FB_FLUSH_GPU_CACHE", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x20801315,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_FB_GET_GPU_CACHE_INFO", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x20801322,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_FB_GET_OFFLINED_PAGES", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x20801344,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_FB_GET_REMAPPED_ROWS", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x20801347,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_FB_GET_ROW_REMAPPER_HISTOGRAM", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x20801357,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 7, name: "NV2080_CTRL_CMD_FB_GET_STATUS", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 8, last: 10, name: "NV2080_CTRL_CMD_FB_GET_STATUS", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
            Run { first: 11, last: 29, name: "NV2080_CTRL_CMD_FB_QUERY_DRAM_ENCRYPTION_INFOROM_SUPPORT", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x20801701,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_MC_GET_ARCH_INFO", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x20801702,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x2080170d,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_MC_GET_ENGINE_NOTIFICATION_INTR_VECTORS", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x2080170e,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_MC_GET_STATIC_INTR_TABLE", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x2080170f,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_MC_GET_INTR_CATEGORY_SUBTREE_MAP", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x20801801,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_BUS_GET_PCI_INFO", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x20801803,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_BUS_GET_PCI_BAR_INFO", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x20801813,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_BUS_GET_PEX_COUNTERS", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x20801819,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_BUS_GET_PEX_UTIL_COUNTERS", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x20801823,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_BUS_GET_INFO_V2", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x20801829,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_BUS_GET_PCIE_REQ_ATOMICS_CAPS", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x2080182a,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_BUS_GET_PCIE_SUPPORTED_GPU_ATOMICS", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x2080182b,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_BUS_GET_C2C_INFO", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x20801830,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_BUS_GET_PCIE_CPL_ATOMICS_CAPS", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x2080200b,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_PERF_GET_LEVEL_INFO_V2", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x2080205a,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_PERF_GET_POWERSTATE", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x2080205b,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_PERF_SET_POWERSTATE", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x20802068,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 1, name: "NV2080_CTRL_CMD_PERF_GET_CURRENT_PSTATE", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 2, last: 29, name: "NV2080_CTRL_CMD_PERF_GET_CURRENT_PSTATE", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x20802096,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_PERF_GET_GPUMON_PERFMON_UTIL_SAMPLES_V2", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x20802205,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_RC_GET_ERROR_COUNT", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x20802209,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_RC_GET_WATCHDOG_INFO", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x2080220c,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_RC_RELEASE_WATCHDOG_REQUESTS", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x2080220d,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_SET_RC_RECOVERY", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x2080220e,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GET_RC_RECOVERY", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x20802213,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_RC_GET_ERROR_V2", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x20802502,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_DMA_INVALIDATE_TLB", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x20802801,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_LPWR_DIFR_CTRL", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x20802a01,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_CE_GET_CAPS", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::AnsweredInChain)) },
        ],
    },
    IngressRow {
        id: 0x20802a02,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_CE_GET_CE_PCE_MASK", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x20802a03,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_CE_GET_CAPS_V2", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x20802a06,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_CE_UPDATE_CLASS_DB", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20802a07,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_CE_GET_PHYSICAL_CAPS", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20802a08,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_CE_GET_FAULT_METHOD_BUFFER_SIZE", verdict: Verdict::Refuse(Refusal::KernelOnly) },
        ],
    },
    IngressRow {
        id: 0x20802a0a,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_CE_GET_ALL_CAPS", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x20802a0b,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_CE_GET_ALL_PHYSICAL_CAPS", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20802a0d,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 5, name: "NV2080_CTRL_CMD_CE_UPDATE_PCE_LCE_MAPPINGS_V2", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 6, last: 29, name: "NV2080_CTRL_CMD_CE_UPDATE_PCE_LCE_MAPPINGS_V2", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x20802a0f,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 5, name: "NV2080_CTRL_CMD_INTERNAL_CE_GET_PCE_CONFIG_FOR_LCE_TYPE", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 6, last: 29, name: "NV2080_CTRL_CMD_INTERNAL_CE_GET_PCE_CONFIG_FOR_LCE_TYPE", verdict: Verdict::Refuse(Refusal::Internal) },
        ],
    },
    IngressRow {
        id: 0x20802a12,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 6, name: "NV2080_CTRL_CMD_CE_IS_DECOMP_LCE_ENABLED", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 7, last: 29, name: "NV2080_CTRL_CMD_CE_IS_DECOMP_LCE_ENABLED", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x20803039,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_NVLINK_SYNC_LINK_MASKS_AND_VBIOS_INFO", verdict: Verdict::Refuse(Refusal::KernelOnly) },
        ],
    },
    IngressRow {
        id: 0x20803083,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_NVLINK_GET_PLATFORM_INFO", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x20803088,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_NVLINK_GET_LOCAL_DEVICE_INFO", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x20803400,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_ECC_GET_CLIENT_EXPOSED_COUNTERS", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x20803401,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 1, name: "NV2080_CTRL_CMD_ECC_GET_VOLATILE_COUNTS", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
            Run { first: 2, last: 6, name: "NV2080_CTRL_CMD_ECC_GET_ECI_COUNTERS", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
            Run { first: 7, last: 29, name: "NV2080_CTRL_CMD_ECC_GET_VOLATILE_COUNTS", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x20803404,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_ECC_GET_REPAIR_STATUS", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x20803601,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GSP_GET_FEATURES", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x20803801,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV2080_CTRL_CMD_GRMGR_GET_GR_FS_INFO", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x20808163,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "GSS_ENC_SESSION_ACQUIRE", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x20808164,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "GSS_ENC_SESSION_RELEASE", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x208081ea,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 10, name: "NV2080_CTRL_CMD_RUSD_GET_SUPPORTED_FEATURES", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 11, last: 23, name: "NV2080_CTRL_CMD_RUSD_GET_SUPPORTED_FEATURES", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
            Run { first: 24, last: 29, name: "NV2080_CTRL_CMD_RUSD_GET_SUPPORTED_FEATURES", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
        ],
    },
    IngressRow {
        id: 0x50700117,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV5070_CTRL_CMD_SET_RMFREE_FLAGS", verdict: Verdict::Refuse(Refusal::KernelOnly) },
        ],
    },
    IngressRow {
        id: 0x50700118,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV5070_CTRL_CMD_IMP_SET_GET_PARAMETER", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::DisplayEngine)) },
        ],
    },
    IngressRow {
        id: 0x5070020c,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV5070_CTRL_CMD_GET_RG_SCAN_LINE", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::DisplayEngine)) },
        ],
    },
    IngressRow {
        id: 0x5070020d,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV5070_CTRL_CMD_GET_FRAMELOCK_HEADER_LOCKPINS", verdict: Verdict::Refuse(Refusal::KernelOnly) },
        ],
    },
    IngressRow {
        id: 0x50700709,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV5070_CTRL_CMD_SYSTEM_GET_CAPS_V2", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::DisplayEngine)) },
        ],
    },
    IngressRow {
        id: 0x50800101,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 14, name: "NV5080_CTRL_CMD_DEFERRED_API", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::NotValidatedAtTag)) },
            Run { first: 15, last: 29, name: "NV5080_CTRL_CMD_DEFERRED_API", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x50800102,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 14, name: "NV5080_CTRL_CMD_REMOVE_API", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::NotValidatedAtTag)) },
            Run { first: 15, last: 29, name: "NV5080_CTRL_CMD_REMOVE_API", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x50800103,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 14, name: "NV5080_CTRL_CMD_DEFERRED_API_V2", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::NotValidatedAtTag)) },
            Run { first: 15, last: 29, name: "NV5080_CTRL_CMD_DEFERRED_API_V2", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x50800104,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 4, name: "NV5080_CTRL_CMD_DEFERRED_API_INTERNAL", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 5, last: 14, name: "NV5080_CTRL_CMD_DEFERRED_API_INTERNAL", verdict: Verdict::Refuse(Refusal::Internal) },
            Run { first: 15, last: 29, name: "NV5080_CTRL_CMD_DEFERRED_API_INTERNAL", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x83de0307,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV83DE_CTRL_CMD_DEBUG_SET_MODE_MMU_DEBUG", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::SmDebugger)) },
        ],
    },
    IngressRow {
        id: 0x83de0309,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV83DE_CTRL_CMD_DEBUG_SET_EXCEPTION_MASK", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x83de0317,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV83DE_CTRL_CMD_DEBUG_SUSPEND_CONTEXT", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::SmDebugger)) },
        ],
    },
    IngressRow {
        id: 0x83de0318,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV83DE_CTRL_CMD_DEBUG_RESUME_CONTEXT", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::SmDebugger)) },
        ],
    },
    IngressRow {
        id: 0x906f0101,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV906F_CTRL_GET_CLASS_ENGINEID", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x906f0102,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV906F_CTRL_CMD_RESET_CHANNEL", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x906f0106,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV906F_CTRL_CMD_GET_MMU_FAULT_INFO", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x90960101,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV9096_CTRL_CMD_SET_ZBC_COLOR_CLEAR", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x90960102,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV9096_CTRL_CMD_SET_ZBC_DEPTH_CLEAR", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::AnsweredInChain)) },
        ],
    },
    IngressRow {
        id: 0x90960103,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV9096_CTRL_CMD_GET_ZBC_CLEAR_TABLE", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::AnsweredInChain)) },
        ],
    },
    IngressRow {
        id: 0x90960104,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV9096_CTRL_CMD_SET_ZBC_CLEAR_TABLE", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::AnsweredInChain)) },
        ],
    },
    IngressRow {
        id: 0x90960105,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV9096_CTRL_CMD_SET_ZBC_STENCIL_CLEAR", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::AnsweredInChain)) },
        ],
    },
    IngressRow {
        id: 0x90960106,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NV9096_CTRL_CMD_GET_ZBC_CLEAR_TABLE_SIZE", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x90960107,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV9096_CTRL_CMD_GET_ZBC_CLEAR_TABLE_ENTRY", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x90e70113,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV90E7_CTRL_CMD_BBX_GET_LAST_FLUSH_TIME", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x90f10106,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0xa06c0101,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NVA06C_CTRL_CMD_GPFIFO_SCHEDULE", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0xa06c0102,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NVA06C_CTRL_CMD_BIND", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::AnsweredInChain)) },
        ],
    },
    IngressRow {
        id: 0xa06c0103,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NVA06C_CTRL_CMD_SET_TIMESLICE", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0xa06c0105,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NVA06C_CTRL_CMD_PREEMPT", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0xa06c010a,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVA06C_CTRL_CMD_INTERNAL_PROMOTE_FAULT_METHOD_BUFFERS", verdict: Verdict::Refuse(Refusal::KernelOnly) },
        ],
    },
    IngressRow {
        id: 0xa06f0103,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVA06F_CTRL_CMD_GPFIFO_SCHEDULE", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0xa06f0104,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NVA06F_CTRL_CMD_BIND", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0xa06f0108,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVA06F_CTRL_CMD_SET_ERROR_NOTIFIER", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::AnsweredInChain)) },
        ],
    },
    IngressRow {
        id: 0xa06f0109,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVA06F_CTRL_CMD_SET_INTERLEAVE_LEVEL", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::AnsweredInChain)) },
        ],
    },
    IngressRow {
        id: 0xa06f0111,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVA06F_CTRL_CMD_RESTART_RUNLIST", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::AnsweredInChain)) },
        ],
    },
    IngressRow {
        id: 0xa06f0112,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVA06F_CTRL_CMD_STOP_CHANNEL", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0xb0cc0105,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NVB0CC_CTRL_CMD_ALLOC_PMA_STREAM", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::Profiler)) },
        ],
    },
    IngressRow {
        id: 0xb0cc010a,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NVB0CC_CTRL_CMD_EXEC_REG_OPS", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::RegisterAccess)) },
        ],
    },
    IngressRow {
        id: 0xc36f0108,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0xc3700101,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NVC370_CTRL_CMD_IDLE_CHANNEL", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::DisplayEngine)) },
        ],
    },
    IngressRow {
        id: 0xc3700102,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NVC370_CTRL_CMD_SET_ACCL", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::DisplayEngine)) },
        ],
    },
    IngressRow {
        id: 0xc3700103,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 29, name: "NVC370_CTRL_CMD_GET_ACCL", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::DisplayEngine)) },
        ],
    },
    IngressRow {
        id: 0xc3700104,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVC370_CTRL_CMD_GET_CHANNEL_INFO", verdict: Verdict::Refuse(Refusal::KernelOnly) },
        ],
    },
    IngressRow {
        id: 0xc3700201,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVC370_CTRL_CMD_GET_LOCKPINS_CAPS", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::DisplayEngine)) },
        ],
    },
    IngressRow {
        id: 0xc3700202,
        forwarded: true,
        runs: &[
            Run { first: 0, last: 24, name: "NVC370_CTRL_CMD_SET_SWAPRDY_GPIO_WAR", verdict: Verdict::Refuse(Refusal::KernelOnly) },
            Run { first: 25, last: 29, name: "NVC370_CTRL_CMD_SET_SWAPRDY_GPIO_WAR", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
        ],
    },
    IngressRow {
        id: 0xc3720101,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVC372_CTRL_CMD_IS_MODE_POSSIBLE", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::DisplayEngine)) },
        ],
    },
];

/// Every class kayfabe serves (or a rule row names), sorted by id; one run per span of equal verdicts.
pub static CLASSES: &[IngressRow] = &[
    IngressRow {
        id: 0x00000000,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV01_ROOT", verdict: Verdict::Allow(Basis::OgkmClientRoot) },
        ],
    },
    IngressRow {
        id: 0x00000001,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV01_ROOT_NON_PRIV", verdict: Verdict::Allow(Basis::OgkmClientRoot) },
        ],
    },
    IngressRow {
        id: 0x00000002,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV01_CONTEXT_DMA", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x00000004,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV01_TIMER", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x00000005,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV01_EVENT", verdict: Verdict::Allow(Basis::Empirical) },
        ],
    },
    IngressRow {
        id: 0x0000000f,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "FABRIC_MANAGER_SESSION", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x00000020,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0020_GPU_MANAGEMENT", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x0000003e,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV01_MEMORY_SYSTEM", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000003f,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV01_MEMORY_LOCAL_PRIVILEGED", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::CallerMemory)) },
        ],
    },
    IngressRow {
        id: 0x00000040,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV01_MEMORY_LOCAL_USER", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x00000041,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV01_ROOT_CLIENT", verdict: Verdict::Allow(Basis::OgkmClientRoot) },
        ],
    },
    IngressRow {
        id: 0x00000060,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV0060_SYNC_GPU_BOOST", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x00000070,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV01_MEMORY_VIRTUAL", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x00000071,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV01_MEMORY_SYSTEM_OS_DESCRIPTOR", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::CallerMemory)) },
        ],
    },
    IngressRow {
        id: 0x00000073,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV04_DISPLAY_COMMON", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x00000078,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 14, name: "NV01_EVENT_KERNEL_CALLBACK", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::NotValidatedAtTag)) },
            Run { first: 15, last: 29, name: "NV01_EVENT_KERNEL_CALLBACK", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x00000079,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV01_EVENT_OS_EVENT", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000007e,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV01_EVENT_KERNEL_CALLBACK_EX", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x00000080,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV01_DEVICE_0", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x00000083,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV01_MEMORY_LIST_OBJECT", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x00000090,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "KERNEL_GRAPHICS_CONTEXT", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::EngineNotModelled)) },
        ],
    },
    IngressRow {
        id: 0x000000de,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "RM_USER_SHARED_DATA", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x00000100,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "LOCK_STRESS_OBJECT", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::HostScope)) },
        ],
    },
    IngressRow {
        id: 0x00002080,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV20_SUBDEVICE_0", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x00002081,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV2081_BINAPI", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000402c,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV40_I2C", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::NoBoardBus)) },
        ],
    },
    IngressRow {
        id: 0x00005080,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 14, name: "NV50_DEFERRED_API_CLASS", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::NotValidatedAtTag)) },
            Run { first: 15, last: 29, name: "NV50_DEFERRED_API_CLASS", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x000050a0,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NV50_MEMORY_VIRTUAL", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x000083de,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "GT200_DEBUGGER", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000902d,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "FERMI_TWOD_A", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x00009067,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "FERMI_CONTEXT_SHARE_A", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x00009072,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "GF100_DISP_SW", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x00009096,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "GF100_ZBC_CLEAR", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x000090f1,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "FERMI_VASPACE_A", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000a06c,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "KEPLER_CHANNEL_GROUP_A", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000a140,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "KEPLER_INLINE_TO_MEMORY_B", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000b4b7,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVB4B7_VIDEO_ENCODER", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::EngineNotModelled)) },
        ],
    },
    IngressRow {
        id: 0x0000b8b0,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVB8B0_VIDEO_DECODER", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000b8fa,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVB8FA_VIDEO_OFA", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000c076,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "GP100_UVM_SW", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x0000c361,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "VOLTA_USERMODE_A", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000c36f,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "VOLTA_CHANNEL_GPFIFO_A", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::EngineNotModelled)) },
        ],
    },
    IngressRow {
        id: 0x0000c371,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVC371_DISP_SF_USER", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x0000c372,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVC372_DISPLAY_SW", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x0000c461,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "TURING_USERMODE_A", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000c46f,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "TURING_CHANNEL_GPFIFO_A", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000c4b0,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVC4B0_VIDEO_DECODER", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000c4b7,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVC4B7_VIDEO_ENCODER", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000c561,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "AMPERE_USERMODE_A", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::EngineNotModelled)) },
        ],
    },
    IngressRow {
        id: 0x0000c56f,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "AMPERE_CHANNEL_GPFIFO_A", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000c570,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVC570_DISPLAY", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x0000c573,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVC573_DISP_CAPABILITIES", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x0000c574,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "UVM_CHANNEL_RETAINER", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x0000c57a,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVC57A_CURSOR_IMM_CHANNEL_PIO", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x0000c57b,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVC57B_WINDOW_IMM_CHANNEL_DMA", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x0000c57d,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVC57D_CORE_CHANNEL_DMA", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x0000c57e,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVC57E_WINDOW_CHANNEL_DMA", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x0000c597,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "TURING_A", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000c5b5,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "TURING_DMA_COPY_A", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000c5c0,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "TURING_COMPUTE_A", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000c661,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "HOPPER_USERMODE_A", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000c670,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVC670_DISPLAY", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x0000c671,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVC671_DISP_SF_USER", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x0000c673,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVC673_DISP_CAPABILITIES", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x0000c67a,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVC67A_CURSOR_IMM_CHANNEL_PIO", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x0000c67b,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVC67B_WINDOW_IMM_CHANNEL_DMA", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x0000c67d,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVC67D_CORE_CHANNEL_DMA", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x0000c67e,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVC67E_WINDOW_CHANNEL_DMA", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x0000c697,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "AMPERE_A", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000c6b0,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVC6B0_VIDEO_DECODER", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000c6b5,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "AMPERE_DMA_COPY_A", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000c6c0,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "AMPERE_COMPUTE_A", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000c6fa,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVC6FA_VIDEO_OFA", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000c761,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 7, name: "BLACKWELL_USERMODE_A", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 8, last: 29, name: "BLACKWELL_USERMODE_A", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000c770,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVC770_DISPLAY", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x0000c771,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVC771_DISP_SF_USER", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x0000c773,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVC773_DISP_CAPABILITIES", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x0000c77d,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVC77D_CORE_CHANNEL_DMA", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x0000c77f,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVC77F_ANY_CHANNEL_DMA", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x0000c797,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "AMPERE_B", verdict: Verdict::Allow(Basis::Empirical) },
        ],
    },
    IngressRow {
        id: 0x0000c7b0,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVC7B0_VIDEO_DECODER", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000c7b5,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "AMPERE_DMA_COPY_B", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000c7b7,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVC7B7_VIDEO_ENCODER", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000c7c0,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "AMPERE_COMPUTE_B", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000c7fa,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVC7FA_VIDEO_OFA", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000c86f,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "HOPPER_CHANNEL_GPFIFO_A", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000c8b5,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "HOPPER_DMA_COPY_A", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000c96f,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 5, name: "BLACKWELL_CHANNEL_GPFIFO_A", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 6, last: 29, name: "BLACKWELL_CHANNEL_GPFIFO_A", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000c997,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "ADA_A", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000c9b0,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVC9B0_VIDEO_DECODER", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000c9b5,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 5, name: "BLACKWELL_DMA_COPY_A", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 6, last: 29, name: "BLACKWELL_DMA_COPY_A", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000c9b7,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVC9B7_VIDEO_ENCODER", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000c9c0,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "ADA_COMPUTE_A", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000c9fa,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVC9FA_VIDEO_OFA", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000ca6f,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 7, name: "BLACKWELL_CHANNEL_GPFIFO_B", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 8, last: 29, name: "BLACKWELL_CHANNEL_GPFIFO_B", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000ca70,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVCA70_DISPLAY", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x0000ca71,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 7, name: "NVCA71_DISP_SF_USER", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 8, last: 29, name: "NVCA71_DISP_SF_USER", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x0000ca73,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 7, name: "NVCA73_DISP_CAPABILITIES", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 8, last: 29, name: "NVCA73_DISP_CAPABILITIES", verdict: Verdict::Refuse(Refusal::Privileged) },
        ],
    },
    IngressRow {
        id: 0x0000ca7a,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVCA7A_CURSOR_IMM_CHANNEL_PIO", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x0000ca7b,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVCA7B_WINDOW_IMM_CHANNEL_DMA", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x0000ca7d,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVCA7D_CORE_CHANNEL_DMA", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x0000ca7e,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVCA7E_WINDOW_CHANNEL_DMA", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x0000cab5,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 7, name: "BLACKWELL_DMA_COPY_B", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 8, last: 29, name: "BLACKWELL_DMA_COPY_B", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000cb97,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "HOPPER_A", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000cbc0,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "HOPPER_COMPUTE_A", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000cd40,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 5, name: "BLACKWELL_INLINE_TO_MEMORY_A", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 6, last: 29, name: "BLACKWELL_INLINE_TO_MEMORY_A", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000cd97,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 5, name: "BLACKWELL_A", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 6, last: 29, name: "BLACKWELL_A", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000cdb0,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 5, name: "NVCDB0_VIDEO_DECODER", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 6, last: 29, name: "NVCDB0_VIDEO_DECODER", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000cdc0,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 5, name: "BLACKWELL_COMPUTE_A", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 6, last: 29, name: "BLACKWELL_COMPUTE_A", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000cdfa,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 5, name: "NVCDFA_VIDEO_OFA", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 6, last: 29, name: "NVCDFA_VIDEO_OFA", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000ce97,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 7, name: "BLACKWELL_B", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 8, last: 29, name: "BLACKWELL_B", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000ceb0,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 14, name: "NVCEB0_VIDEO_DECODER", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 15, last: 29, name: "NVCEB0_VIDEO_DECODER", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x0000ceb7,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 14, name: "NVCEB7_VIDEO_ENCODER", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 15, last: 29, name: "NVCEB7_VIDEO_ENCODER", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000cec0,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 7, name: "BLACKWELL_COMPUTE_B", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 8, last: 29, name: "BLACKWELL_COMPUTE_B", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000cefa,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVCEFA_VIDEO_OFA", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::EngineNotModelled)) },
        ],
    },
    IngressRow {
        id: 0x0000cfb0,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 7, name: "NVCFB0_VIDEO_DECODER", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 8, last: 29, name: "NVCFB0_VIDEO_DECODER", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x0000cfb7,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 7, name: "NVCFB7_VIDEO_ENCODER", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 8, last: 29, name: "NVCFB7_VIDEO_ENCODER", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000cffa,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 7, name: "NVCFFA_VIDEO_OFA", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 8, last: 29, name: "NVCFFA_VIDEO_OFA", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x0000d1b0,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 14, name: "NVD1B0_VIDEO_DECODER", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 15, last: 29, name: "NVD1B0_VIDEO_DECODER", verdict: Verdict::Allow(Basis::Mode2Rpc) },
        ],
    },
    IngressRow {
        id: 0x0000d1b7,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 14, name: "NVD1B7_VIDEO_ENCODER", verdict: Verdict::Refuse(Refusal::AbsentAtTag) },
            Run { first: 15, last: 29, name: "NVD1B7_VIDEO_ENCODER", verdict: Verdict::Allow(Basis::OgkmNonPrivileged) },
        ],
    },
    IngressRow {
        id: 0x0000d1fa,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVD1FA_VIDEO_OFA", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::EngineNotModelled)) },
        ],
    },
    IngressRow {
        id: 0x0000d2b0,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVD2B0_VIDEO_DECODER", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::EngineNotModelled)) },
        ],
    },
    IngressRow {
        id: 0x0000d2fa,
        forwarded: false,
        runs: &[
            Run { first: 0, last: 29, name: "NVD2FA_VIDEO_OFA", verdict: Verdict::Refuse(Refusal::Judgement(Judgement::EngineNotModelled)) },
        ],
    },
];
