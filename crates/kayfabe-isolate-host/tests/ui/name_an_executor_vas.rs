// ★★★★★ W229 row 1: the isolate's own address space must be UNSPELLABLE as a struct
// expression outside this crate. `ExecutorVas::range` is private, so a caller holding a
// guest `Vas`'s raw handle cannot wrap it and hand it to `alloc_channel_for_isolate`.
//
// ⊘ This is the row that matters. The type's guarantee is *"no guest channel is bound to
// this space"* — a claim about how the handle was OBTAINED — and a struct expression is
// exactly the way to assert that claim without establishing it.
use kayfabe_isolate_host::rm::ExecutorVas;

fn main() {
    let _named = ExecutorVas { range: 0xcafe_0005 };
}
