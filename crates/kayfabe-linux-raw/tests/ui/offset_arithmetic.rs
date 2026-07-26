// §4.2.1 refusal 4 (§4.6 row 9) plus the constructive half of the rule: a `HostOffset` has
// no unchecked arithmetic (only `checked_add`, which returns a `Result`), and it does not
// convert to or from a guest-physical address in either direction. Out-of-range in a GUEST
// address space is a fault MISS=FAULT already covers; out-of-range in a HOST address space
// is a VM escape. Different consequence, so the compiler should be the one that says so.
use kayfabe_arch::ids::Gpa;
use kayfabe_linux_raw::HostOffset;

fn main() {
    let offset = HostOffset::new(0x1000);

    let _advanced = offset + 0x1000;
    let _rewound = offset - 0x800;
    let _from_guest: HostOffset = HostOffset::from(Gpa(0x1000));
    let _to_guest: Gpa = Gpa::from(offset);
}
