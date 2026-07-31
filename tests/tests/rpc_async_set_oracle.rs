//! ★★★ **GSP-D3 — the "no reply" set, DERIVED FROM THE DRIVER instead of from us.**
//!
//! [`kayfabe_gsp::RpcFunction::disposition`] answers `NoReply` for the RPCs the guest
//! issues and never awaits. Posting a reply to one of those surfaces in the driver as an
//! **unsolicited message**, which its bootup poll answers with `NV_ASSERT(0)`
//! (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:1464-1482`), and outside the
//! bootup window it desynchronises the reply stream the guest is matching on
//! `(function, sequence)`.
//!
//! The set used to be two entries — `GSP_SET_SYSTEM_INFO` (72) and `SET_REGISTRY` (73) —
//! and both are real. It was **short by one**. `ECC_NOTIFIER_WRITE_ACK` (202) is a third
//! `_issueRpcAsync` sender, and nothing would have found it, because the set was written
//! by reading our own code.
//!
//! ## ⊘ Why this is a scan of `rpc.c` and not a longer literal list
//!
//! A hand-written list of the driver's asynchronous RPCs is a *transcription*, and this
//! repository's most-repeated defect is the transcription that goes stale in silence: it
//! cannot detect a shared misreading, and shortening it weakens the claim with **zero red
//! tests**. So this test asks the driver. It finds every call site of `_issueRpcAsync` and
//! `_issueRpcAsyncLarge`, walks back from each to the `rpcWriteCommonHeader` that set the
//! function id, resolves the name through `rpc_global_enums.h`'s own X-macro table, and
//! requires the result to be **exactly** our `NoReply` set — no more and no fewer.
//!
//! ★ The failure polarity is deliberate and it is two-sided:
//!
//! - a function the driver sends asynchronously that we would answer → we desync a real
//!   guest;
//! - a function we refuse to answer that the driver **does** await → we hang it.
//!
//! Both are red here, which is what makes this an oracle rather than a checklist.
//!
//! ## ⚠ What it cannot see
//!
//! `_issueRpcAsync` is `static` in `rpc.c`, so the file is the whole universe of call sites
//! at both vendored tags — checked, not assumed, by
//! [`the_async_senders_live_only_in_rpc_c`]. But a driver that dispatched asynchronously
//! through a function pointer, or that awaited a reply somewhere other than
//! `_issueRpcAndWait`, would be invisible to a text scan. This raises the cost of the wrong
//! set; it does not make it impossible.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use kayfabe_gsp::{Disposition, RpcFunction};

/// The vendored trees, on the same terms as `vbios_real_parser_oracle.rs`: relocatable,
/// and a **loud** skip rather than a substitute when one is absent.
fn trees() -> Vec<(&'static str, PathBuf)> {
    [
        (
            "ogkm-580",
            std::env::var("KAYFABE_OGKM_580").unwrap_or_else(|_| {
                "/workspace/nvidia-gpu-passthrough/research_clones/ogkm-580.159.04".into()
            }),
        ),
        (
            "ogkm-610",
            std::env::var("KAYFABE_OGKM_610")
                .unwrap_or_else(|_| "/workspace/nvidia-gpu-passthrough/research_clones/ogkm".into()),
        ),
    ]
    .into_iter()
    .map(|(tag, p)| (tag, PathBuf::from(p)))
    .filter(|(_, p)| p.join(RPC_C).is_file() && p.join(RPC_ENUMS_H).is_file())
    .collect()
}

const RPC_C: &str = "src/nvidia/src/kernel/vgpu/rpc.c";
const RPC_ENUMS_H: &str = "src/nvidia/inc/kernel/vgpu/rpc_global_enums.h";

/// Announce both arms, so a skip is a record rather than a silence.
fn report(test: &str, tags: &[(&'static str, PathBuf)]) {
    let mut err = std::io::stderr();
    let _ = if tags.is_empty() {
        writeln!(
            err,
            "RPC-ASYNC-ORACLE: SKIPPED {test} — no vendored open-kernel-modules tree (set \
             KAYFABE_OGKM_580 / KAYFABE_OGKM_610). The test asserts NOTHING; this line is \
             the only record that it did not run."
        )
    } else {
        let names: Vec<&str> = tags.iter().map(|(t, _)| *t).collect();
        writeln!(err, "RPC-ASYNC-ORACLE: RAN {test} against {names:?}")
    };
}

/// `X(RM, NAME, id)` from `rpc_global_enums.h`, as `NAME -> id`.
///
/// The header's own rule is that ids are never reused (`:4` at both tags), so this map is
/// the driver's whole function-id namespace and nothing here needs a version key.
fn enum_table(root: &Path) -> BTreeMap<String, u32> {
    let text = std::fs::read_to_string(root.join(RPC_ENUMS_H)).expect("the X-macro header");
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let l = line.trim();
        let Some(rest) = l.strip_prefix("X(RM,") else {
            continue;
        };
        let Some((name, tail)) = rest.split_once(',') else {
            continue;
        };
        let id = tail.trim().trim_end_matches(')').trim();
        if let Ok(v) = id.parse::<u32>() {
            out.insert(name.trim().to_string(), v);
        }
    }
    assert!(out.len() > 100, "the X-macro table did not parse: {out:?}");
    out
}

/// The function ids the driver sends **without awaiting a reply**, derived from `rpc.c`.
///
/// Returns `(ids, call_sites)`: the ids, and how many call sites produced them. The count
/// is reported so a scan that silently stopped matching cannot pass as "the set is
/// unchanged".
fn async_ids(root: &Path) -> (BTreeSet<u32>, usize) {
    let text = std::fs::read_to_string(root.join(RPC_C)).expect("rpc.c");
    let table = enum_table(root);

    let mut ids = BTreeSet::new();
    let mut sites = 0usize;
    let mut at = 0usize;
    for (n, line) in text.lines().enumerate() {
        let here = at;
        at += line.len() + 1;
        // A CALL, not the definition: the definitions are `static NV_STATUS
        // _issueRpcAsync(OBJGPU *pGpu, ...)` and are excluded by requiring an assignment.
        let is_call = (line.contains("_issueRpcAsync(") || line.contains("_issueRpcAsyncLarge("))
            && line.contains('=')
            && !line.contains("static");
        if !is_call {
            continue;
        }
        sites += 1;
        let at = here;

        // ★ The enclosing function, bounded by the previous `}` in column 0 rather than by
        // a line count. A count is a tuning knob that silently starts reading the previous
        // function's header the day a body grows past it — and `rpcGspSetSystemInfo` is
        // already ~190 lines of struct filling, which is how the first draft of this scan
        // failed.
        let body_start = text[..at].rfind("\n}").map_or(0, |x| x + 2);
        let body = &text[body_start..at];

        // ★★ Searched over the SLICE, not line by line: `rpcWriteCommonHeader(pGpu, pRpc,`
        // wraps before its function-id argument at `ogkm-580: rpc.c:10521`, so a
        // line-oriented match finds the call and misses the constant. That is exactly the
        // kind of near-miss that would have made this oracle quietly incomplete.
        let hdr = body.rfind("rpcWriteCommonHeader").unwrap_or_else(|| {
            panic!("an async call site at {RPC_C}:{} names no function", n + 1)
        });
        let tail = &body[hdr..];
        let k = tail
            .find("NV_VGPU_MSG_FUNCTION_")
            .unwrap_or_else(|| panic!("{RPC_C}:{}: no function id after the header", n + 1))
            + "NV_VGPU_MSG_FUNCTION_".len();
        let rest = &tail[k..];
        let stop = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        let name = &rest[..stop];
        let id = *table
            .get(name)
            .unwrap_or_else(|| panic!("{name} is not in the X-macro table"));
        ids.insert(id);
    }
    (ids, sites)
}

/// Our side: every id whose [`Disposition`] is `NoReply`, taken from the production ABI
/// table rather than from a literal.
///
/// ⊘ Quantified over the driver's **whole** id namespace, not over a list of ours: that is
/// what makes "we answer something the driver does not await" detectable rather than
/// merely "we fail to answer something we already knew about".
fn our_no_reply(root: &Path) -> BTreeSet<u32> {
    let codes = kayfabe_device::abi::FUNCTIONS;
    enum_table(root)
        .values()
        .copied()
        .filter(|id| codes.classify(*id).disposition() == Disposition::NoReply)
        .collect()
}

#[test]
fn the_no_reply_set_is_exactly_what_the_driver_sends_asynchronously() {
    let tags = trees();
    report("the_no_reply_set_is_exactly_what_the_driver_sends_asynchronously", &tags);
    if tags.is_empty() {
        return;
    }
    for (tag, root) in &tags {
        let (derived, sites) = async_ids(root);
        // ★ Non-vacuity for the SCAN, before the comparison: a scan that matched nothing
        // would agree with any set at all. Four call sites, three functions — the extra
        // one is `SET_REGISTRY`, which picks `_issueRpcAsyncLarge` or `_issueRpcAsync`
        // depending on whether the packed registry table fits one message.
        assert_eq!(sites, 4, "{tag}: the async call-site count changed");
        assert_eq!(
            derived,
            BTreeSet::from([72, 73, 202]),
            "{tag}: the driver's asynchronous senders"
        );
        assert_eq!(
            our_no_reply(root),
            derived,
            "{tag}: our NoReply set and the driver's async set have parted. A function the \
             driver sends asynchronously that we answer DESYNCS it; one we refuse to answer \
             that it awaits HANGS it."
        );
    }
}

#[test]
fn the_async_senders_live_only_in_rpc_c() {
    // The scan reads one file, so "one file is the universe" has to be a measurement and
    // not an assumption. `_issueRpcAsync` is `static`, so this is expected — and checked.
    let tags = trees();
    report("the_async_senders_live_only_in_rpc_c", &tags);
    for (tag, root) in &tags {
        let src = root.join("src");
        let mut hits = Vec::new();
        let mut stack = vec![src];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else {
                continue;
            };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "c" || x == "h")
                    && std::fs::read_to_string(&p)
                        .is_ok_and(|t| t.contains("_issueRpcAsync"))
                {
                    hits.push(p);
                }
            }
        }
        assert_eq!(
            hits.len(),
            1,
            "{tag}: `_issueRpcAsync` is named outside rpc.c: {hits:?}"
        );
        assert!(hits[0].ends_with("vgpu/rpc.c"));
    }
}

#[test]
fn the_two_awaited_dispositions_are_still_distinguishable() {
    // ⊘ A guard against the lazy fix: making everything `NoReply` would satisfy nothing
    // above by itself, but making the *classifier* collapse would. `UNLOADING_GUEST_DRIVER`
    // ends in `_issueRpcAndWait` (`ogkm-580: rpc.c:9168-9192`) and an unanswered one blocks
    // `rmmod` for the whole RPC timeout, so it must stay on the other side of the line.
    let codes = kayfabe_device::abi::FUNCTIONS;
    assert_eq!(
        codes.classify(47).disposition(),
        Disposition::ReplyRequired
    );
    assert_eq!(codes.classify(76).disposition(), Disposition::Reply);
    assert_eq!(
        codes.classify(202),
        RpcFunction::EccNotifierWriteAck,
        "the id must classify, or `disposition` never sees it"
    );
}
