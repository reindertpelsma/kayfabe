// ★★★★★ w323 row 2: the witness must be UNSPELLABLE as a struct expression. If it could
// be named into existence, `execute`'s new parameter would be a formality a caller
// satisfies with a literal — which is asserting the claim rather than establishing it,
// the same over-claim `ExecutorVas` had corrected one crate over.
use kayfabe_util::trapwitness::OffTrap;

fn main() {
    let _named = OffTrap {
        what: "I am definitely not on a vCPU",
        inline: false,
        _not_send: std::marker::PhantomData,
    };
}
