fn main() {
    use kf_abi::windows_twin::{linux_twin,TwinRefusal};
    let result=linux_twin("610.88","r610_85-151",0);
    let e=result.unwrap_err();
    assert!(matches!(e,TwinRefusal::ChangelistsDiffer(t) if t.win_cl==38_566_155 && t.linux_cl==38_610_010));
    println!("PUBLIC_WINDOWS610_IDENTITY_ACCEPTED=0");
    println!("REFUSAL={e}");
    println!("NO_ABI_OVERRIDE_OR_HARDWARE_RUN=1");
}
