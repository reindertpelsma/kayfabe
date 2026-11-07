# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Frozen main/universe allowlist. Box host fetches; guest has no vendor network.

Only package names/paths from hash-pinned Ubuntu indices are eligible. apt in the
guest also authenticates the unchanged InRelease/Packages with Ubuntu's keyring.
"""

import lzma
from pathlib import Path
from common import download, verify_file, relative_path
from official import MediaDrift, fetch


class AptMirror:
    def __init__(self, recipe, cache, fetcher=fetch, metadata_fetcher=download):
        if recipe["apt"]["components"] != ["main", "universe"]:
            raise ValueError("main/universe only")
        self.cache = Path(cache)
        self.fetcher = fetcher
        self.paths = {}
        self.packages = {}
        self.names = set()
        self.used = {}
        self.failure = None
        self.bytes = 0
        self.budget = recipe["apt"]["download_budget_bytes"]
        prefix = "https://snapshot.ubuntu.com/ubuntu/" + recipe["apt"]["snapshot"] + "/"
        for pin in recipe["apt"]["metadata"]:
            relative_path(pin["path"])
            p = metadata_fetcher(pin, self.cache)
            verify_file(p, pin)
            self.paths[pin["path"]] = p
            if pin["path"].endswith("Packages.xz"):
                directory = pin["path"].rsplit("/", 1)[0]
                self.paths[directory + "/by-hash/SHA256/" + pin["sha256"]] = p
                with lzma.open(p, "rt", encoding="utf-8") as source:
                    row = {}
                    for line in source:
                        if not line.strip():
                            self.add(row, prefix)
                            row = {}
                        elif not line.startswith((" ", "\t")) and ": " in line:
                            key, value = line.rstrip("\n").split(": ", 1)
                            if key in {
                                "Package",
                                "Version",
                                "Filename",
                                "Size",
                                "SHA256",
                            }:
                                row[key] = value
                    if row:
                        self.add(row, prefix)

    def add(self, row, prefix):
        if not row:
            return
        path = row["Filename"]
        relative_path(path)
        if not path.startswith(("pool/main/", "pool/universe/")):
            raise ValueError("package outside pool")
        pin = {
            "id": "apt-" + row["SHA256"][:24],
            "url": prefix + path,
            "sha256": row["SHA256"],
            "size": int(row["Size"]),
            "package": row["Package"],
            "version": row["Version"],
        }
        previous = self.packages.get(path)
        if previous and (previous["sha256"], previous["size"]) != (
            pin["sha256"],
            pin["size"],
        ):
            raise MediaDrift("MEDIA_DRIFT: inconsistent Ubuntu package filename")
        self.packages[path] = pin
        self.names.add(row["Package"])

    def get(self, path):
        if path in self.paths:
            return self.paths[path]
        pin = self.packages.get(path)
        if pin is None:
            return None
        try:
            if path not in self.used:
                if self.bytes + pin["size"] > self.budget:
                    raise MediaDrift("MEDIA_DRIFT: apt fetch budget exceeded")
                self.bytes += pin["size"]
                self.used[path] = pin
            return self.fetcher(pin, self.cache)
        except Exception as error:
            self.failure = error
            raise
