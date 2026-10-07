// §5.2 item 1 (§4.6 row 4): a literal cannot flow into the alignment path, because it does
// not typecheck. `HostPageSize` has a private field, no `from_bytes`, and its test-only
// `forced` constructor does not exist outside a test build — arm64 hosts run 16 KiB and
// 64 KiB pages, so a hardcoded 4096 is a misalignment, not a constant.
use kayfabe_linux_raw::HostPageSize;

fn main() {
    let _literal = HostPageSize(4096);
    let _named = HostPageSize::from_bytes(4096);
    let _converted: HostPageSize = 4096u64.into();
    let _forced = HostPageSize::forced(65536);
}
