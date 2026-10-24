from pathlib import Path
import subprocess, json, tarfile, hashlib, os

D = Path("/workspace/bw3-round-b1f-run")
W = Path("/workspace/bw-native-edge-wt")
O = Path("/workspace/bw-provider-retry-wt")
K = W / "traces/blackwell_ab_20261023/ROUND_B1f_0115/run_kit"
iid = int((D / "own-id").read_text())
alias = (D / "alias").read_text().strip()
conf = D / "ssh.conf"
r = next(
    x
    for x in json.loads(
        (W / "traces/blackwell_ab_20261023/ROUND_B1f_0115/owned_instances.json").read_text()
    )
    if x["id"] == iid
)
assert r["auto_destroy_verified"]
os.kill(r["auto_destroy_pid"], 0)


def remote(cmd, stdin=None, timeout=120):
    return subprocess.run(
        ["ssh", "-F", str(conf), alias, cmd],
        input=stdin,
        capture_output=True,
        text=isinstance(stdin, str) or stdin is None,
        timeout=timeout,
        check=True,
    )


inv = remote(
    "set -eu; date -u; df -h /; test -c /dev/kvm; test $(awk '/MemTotal/{print $2}' /proc/meminfo) -ge 32000000; test $(df --output=avail -k / | tail -1) -ge 110000000; test ! -e /root/sweep_controller.sh; echo KVM_DISK_PASS"
)
(D / "initial_inventory.txt").write_text(inv.stdout)
print(inv.stdout, flush=True)
files = {
    "provision_box.sh": W / "scripts/bench/provision_box.sh",
    "sweep2/media_manifest.json": D / "media_manifest.json",
}
for name, target in [
    ("controller.sh", "sweep_controller.sh"),
    ("media.sh", "sweep_windows_media.sh"),
    ("install.sh", "sweep_windows_install.sh"),
    ("bind.sh", "sweep_windows580.sh"),
]:
    files[target] = K / name
for f in K.iterdir():
    if f.is_file():
        files["sweep2/" + f.name] = f
for name in [
    "prepare_cold580.ps1",
    "verify_stage580.ps1",
    "enable580_once.ps1",
    "diag_min.ps1",
    "accept580.ps1",
]:
    files["sweep2/probes/" + name] = K / name
files["sweep2/probes/init_state.ps1"] = O / "scripts/bench/blackwell/init_state.ps1"
for name in ["dump_header.py", "keyline_times.py", "dump_hdr_hex.ps1"]:
    files["sweep2/" + name] = W / "scripts/bench/blackwell" / name
(D / "lease.json").write_text(json.dumps(r, indent=2) + "\n")
files["sweep2/lease.json"] = D / "lease.json"
files["sweep2/observation_authority.json"] = D / "observation_authority.json"
files["sweep2/b1_commit.txt"] = D / "b1_commit.txt"
files["sweep2/b1_merge.json"] = D / "b1_merge.json"
for name in ["windows_observation.py", "saved_xid32_observation.py", "cuda_health_probe.py"]:
    files["sweep2/guard/blackwell/" + name] = W / "scripts/bench/blackwell" / name
files["sweep2/channel_projection.py"] = W / "scripts/bench/blackwell/channel_projection.py"

for name in [
    "phase_xid_guard.py",
    "xid_policy.py",
    "shutdown_census.py",
    "arm_history.py",
    "media_wait.py",
    "init_observer.py",
    "readiness.py",
]:
    files["sweep2/guard/blackwell/" + name] = O / "scripts/bench/blackwell" / name
files["sweep2/guard/native_xid_guard.py"] = O / "scripts/bench/native_xid_guard.py"
probes = Path("/workspace/codex/sweep-20261019-probes")
manifest = {}
for name in ["cup2.exe", "cup3.exe", "cup8.exe", "kf_dxprobe.exe"]:
    f = probes / name
    assert f.is_file()
    manifest[name] = hashlib.sha256(f.read_bytes()).hexdigest()
    files["sweep2/apps/" + name] = f
(D / "probe_hashes.json").write_text(json.dumps(manifest, indent=2) + "\n")
bundle = D / "bundle.tar.gz"
with tarfile.open(bundle, "w:gz") as tar:
    for name, path in files.items():
        assert path.stat().st_size < 1024 * 1024 and ".." not in name.split("/")
        tar.add(path, arcname=name, recursive=False)
sha = hashlib.sha256(bundle.read_bytes()).hexdigest()
remote("umask 077; cat > /root/sweep_bundle.tar.gz", bundle.read_bytes())
remote(
    f'set -eu; test $(sha256sum /root/sweep_bundle.tar.gz | cut -d" " -f1) = {sha}; tar -xzf /root/sweep_bundle.tar.gz -C /root; mkdir -p /root/prov'
)
# Upload is followed by the separately checked, bounded eight-stream media transport.
(D / "bundle_receipt.json").write_text(
    json.dumps(
        {
            "bundle_sha256": sha,
            "files": {
                name: hashlib.sha256(p.read_bytes()).hexdigest() for name, p in files.items()
            },
        },
        indent=2,
    )
    + "\n"
)
print("BUNDLE_UPLOADED_NO_CONTROLLER", flush=True)
