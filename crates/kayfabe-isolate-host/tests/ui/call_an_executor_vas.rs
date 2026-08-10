// ★★★★★ W229 row 2: nor as a call. There is no tuple constructor and no `From<HostHandle>`,
// so the second obvious spelling is a name-resolution error rather than a privacy one.
//
// ★ Two rows because they fail with DIFFERENT errors (E0451 vs E0423) and rustc reports
// only the first when both live in one file — a single-row suite would have pinned one
// spelling and silently stopped checking the other.
use kayfabe_isolate_host::rm::ExecutorVas;

fn main() {
    let _called = ExecutorVas(0xcafe_0005);
}
