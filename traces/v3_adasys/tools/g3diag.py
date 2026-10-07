import re,sys
p='/root/kayfabe/crates/kf-harness/src/bin/kf-gate3.rs'
s=open(p).read()
anchor='    let sem = word(CHAN + SEM)?;\n'
diag='''    {
        // DIAG (throwaway, v3-adasys): what did the failing buffers actually receive?
        let w32 = |b: &[u8], i: usize| u32::from_le_bytes([b[4*i], b[4*i+1], b[4*i+2], b[4*i+3]]);
        let d3 = rd(DST3, BYTES as usize)?;
        let want3 = pattern(0x5A5A_0000);
        let bad3 = (0..(BYTES as usize / 4)).filter(|&i| w32(&d3, i) != w32(&want3, i)).count();
        println!("DIAG DST3 bad_words={bad3}/{} first4={:08x?} want4={:08x?}", BYTES / 4, (0..4).map(|i| w32(&d3, i)).collect::<Vec<_>>(), (0..4).map(|i| w32(&want3, i)).collect::<Vec<_>>());
        let mut rs = vec![0u8; BYTES as usize];
        ram_view.read_into(kf_linux_raw::HostOffset::new(R_SRC), &mut rs).map_err(|e| format!("{e:?}"))?;
        println!("DIAG R_SRC_cpu_readback_ok={}", rs == want3);
        let mut rdst = vec![0u8; BYTES as usize];
        ram_view.read_into(kf_linux_raw::HostOffset::new(R_DST), &mut rdst).map_err(|e| format!("{e:?}"))?;
        let wantd = pattern(0xC0DE_0000);
        let badd = (0..(BYTES as usize / 4)).filter(|&i| w32(&rdst, i) != w32(&wantd, i)).count();
        println!("DIAG R_DST bad_words={badd}/{} first4={:08x?}", BYTES / 4, (0..4).map(|i| w32(&rdst, i)).collect::<Vec<_>>());
        std::thread::sleep(std::time::Duration::from_millis(500));
        ram_view.read_into(kf_linux_raw::HostOffset::new(R_DST), &mut rdst).map_err(|e| format!("{e:?}"))?;
        let badd2 = (0..(BYTES as usize / 4)).filter(|&i| w32(&rdst, i) != w32(&wantd, i)).count();
        println!("DIAG R_DST_after_500ms bad_words={badd2}");
        // Did the engine's write land ANYWHERE in guest RAM?
        let mut all = vec![0u8; RAM_BYTES as usize];
        ram_view.read_into(kf_linux_raw::HostOffset::new(0), &mut all).map_err(|e| format!("{e:?}"))?;
        let w0 = w32(&wantd, 0);
        let hits: Vec<usize> = (0..all.len() / 4).filter(|&i| w32(&all, i) == w0).take(8).map(|i| i * 4).collect();
        println!("DIAG SRC_pattern_word0 {w0:#x} found_in_guest_ram_at={hits:x?}");
        let nz = (0..all.len() / 4).filter(|&i| w32(&all, i) != 0).count();
        println!("DIAG guest_ram_nonzero_words={nz} (R_SRC holds {} by the CPU)", BYTES / 4);
        // And the FB side: did DST3 receive any recognisable guest-RAM data?
        let fbw = w32(&d3, 0);
        println!("DIAG DST3_word0={fbw:#x} nonzero_words={}", (0..d3.len() / 4).filter(|&i| w32(&d3, i) != 0).count());
    }
'''
assert anchor in s
s=s.replace(anchor, diag+anchor,1)
open(p,'w').write(s)
print("patched")
