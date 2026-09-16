#!/usr/bin/env bash
set -u
cd /workspace/kf-w753
SAVE=.w753/save3; rm -rf "$SAVE"; mkdir -p "$SAVE"
RM=crates/kayfabe-isolate-host/src/rm.rs
SC=crates/kayfabe-qemu-raw/src/scratchpad.rs
cp "$RM" "$SAVE/rm.rs"; cp "$SC" "$SAVE/scratchpad.rs"
run(){ CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 cargo test "$@" 2>&1 | grep -E "^test result" | tail -1; }

echo "--- KP13 the_two_handle_spaces_cannot_collide: put BIRTH_HANDLE_BASE back above ours"
python3 -c "
p='$RM'; s=open(p).read()
old='const BIRTH_HANDLE_BASE: u32 = 0xB147_0000;'
new='const BIRTH_HANDLE_BASE: u32 = 0xCAFE_B000;'
assert s.count(old)==1
open(p,'w').write(s.replace(old,new,1))"
run -p kayfabe-isolate-host --test ownership_split_gates the_two_handle_spaces_cannot_collide
cp "$SAVE/rm.rs" "$RM"

echo "--- KP14 the_store_unmap_routes...: unmap ignores the birth index again"
python3 -c "
p='$RM'; s=open(p).read()
old='''        if let Some((birth, _)) = self.conn.birth_for_range(h_dma) {
            return birth.unmap_dma(h_dma, at.0);
        }
        self.conn.raw_unmap_dma(h_dma, at.0)'''
new='        self.conn.raw_unmap_dma(h_dma, at.0)'
assert s.count(old)==1
open(p,'w').write(s.replace(old,new,1))"
run -p kayfabe-isolate-host --test ownership_split_gates the_store_unmap_routes
cp "$SAVE/rm.rs" "$RM"

echo "--- KP15 every_nvos46_site_asserts_its_own_placement: drop the B site's teardown"
python3 -c "
p='$RM'; s=open(p).read()
old='                let _ = self.unmap_dma(h_dma, out.dma_offset);'
assert s.count(old)==1
open(p,'w').write(s.replace(old,'',1))"
run -p kayfabe-isolate-host --test fixed_placement_is_asserted every_nvos46_site
cp "$SAVE/rm.rs" "$RM"

echo "--- KP16 the_surrender_exit_has_exactly_one_call_site: a second caller"
python3 -c "
p='$RM'; s=open(p).read()
m='    fn raw_unmap_dma(&self, h_dma: u32, gpu_va: u64) -> Result<(), RmError> {'
assert s.count(m)==1
inj='    #[allow(dead_code)]\n    fn a_second_surrender(&self) -> u32 { let birth = self.client; birth.raw_for_surrender() }\n\n'
open(p,'w').write(s.replace(m, inj+m,1))"
run -p kayfabe-isolate-host --test own_client_invariant the_surrender_exit
cp "$SAVE/rm.rs" "$RM"

echo "--- KP17 bare_vaspaces_and_birth_client_are_different_questions: make scratchpad route through K"
python3 -c "
p='$SC'; s=open(p).read()
old='        matches!(self, VasOwner::BirthClient)'
new='        matches!(self, VasOwner::BirthClient | VasOwner::Scratchpad)'
assert s.count(old)==1
open(p,'w').write(s.replace(old,new,1))"
run -p kayfabe-qemu-raw --lib bare_vaspaces_and_birth_client
cp "$SAVE/scratchpad.rs" "$SC"

echo "--- KP18 route_k_is_a_third_arm...: default a typo to k instead of refusing"
python3 -c "
p='$SC'; s=open(p).read()
old='        Some(\"k\") => Ok(VasOwner::BirthClient),\n        Some(_) => Err(('
new='        Some(\"k\") => Ok(VasOwner::BirthClient),\n        Some(_) if false => Err(('
assert s.count(old)==1
s=s.replace(old,new,1)
s=s.replace('        )),\n    }\n}','        )),\n        Some(_) => Ok(VasOwner::BirthClient),\n    }\n}',1)
open(p,'w').write(s)"
run -p kayfabe-qemu-raw --lib route_k_is_a_third_arm
cp "$SAVE/scratchpad.rs" "$SC"

echo "=== RESTORED? ==="; git diff --stat crates/ | tail -2
