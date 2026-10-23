// CODE READ: test fragment for the separate Q4 input encoder, not a product-code change.
#[cfg(test)]
mod q4_probe_tests {
    use super::*;
    fn v() -> kf_abi::DriverVersion { kf_abi::DriverVersion::parse("595.84").unwrap() }
    #[test] fn q4_probe_chosen_client_is_carried_in_root_params() {
        let p=q4_probe_root_params(v(),0xc1e0_0100,1234).unwrap();
        let facts=kf_abi::versions::table_for(v()).unwrap().decode_client_alloc_facts(&p).unwrap();
        assert_eq!(facts.h_client,0xc1e0_0100);
    }
    #[test] fn q4_probe_userspace_pid_is_carried_independently() {
        let p=q4_probe_root_params(v(),0xc1e0_0100,1234).unwrap();
        let facts=kf_abi::versions::table_for(v()).unwrap().decode_client_alloc_facts(&p).unwrap();
        assert_eq!(facts.process_id,1234);
    }
    #[test] fn q4_probe_full_ioctl_layout_keeps_os_pointer_clear() {
        let p=q4_probe_root_params(v(),0xc1e0_0100,1234).unwrap();
        assert_eq!(p.len(),120);assert!(p[8..].iter().all(|b|*b==0));
    }
}
