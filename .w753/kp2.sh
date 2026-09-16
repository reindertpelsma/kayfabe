#!/usr/bin/env bash
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
