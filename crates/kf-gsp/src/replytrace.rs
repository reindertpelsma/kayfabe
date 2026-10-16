//! Host-enabled, bounded diagnostics of replies successfully published to the queue.
//! No guest RAM reads, host GPU operations or reply mutation. Body hex stays on the
//! rental; exports must screen fields (especially addresses) before publication.
use crate::boot::GspAbi;
use crate::element::OutgoingRpc;
use std::fmt::Write;

const MAX_RECORDS: usize = 512;
const MAX_BYTES: usize = 256 * 1024;
const MAX_BODY: usize = 4096;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct ReplyBodyTrace {
    enabled: bool,
    records: usize,
    bytes: usize,
}

impl ReplyBodyTrace {
    pub(crate) fn from_host_env() -> Self {
        Self {
            enabled: std::env::var_os("KF3_RPC_BODY_TRACE").is_some_and(|v| v == "1"),
            ..Self::default()
        }
    }

    pub(crate) fn posted(
        &mut self,
        abi: &GspAbi,
        rpc: &OutgoingRpc,
        message_seq: u32,
    ) -> Option<String> {
        if !self.enabled || self.records >= MAX_RECORDS || self.bytes >= MAX_BYTES {
            return None;
        }
        let selector = if rpc.function == abi.rpc.codes.gsp_rm_control {
            let q = abi.driver.decode_rpc_control(&rpc.payload).ok()?;
            // Public NV0073 display controls and ctrl2080internal.h's display
            // channel pushbuffer params (580.65.06). Diagnostic selection only.
            if q.cmd >> 16 != 0x0073 && q.cmd != 0x2080_0a58 {
                return None;
            }
            Some(q.cmd)
        } else if rpc.function == abi.rpc.codes.gsp_rm_alloc {
            abi.driver
                .decode_rpc_alloc(&rpc.payload)
                .ok()
                .map(|q| q.class)
        } else if rpc.function == 4 {
            // The two Other(4) calls in the measured display subtree. Observe
            // bytes only; no semantics or successful response is invented here.
            None
        } else {
            // In particular, static-info/board replies are not exported here.
            return None;
        };
        let saved = rpc.payload.len().min(MAX_BODY).min(MAX_BYTES - self.bytes);
        let mut body = String::with_capacity(saved * 2);
        for byte in &rpc.payload[..saved] {
            let _ = write!(body, "{byte:02x}");
        }
        let key = selector.map_or_else(|| "-".to_owned(), |v| format!("{v:#x}"));
        let record = self.records;
        self.records += 1;
        self.bytes += saved;
        Some(format!(
            "kf-gsp: reply-body record={record} message_seq={message_seq} function={:#x} selector={key} rpc_result={:#x} rpc_result_private={:#x} total={} saved={saved} truncated={} body={body}",
            rpc.function,
            rpc.rpc_result,
            rpc.rpc_result_private,
            rpc.payload.len(),
            usize::from(saved != rpc.payload.len()),
        ))
    }
}
