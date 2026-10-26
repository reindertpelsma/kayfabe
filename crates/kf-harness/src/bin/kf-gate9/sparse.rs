//! Independent leaf fixtures; compare sparse transitions with the Rust diff model.

use super::*;
use kf_cuda::abi::{KfFormat, RF_PS};
use kf_cuda::synth::{Image, big_pde, pde, pte, ver3};

const SPARSE: u32 = 1 << 7;

fn tree(v3: bool, big: bool, raw: u64, stale_small: bool) -> (Image, u64) {
    let mut img = Image::at(PT_A, PT_BYTES);
    let root = img.alloc(if v3 { 16 } else { 32 }, 4096);
    let mut parent = root;
    for _ in 0..if v3 { 4 } else { 3 } {
        let next = img.alloc(4096, 4096);
        img.put64(parent, if v3 { ver3::pde(next) } else { pde(next) });
        parent = next;
    }
    let bp = img.alloc(256, 4096);
    let sp = img.alloc(4096, 4096);
    img.put64(parent, if v3 { ver3::pde(bp) } else { big_pde(bp) });
    img.put64(parent + 8, if v3 { ver3::pde(sp) } else { pde(sp) });
    for i in 0..2 {
        let word = if raw & 1 != 0 { raw + i * 0x10000 } else { raw };
        img.put64(if big { bp } else { sp } + i * 8, word);
        if big && stale_small {
            img.put64(sp + i * 128, if v3 { ver3::pte(DATA) } else { pte(DATA) });
        }
    }
    (img, root)
}

pub(super) fn check(l: &mut Checks) -> Result<(), String> {
    for v3 in [false, true] {
        for big in [false, true] {
            for enabled in [false, true] {
                scenario(l, v3, big, enabled)?;
            }
        }
    }
    Ok(())
}

fn scenario(l: &mut Checks, v3: bool, big: bool, enabled: bool) -> Result<(), String> {
    let mut fmt = if v3 {
        kf_format_ver3()
    } else {
        kf_format_ver2()
    };
    // ABI 6 padding reserved for the ABI 7 default-OFF sparse switch.
    fmt.pad0[0] = u8::from(enabled);
    let cfg = WalkCfg {
        table_version: fmt.table_version,
        ..WalkCfg::default()
    };
    let mut k = WalkKernel::bring_up_on(
        cfg,
        fmt,
        kf_cuda::walk::WalkDevice::PciBusId(&kf_harness::gate_bdf()?),
    )
    .map_err(|e| e.to_string())?;
    let img = k.upload(&vec![0; IMG_BYTES]).map_err(|e| e.to_string())?;
    let ps = if big { 0x10000 } else { PAGE };
    let class = RF_PS.put(u32::from(if big { fmt.big_ps } else { fmt.small_ps }));
    let valid = if v3 { ver3::pte(DATA) } else { pte(DATA) };
    let mut model = Committed::default();
    for (step, raw) in [valid, 8, 8, 0, 8, valid, 0x28, 0].into_iter().enumerate() {
        let (t, root) = tree(v3, big, raw, enabled && big && raw == 8);
        k.write_image(&img, t.origin, &t.mem[..t.used()])
            .map_err(|e| e.to_string())?;
        let want = wanted(fmt, raw, ps, class, enabled);
        let expected = diffmodel::diff(&model, &want, cfg.runs_per_pdb as usize);
        let got = k
            .refresh_image(&img, &[WalkEntry { pdb: root, slot: 0 }])
            .map_err(|e| e.to_string())?;
        got.validate().map_err(|e| e.to_string())?;
        let matches = got.runs.iter().map(key).collect::<Vec<_>>()
            == expected.runs.iter().map(key).collect::<Vec<_>>();
        l.check(
            if big {
                "big_sparse_diff"
            } else {
                "small_sparse_diff"
            },
            matches && got.header.refusals == 0 && !got.truncated(),
            format!(
                "v3={v3} big={big} enabled={enabled} step={step} raw={raw:#x} \
                     got={:?} want={:?}",
                got.runs, expected.runs
            ),
        );
        model = diffmodel::commit(
            &model,
            &expected.runs,
            &vec![AckCode::Applied; expected.runs.len()],
        );
        k.ack(
            got.header.generation,
            vec![KFWR_ACK_APPLIED; got.runs.len()],
        )
        .map_err(|e| e.to_string())?;
    }
    k.release(img);
    Ok(())
}

fn wanted(fmt: KfFormat, raw: u64, ps: u64, class: u32, enabled: bool) -> Vec<KfMapRun> {
    let sparse = kf_mem::sparse::classify_leaf(&fmt, raw) == kf_mem::sparse::LeafState::Sparse;
    if raw & 1 == 0 && !(enabled && sparse) {
        return vec![];
    }
    (0..2)
        .map(|i| KfMapRun {
            va: i * ps,
            gpga: if sparse { 0 } else { DATA + i * 0x10000 },
            len: ps,
            flags: class | if sparse { SPARSE } else { 0 },
            op: KFWR_OP_MAP,
            pdb_index: 0,
        })
        .collect()
}
