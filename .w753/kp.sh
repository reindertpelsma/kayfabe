#!/usr/bin/env bash
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
