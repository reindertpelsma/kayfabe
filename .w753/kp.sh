#!/usr/bin/env bash
# Break an assert, watch it go red, restore. Every mutation is a `sed` stated in full so
# the report can say HOW each one was broken.
set -u
cd /workspace/kf-w753
run() { CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 cargo test -p kayfabe-isolate-host "$@" 2>&1 | grep -E "^test result|FAILED|error\[" | tail -3; }

echo "=== KP1: neuter handed_over's role consumption (by value -> by reference) ==="
sed -i 's/            _role: ScratchpadRole,\n            client: u32,\n            minted_by: u32,/X/' crates/kayfabe-isolate-host/src/rm.rs
python3 - <<'PY'
p='crates/kayfabe-isolate-host/src/rm.rs'; s=open(p).read()
i=s.index('mod handed_client {'); j=s.index('use handed_client::HandedClient;')
seg=s[i:j].replace('_role: ScratchpadRole,','_role: &ScratchpadRole,',1)
open(p,'w').write(s[:i]+seg+s[j:])
PY
python3 - <<'PY'
p='crates/kayfabe-isolate-host/src/rm.rs'; s=open(p).read()
i=s.index('mod handed_client {'); j=s.index('use handed_client::HandedClient;')
assert '_role: &ScratchpadRole,' in s[i:j], "mutation did not apply"
PY
run --test own_client_invariant handed_client_is_unforgeable
git checkout -- crates/kayfabe-isolate-host/src/rm.rs

echo "=== KP2: make BirthClient lend mirror Isolate's rule (minted_by == target) ==="
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
run --test fd_crossing birth_client
git checkout -- crates/kayfabe-isolate-host/src/fdcross.rs

echo "=== KP3: drop the per-verb fd rule from serve_one ==="
python3 - <<'PY'
p='crates/kayfabe-isolate-host/src/child.rs'; s=open(p).read()
old="    if !fds.is_empty() && !matches!(request, Request::AdoptBirthClient { .. }) {"
new="    if false && !fds.is_empty() && !matches!(request, Request::AdoptBirthClient { .. }) {"
assert s.count(old)==1
open(p,'w').write(s.replace(old,new,1))
PY
run --lib a_descriptor_on_a_request_that_may_not_carry_one_is_refused
git checkout -- crates/kayfabe-isolate-host/src/child.rs

echo "=== KP4: answer a birth client with NO descriptors as if it were fine ==="
python3 - <<'PY'
p='crates/kayfabe-isolate-host/src/child.rs'; s=open(p).read()
old="    let Ok([ctl, node]) = <[OwnedFd; 2]>::try_from(fds) else {"
new="    let Ok([ctl, node]) = <[OwnedFd; 2]>::try_from(if fds.is_empty() { vec![a_char_device_for_mutation(), a_char_device_for_mutation()] } else { fds }) else {"
assert s.count(old)==1
s=s.replace(old,new,1)
s=s.replace("fn adopt_birth_client(", "fn a_char_device_for_mutation() -> OwnedFd { OwnedFd::from(std::fs::File::open(\"/dev/null\").unwrap()) }\nfn adopt_birth_client(",1)
open(p,'w').write(s)
PY
run --lib a_birth_client_without_its_descriptors_is_refused_by_its_own_name
git checkout -- crates/kayfabe-isolate-host/src/child.rs

echo "=== KP5: skip the KERNEL kind check (trust the sender's claim) ==="
python3 - <<'PY'
p='crates/kayfabe-isolate-host/src/child.rs'; s=open(p).read()
old="        CrossedFd::adopt(node, origin, kayfabe_linux_raw::DescriptorKind::CharDevice),"
new="        CrossedFd::adopt(node, origin, kayfabe_linux_raw::DescriptorKind::RegularFile),"
assert s.count(old)==1
open(p,'w').write(s.replace(old,new,1))
PY
run --lib a_birth_client_descriptor_that_is_not_a_char_device_is_refused
git checkout -- crates/kayfabe-isolate-host/src/child.rs

echo "=== KP6: reader loses its control buffer (allowance 0) ==="
python3 - <<'PY'
p='crates/kayfabe-isolate-host/src/fdcross.rs'; s=open(p).read()
old="    let mut filled = 0;\n    while filled < 4 {\n        let n = match recv_with_fds(sock, &mut len_bytes[filled..], fds, max_fds) {"
new="    let mut filled = 0;\n    while filled < 4 {\n        let n = match recv_with_fds(sock, &mut len_bytes[filled..], fds, 0) {"
assert s.count(old)==1
open(p,'w').write(s.replace(old,new,1))
PY
run --test fd_crossing a_reader_without_a_control_buffer_loses_the_descriptor_and_reports_nothing
git checkout -- crates/kayfabe-isolate-host/src/fdcross.rs
echo "=== RESTORED ==="
git status --short crates/ | head
