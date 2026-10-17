fn main() {
    let (version,twin)=kf_abi::windows_twin::linux_twin("581.51","VK580_65-5",0).expect("exact public twin");
    assert_eq!(twin.win_cl,36_629_274);
    assert_eq!(twin.win_cl,twin.linux_cl);
    kf_abi::versions::table_for(version).expect("compiled exact ABI table");
    println!("WINDOWS58151_EXACT_IDENTITY_ACCEPTED=1 linux_tag={} shared_CL={}",twin.linux_tag,twin.win_cl);
    println!("COMPILED_ABI_TABLE_ACCEPTED=1; hardware acceptance remains UNRUN");
}
