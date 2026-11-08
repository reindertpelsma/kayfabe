#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Bounded research observation, not an image builder or release gate.

Creates only disposable 32 MiB regular files. No mount, chroot, VM, network,
package installation, or host configuration change. Results are observations:
even equal fixture hashes cannot qualify the real application image.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

EPOCH = 1794009600
FS_UUID = "fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e0001"
HASH_SEED = "fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e0002"
IMAGE_BYTES = 32 * 1024**2
CONFIG = """[defaults]
base_features = sparse_super,large_file,filetype,resize_inode,dir_index,ext_attr
blocksize = 4096
inode_size = 256
inode_ratio = 16384
[fs_types]
ext4 = {
features = has_journal,extent,huge_file,flex_bg,metadata_csum,64bit,dir_nlink,extra_isize
}
"""


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def command(argv, env=None):
    completed = subprocess.run(
        argv, env=env, check=True, capture_output=True, text=True, timeout=30
    )
    return completed.stdout + completed.stderr


def tree(root, reverse=False, older=False):
    files = {
        "etc/hostname": b"apps-image\n",
        "opt/apps/model.txt": b"public model fixture\n",
        "opt/apps/program": b"open application fixture\n" * 128,
    }
    for relative in sorted(files, reverse=reverse):
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(files[relative])
        path.chmod(0o755 if relative.endswith("program") else 0o644)
    (root / "opt/apps/program-link").symlink_to("program")
    os.link(root / "opt/apps/program", root / "opt/apps/program-hardlink")
    for path in sorted([root, *root.rglob("*")], reverse=True):
        stamp = EPOCH - 86400 if older and path.name == "model.txt" else EPOCH
        os.utime(path, (EPOCH, stamp), follow_symlinks=False)
    return {
        key: hashlib.sha256(value).hexdigest() for key, value in sorted(files.items())
    }


def observe(scratch):
    started = time.monotonic()
    tools = {name: shutil.which(name) for name in ("mke2fs", "e2fsck", "debugfs")}
    if not all(tools.values()):
        raise RuntimeError("mke2fs, e2fsck and debugfs must already be installed")
    config = scratch / "mke2fs.conf"
    config.write_text(CONFIG)
    roots = {}
    inventories = {}
    for name, reverse, older in (
        ("a", False, False),
        ("b", True, False),
        ("older", True, True),
    ):
        roots[name] = scratch / name
        roots[name].mkdir()
        inventories[name] = tree(roots[name], reverse, older)
    images = {}
    for name, source, fixed_seed, fixed_epoch, fixed_uuid in (
        ("normalized_a", "a", True, True, True),
        ("normalized_b", "b", True, True, True),
        ("normalized_repeat", "a", True, True, True),
        ("older_mtime", "older", True, True, True),
        ("random_hash_seed", "a", False, True, True),
        ("wall_clock", "a", True, False, True),
        ("random_fs_uuid", "a", True, True, False),
    ):
        path = scratch / (name + ".ext4")
        with path.open("xb") as stream:
            stream.truncate(IMAGE_BYTES)
        env = {
            "PATH": os.defpath + ":/usr/sbin:/sbin",
            "LC_ALL": "C",
            "TZ": "UTC",
            "MKE2FS_CONFIG": str(config),
        }
        if fixed_epoch:
            env["SOURCE_DATE_EPOCH"] = str(EPOCH)
        extended = [
            "lazy_itable_init=0",
            "lazy_journal_init=0",
            "nodiscard",
            "root_owner=0:0",
        ]
        if fixed_seed:
            extended.append("hash_seed=" + HASH_SEED)
        argv = [
            tools["mke2fs"],
            "-q",
            "-t",
            "ext4",
            "-b",
            "4096",
            "-I",
            "256",
            "-N",
            "1024",
            "-m",
            "0",
            "-U",
            FS_UUID if fixed_uuid else "random",
            "-L",
            "KF-OPEN-PROBE",
            "-E",
            ",".join(extended),
            "-d",
            str(roots[source]),
            str(path),
            "8192",
        ]
        before = time.monotonic()
        command(argv, env)
        image_hash = digest(path)
        allocated = path.stat().st_blocks * 512
        fsck = command([tools["e2fsck"], "-f", "-n", str(path)], env)
        after_check = digest(path)
        images[name] = {
            "sha256": image_hash,
            "size": path.stat().st_size,
            "allocated_bytes": allocated,
            "elapsed_seconds_including_hash_and_check": round(
                time.monotonic() - before, 6
            ),
            "readonly_check_unchanged": image_hash == after_check,
            "fsck": fsck.strip(),
            "model_inode": command(
                [tools["debugfs"], "-R", "stat /opt/apps/model.txt", str(path)], env
            ).strip(),
        }
        path.unlink()
    hashes = {name: row["sha256"] for name, row in images.items()}
    return {
        "observation_date": "2026-11-08",
        "probe_sha256": digest(Path(__file__)),
        "scope": "one host, tiny ext4 fixtures only; no chroot/apps build, boot or cross-host qualification",
        "uid": os.geteuid(),
        "cap_eff": next(
            line.split()[1]
            for line in Path("/proc/self/status").read_text().splitlines()
            if line.startswith("CapEff:")
        ),
        "source_date_epoch": EPOCH,
        "config": CONFIG,
        "config_sha256": digest(config),
        "tools": {
            name: {
                "version": command([path, "-V"]).strip(),
                "sha256": digest(Path(path)),
            }
            for name, path in tools.items()
        },
        "content_inventories_equal": inventories["a"]
        == inventories["b"]
        == inventories["older"],
        "observations": {
            "different_creation_order_equal": hashes["normalized_a"]
            == hashes["normalized_b"],
            "repeat_equal": hashes["normalized_a"] == hashes["normalized_repeat"],
            "older_mtime_equal": hashes["normalized_a"] == hashes["older_mtime"],
            "unfixed_hash_seed_equal": hashes["normalized_a"]
            == hashes["random_hash_seed"],
            "unfixed_epoch_equal": hashes["normalized_a"] == hashes["wall_clock"],
            "unfixed_fs_uuid_equal": hashes["normalized_a"] == hashes["random_fs_uuid"],
        },
        "images": images,
        "elapsed_seconds": round(time.monotonic() - started, 6),
        "peak_virtual_image_bytes": IMAGE_BYTES,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--scratch", type=Path, required=True)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(
        prefix="ext4-fixture-", dir=args.scratch
    ) as directory:
        result = observe(Path(directory))
    args.out.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
