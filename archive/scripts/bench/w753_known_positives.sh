#!/usr/bin/env bash
# ★★★★★ w753 — THE KNOWN-POSITIVE PASS FOR CONSTRAINT 32's GATES.
#
# Break each new assert, watch it go RED, restore FROM A COPY.
#
# ⊘⊘ NOT `git checkout -- <file>`: that restores to the last COMMIT, not to what was
#    mutated. On 2026-09-16 it silently deleted a whole uncommitted increment and the
#    next four mutations then failed to apply, reporting compile errors that read like
#    the production code being wrong. Same class as a_guard_that_can_never_match_what_it
#    _protects: the RESTORE had a different baseline from the thing it restored.
#
# ★ Two of the 17 mutations came back GREEN, and both were the ASSERT's fault, not the
#   mutation's (KP12: a private copy still matched; KP15: the scanner widened its own
#   subject to the rest of the file). That is what this pass is for.
#
# usage: bash scripts/bench/w753_known_positives.sh   (run with a CLEAN tree)

# ★★★ Break an assert, watch it go red, restore FROM A COPY.
# ⊘ NOT `git checkout --`: that restores to the last COMMIT, not to what was mutated, and
#   it silently deleted a whole increment of uncommitted work earlier in this session.
set -u
cd /workspace/kf-w753
SAVE=/workspace/kf-w753/.w753/save
rm -rf "$SAVE"; mkdir -p "$SAVE"
save() { mkdir -p "$SAVE/$(dirname "$1")"; cp "$1" "$SAVE/$1"; }
restore() { cp "$SAVE/$1" "$1"; }
run() { CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 cargo test -p kayfabe-isolate-host "$@" 2>&1 | grep -E "^test result|^error|panicked at" | tail -2; }

RM=crates/kayfabe-isolate-host/src/rm.rs
FX=crates/kayfabe-isolate-host/src/fdcross.rs
CH=crates/kayfabe-isolate-host/src/child.rs
for f in "$RM" "$FX" "$CH"; do save "$f"; done

echo "--- KP1  handed_client_is_unforgeable: take the role BY REFERENCE, not by value"
python3 - <<'PY'
p='crates/kayfabe-isolate-host/src/rm.rs'; s=open(p).read()
i=s.index('mod handed_client {'); j=s.index('use handed_client::HandedClient;')
seg=s[i:j]; assert seg.count('_role: ScratchpadRole,')==1
open(p,'w').write(s[:i]+seg.replace('_role: ScratchpadRole,','_role: &ScratchpadRole,',1)+s[j:])
PY
run --test own_client_invariant handed_client_is_unforgeable; restore "$RM"

echo "--- KP2  the BirthClient lend arm: mirror Isolate's rule (minted_by == target)"
python3 - <<'PY'
p='crates/kayfabe-isolate-host/src/fdcross.rs'; s=open(p).read()
old="""            FdOrigin::BirthClient { .. }
                if target.proc() == crate::SCRATCHPAD_ISOLATE_PROC =>
            {
                Ok(self.fd.as_fd())
            }"""
new="""            FdOrigin::BirthClient { minted_by } if minted_by == target => Ok(self.fd.as_fd()),"""
assert s.count(old)==1
open(p,'w').write(s.replace(old,new,1))
PY
run --test fd_crossing birth_client; restore "$FX"

echo "--- KP3  serve_one: delete the per-verb fd rule"
python3 - <<'PY'
p='crates/kayfabe-isolate-host/src/child.rs'; s=open(p).read()
old="    if !fds.is_empty() && !matches!(request, Request::AdoptBirthClient { .. }) {"
new="    if false {"
assert s.count(old)==1
open(p,'w').write(s.replace(old,new,1))
PY
run --lib a_descriptor_on_a_request_that_may_not_carry_one_is_refused; restore "$CH"

echo "--- KP4  adopt_birth_client: accept an EMPTY fds by substituting descriptors"
python3 - <<'PY'
p='crates/kayfabe-isolate-host/src/child.rs'; s=open(p).read()
old="    let Ok([ctl, node]) = <[OwnedFd; 2]>::try_from(fds) else {"
new=("    let subbed = if fds.is_empty() { vec![OwnedFd::from(std::fs::File::open(\"/dev/null\").unwrap()),"
     " OwnedFd::from(std::fs::File::open(\"/dev/null\").unwrap())] } else { fds };\n"
     "    let Ok([ctl, node]) = <[OwnedFd; 2]>::try_from(subbed) else {")
assert s.count(old)==1
open(p,'w').write(s.replace(old,new,1))
PY
run --lib a_birth_client_without_its_descriptors_is_refused_by_its_own_name; restore "$CH"

echo "--- KP5  adopt_birth_client: check the SECOND descriptor against the wrong kind"
python3 - <<'PY'
p='crates/kayfabe-isolate-host/src/child.rs'; s=open(p).read()
old="        CrossedFd::adopt(node, origin, kayfabe_linux_raw::DescriptorKind::CharDevice),"
new="        CrossedFd::adopt(node, origin, kayfabe_linux_raw::DescriptorKind::RegularFile),"
assert s.count(old)==1
open(p,'w').write(s.replace(old,new,1))
PY
run --lib a_birth_client_descriptor_that_is_not_a_char_device_is_refused; restore "$CH"

echo "--- KP6  read_frame_with_fds: drop the control buffer on the length read (allowance 0)"
python3 - <<'PY'
p='crates/kayfabe-isolate-host/src/fdcross.rs'; s=open(p).read()
old="        let n = match recv_with_fds(sock, &mut len_bytes[filled..], fds, max_fds) {"
new="        let n = match recv_with_fds(sock, &mut len_bytes[filled..], fds, 0) {"
assert s.count(old)==1
open(p,'w').write(s.replace(old,new,1))
PY
run --test fd_crossing a_reader_without_a_control_buffer; restore "$FX"

echo "--- KP7  ProxyRmBackend::adopt_birth_client: drop the not-the-scratchpad refusal"
python3 - <<'PY'
p='crates/kayfabe-isolate-host/src/isolate.rs'; s=open(p).read()
assert s.count("if self.isolate.proc() != crate::SCRATCHPAD_ISOLATE_PROC {")==1
PY
echo "    (no test yet -- recorded as a GAP, see the report)"

echo "=== RESTORED? (git diff must be empty) ==="
git diff --stat crates/ | tail -3

set -u
cd /workspace/kf-w753
SAVE=.w753/save2; rm -rf "$SAVE"; mkdir -p "$SAVE/crates/kayfabe-isolate-host/src" "$SAVE/crates/kayfabe-isolate-host/tests"
RM=crates/kayfabe-isolate-host/src/rm.rs
T=crates/kayfabe-isolate-host/tests/own_client_invariant.rs
cp "$RM" "$SAVE/$RM"; cp "$T" "$SAVE/$T"
run() { CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 cargo test -p kayfabe-isolate-host --test own_client_invariant "$@" 2>&1 | grep -E "^test result" | tail -1; }

echo "--- KP8  BirthConn::dup stamps a LITERAL instead of the handed client"
python3 - <<'PY'
p='crates/kayfabe-isolate-host/src/rm.rs'; s=open(p).read()
old="""            Nvos55Parameters {
                h_client: self.handed.root(),
                h_parent: parent,"""
new="""            Nvos55Parameters {
                h_client: 0xc1d0_0000,
                h_parent: parent,"""
assert s.count(old)==1
open(p,'w').write(s.replace(old,new,1))
PY
run every_escape_in_birth_conn_stamps_the_handed_client
cp "$SAVE/$RM" "$RM"

echo "--- KP9  a THIRD NV_ESC_RM_DUP_OBJECT escape appears (outside birth_conn)"
python3 - <<'PY'
p='crates/kayfabe-isolate-host/src/rm.rs'; s=open(p).read()
m="    fn raw_unmap_dma(&self, h_dma: u32, gpu_va: u64) -> Result<(), RmError> {"
assert s.count(m)==1
inj = ("    #[allow(dead_code)]\n    fn a_third_dup(&self) -> Result<(), RmError> {\n"
       "        let _ = ioctl::readwrite(NV_IOCTL_MAGIC, NV_ESC_RM_DUP_OBJECT as u8, 4);\n        Ok(())\n    }\n\n")
open(p,'w').write(s.replace(m, inj+m,1))
PY
run there_are_exactly_two_dup_object_escapes_and_the_second_is_in_birth_conn
cp "$SAVE/$RM" "$RM"

echo "--- KP10  mod birth_conn grows an NV_ESC_RM_MAP_MEMORY escape (RM refuses it forever)"
python3 - <<'PY'
p='crates/kayfabe-isolate-host/src/rm.rs'; s=open(p).read()
m="        /// `NV_ESC_RM_FREE` inside B."
assert s.count(m)==1
inj = ("        #[allow(dead_code)]\n        fn cpu_map(&self) -> u32 {\n"
       "            let _ = NV_ESC_RM_MAP_MEMORY;\n            0\n        }\n\n")
open(p,'w').write(s.replace(m, inj+m,1))
PY
run every_escape_in_birth_conn_stamps_the_handed_client
cp "$SAVE/$RM" "$RM"

echo "--- KP11  the scoped F11 arm is made VACUOUS (birth_conn span shrunk to nothing)"
python3 - <<'PY'
p='crates/kayfabe-isolate-host/tests/own_client_invariant.rs'; s=open(p).read()
old="        let inside_birth_conn = offset > birth_start && offset < birth_end;"
new="        let inside_birth_conn = false && offset > birth_start && offset < birth_end;"
assert s.count(old)==1
open(p,'w').write(s.replace(old,new,1))
PY
run every_rm_escape_in_rm_rs_stamps_the_isolates_own_client
cp "$SAVE/$T" "$T"

echo "--- KP12  handed_client loses its minted_by accessor (constraint 32's claim unprovable)"
python3 - <<'PY'
p='crates/kayfabe-isolate-host/src/rm.rs'; s=open(p).read()
old="        pub(super) fn minted_by(self) -> u32 {"
new="        #[allow(dead_code)]\n        fn minted_by(self) -> u32 {"
assert s.count(old)==1
open(p,'w').write(s.replace(old,new,1))
PY
run handed_client_is_unforgeable
cp "$SAVE/$RM" "$RM"

echo "=== RESTORED? ==="; git diff --stat crates/ | tail -2

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
