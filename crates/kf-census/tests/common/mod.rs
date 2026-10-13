//! Shared fixtures for the integration tests: synthetic messages encoded with kayfabe's own
//! transport encoder, written as KFGSPCAP or as the observer's JSON lines.
#![allow(dead_code)]

use kf_abi::DriverVersion;
use kf_census::analyze::{AnomalyKind, Report};
use kf_census::capture::{Dir, FileHeader, Os, Writer};
use kf_census::{analyzer_for, observer, stream};
use kf_gsp::{GspAbi, OutgoingRpc, TxHeader, encode_message};

pub const TAG: DriverVersion = DriverVersion {
    major: 580,
    minor: 82,
    patch: 7,
};
pub const ES: usize = 4096;
pub const RING: u32 = 63;

pub fn abi() -> GspAbi {
    kf_rm::abi::gsp_abi_for(TAG).expect("580.82.07 is measured")
}

/// One message to put on a queue.
#[derive(Clone)]
pub struct Msg {
    pub dir: Dir,
    pub function: u32,
    pub rpc_sequence: u32,
    pub rpc_result: u32,
    pub payload: Vec<u8>,
}

pub fn control_payload(cmd: u32, status: u32, params: &[u8]) -> Vec<u8> {
    let w = abi().driver.rm_control_wire();
    let mut p = vec![0u8; w.params_off];
    p[0..4].copy_from_slice(&0xc1d0_0001u32.to_le_bytes()); // hClient
    p[4..8].copy_from_slice(&0xc1d0_0002u32.to_le_bytes()); // hObject
    p[8..12].copy_from_slice(&cmd.to_le_bytes());
    p[w.status_off..w.status_off + 4].copy_from_slice(&status.to_le_bytes());
    p[w.params_size_off..w.params_size_off + 4]
        .copy_from_slice(&(params.len() as u32).to_le_bytes());
    p.extend_from_slice(params);
    p
}

pub fn ctl_req(cmd: u32, params: &[u8]) -> Msg {
    Msg {
        dir: Dir::Cmd,
        function: abi().rpc.codes.gsp_rm_control,
        rpc_sequence: 0,
        rpc_result: 0xffff_ffff,
        payload: control_payload(cmd, 0, params),
    }
}

pub fn ctl_rep(cmd: u32, status: u32, params: &[u8]) -> Msg {
    Msg {
        dir: Dir::Status,
        function: abi().rpc.codes.gsp_rm_control,
        rpc_sequence: 0,
        rpc_result: 0,
        payload: control_payload(cmd, status, params),
    }
}

/// Encode with kayfabe's own transport code. `seqs` gives the element sequence per message.
pub fn encode(msgs: &[Msg], seqs: &[u32]) -> Vec<Vec<u8>> {
    let a = abi();
    msgs.iter()
        .zip(seqs)
        .map(|(m, &s)| {
            encode_message(
                &a.element,
                a.rpc.header_version,
                ES as u32,
                a.element_size_max,
                s,
                &OutgoingRpc {
                    function: m.function,
                    sequence: m.rpc_sequence,
                    rpc_result: m.rpc_result,
                    rpc_result_private: 0,
                    payload: m.payload.clone(),
                },
            )
            .expect("encodes")
        })
        .collect()
}

/// Per-direction sequence numbers 0.. in order.
pub fn natural_seqs(msgs: &[Msg]) -> Vec<u32> {
    let (mut c, mut s) = (0u32, 0u32);
    msgs.iter()
        .map(|m| {
            let n = if m.dir == Dir::Cmd { &mut c } else { &mut s };
            let v = *n;
            *n += 1;
            v
        })
        .collect()
}

pub fn header() -> FileHeader {
    FileHeader {
        os: Os::Windows,
        module_version: 1,
        pci_vendor_id: 0x10de,
        pci_device_id: 0x2786,
        pci_revision_id: 0xa1,
        ts_hz: 1_000_000_000,
        ts_start: 5,
        driver_version: "580.88".to_string(),
        decode_tag: "580.82.07".to_string(),
        scrubbed: false,
    }
}

pub fn tx_header() -> [u8; 32] {
    TxHeader {
        version: 0,
        size: (RING + 1) * ES as u32,
        msg_size: ES as u32,
        msg_count: RING,
        write_ptr: 0,
        flags: 1,
        rx_hdr_off: 0x20,
        entry_off: ES as u32,
    }
    .encode()
}

/// Write runs into a capture the way a module would: whole entries, cut at the ring end and
/// at `chunk` entries, with a monotonic timestamp. `end` appends the End record.
pub fn capture(runs: &[(Dir, Vec<u8>)], chunk: u32, end: bool) -> Vec<u8> {
    let mut w = Writer::new(Vec::new(), &header()).unwrap();
    for d in [Dir::Cmd, Dir::Status] {
        w.bind(1, d, 1, &tx_header()).unwrap();
    }
    let mut next = [0u32; 2];
    let mut ts = 10u64;
    for d in [Dir::Cmd, Dir::Status] {
        let ix = usize::from(d == Dir::Status);
        let bytes: Vec<u8> = runs
            .iter()
            .filter(|(rd, _)| *rd == d)
            .flat_map(|(_, r)| r.iter().copied())
            .collect();
        let entries = (bytes.len() / ES) as u32;
        let mut done = 0u32;
        while done < entries {
            let to_end = RING - next[ix];
            let n = (entries - done).min(chunk).min(to_end);
            ts += 1;
            w.elements(
                1,
                d,
                ts,
                next[ix],
                n,
                &bytes[done as usize * ES..(done + n) as usize * ES],
            )
            .unwrap();
            next[ix] = (next[ix] + n) % RING;
            done += n;
        }
    }
    if end {
        w.end(ts + 1).unwrap()
    } else {
        w.abandon()
    }
}

pub fn run_capture(bytes: &[u8]) -> Report {
    let mut an = analyzer_for(TAG, None).unwrap();
    stream::read(&mut an, "fixture", bytes).unwrap();
    an.finish()
}

/// The same messages in the observer's JSON-lines shape (message bytes trimmed to the declared
/// length rounded to 8, metadata beside it).
pub fn observer_jsonl(
    runs: &[(Dir, Vec<u8>)],
    tamper_function: Option<u32>,
    records_in_footer: Option<usize>,
) -> String {
    let mut out = String::new();
    out.push_str("{\"schema\":\"kayfabe-gsp-text/1\",\"kind\":\"header\",\"capture_complete\":false,\"selection\":\"all\",\"observer\":\"vfio-bootstrap-580\",\"device\":\"kf3-gpu\",\"magic\":1415006027,\"version\":1,\"header_bytes\":64,\"record_header_bytes\":64,\"qpc_frequency\":1000000000,\"started_qpc\":1,\"flags\":1,\"reserved\":0,\"reserved2\":[0,0,0]}\n");
    let mut bytes_total = 64usize;
    for (d, run) in runs {
        let rd = |off: usize| u32::from_le_bytes(run[off..off + 4].try_into().unwrap());
        let len = rd(48 + 8) as usize;
        let keep = (48 + len).next_multiple_of(8);
        let p = &run[..keep];
        let hex: String = p.iter().map(|b| format!("{b:02x}")).collect();
        bytes_total += 64 + keep;
        out.push_str(&format!(
            "{{\"kind\":\"record\",\"generation\":1,\"trigger\":2,\"magic\":1380403019,\"header_bytes\":64,\"payload_bytes\":{keep},\"direction\":{},\"qpc\":7,\"table_pa\":4096,\"queue_sequence\":{},\"rpc_sequence\":{},\"rpc_function\":{},\"rpc_result\":{},\"flags\":1,\"missing_before\":0,\"rpc_version\":{},\"reserved\":0,\"payload_hex\":\"{hex}\"}}\n",
            u32::from(*d == Dir::Status),
            rd(36),
            rd(48 + 24),
            tamper_function.unwrap_or_else(|| rd(48 + 12)),
            rd(48 + 16),
            rd(48),
        ));
    }
    out.push_str(&format!(
        "{{\"kind\":\"footer\",\"file_export_complete\":true,\"capture_complete\":false,\"records\":{},\"source_records\":{},\"omitted_records\":0,\"source_bytes\":{bytes_total},\"driver_stats\":{{\"dropped\":0,\"sequence_gaps\":0}}}}\n",
        records_in_footer.unwrap_or(runs.len()),
        runs.len()
    ));
    out
}

pub fn run_observer(text: &str) -> Report {
    let mut an = analyzer_for(TAG, None).unwrap();
    observer::read(&mut an, "fixture", text.as_bytes()).unwrap();
    an.finish()
}

pub fn count(r: &Report, k: AnomalyKind) -> u64 {
    r.anomalies.get(&k).map_or(0, |a| a.count)
}

/// A small conversation: three controls, each request answered, one status error, an echo,
/// plus an alloc and an event so that non-control functions are exercised.
pub fn conversation() -> Vec<Msg> {
    let a = abi();
    vec![
        ctl_req(0x2080_852a, &[1u8; 24]),
        ctl_rep(0x2080_852a, 0, &[2u8; 1784]),
        ctl_req(0x2080_a02f, &[0u8; 1]),
        ctl_rep(0x2080_a02f, 0x56, &[0u8; 1]),
        ctl_req(0x0073_0128, &[7u8; 20]),
        ctl_rep(0x0073_0128, 0, &[7u8; 20]),
        ctl_req(0x2080_852a, &[1u8; 24]),
        ctl_rep(0x2080_852a, 0, &[2u8; 1784]),
        Msg {
            dir: Dir::Status,
            function: a.rpc.codes.post_event,
            rpc_sequence: 0,
            rpc_result: 0,
            payload: vec![9u8; 24],
        },
    ]
}

pub fn runs_of(msgs: &[Msg], seqs: &[u32]) -> Vec<(Dir, Vec<u8>)> {
    msgs.iter().map(|m| m.dir).zip(encode(msgs, seqs)).collect()
}
