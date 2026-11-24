# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Pins and download anchors of the open Windows rows (STATUS: DESIGN-ONLY, 2026-11-17; pure functions, GPU-free).

Owner rule, 2026-11-17: for a third party who does not trust the repo, a sha256 of a binary pinned in git is NOT an anchor
(nobody can review it). Accepted anchors only: an official upstream checksum fetched at run time from the upstream origin, a vendor
code signature, a CI attestation (not used yet), and for SOURCES a pinned upstream git commit. open_rows.json therefore holds no
binary digest; validate_pins() fails if one appears.
"""
import json
import os
import re
import xml.sax.saxutils

from kfopen_common import HERE, OpenRowError, load_json

ANCHORS = ("git-commit", "upstream-sha256", "authenticode", "pypi-sha256", "nuget-signature", "ci-attestation")
KINDS = ("cts-gl", "cts-vk", "gl-soak", "dx-window", "dx-console", "not-run")
SHA256_RX = re.compile(r"(?i)\b[0-9a-f]{64}\b")
COMMIT_RX = re.compile(r"[0-9a-f]{40}")
BINARY_PIN_KEYS = re.compile(r"(?i)sha-?256|digest|checksum|hash")


class PinMismatch(OpenRowError):
    pass


def load_pins(path=None):
    return validate_pins(load_json(path or os.path.join(HERE, "open_rows.json")))


def validate_pins(doc):
    """Structure and policy of open_rows.json. Raises OpenRowError with one reason."""
    if doc.get("schema") != 1:
        raise OpenRowError("open_rows.json: schema must be 1")
    for name, src in doc["sources"].items():
        if not COMMIT_RX.fullmatch(src.get("commit", "")):
            raise OpenRowError("source %s: commit must be 40 lowercase hex" % name)
        if not src.get("repository", "").startswith("https://") or not src["repository"].endswith(".git"):
            raise OpenRowError("source %s: repository must be an https .git URL" % name)
        if src.get("anchor") != "git-commit":
            raise OpenRowError("source %s: a source is anchored by its git commit" % name)
    for name, tool in doc["tools"].items():
        if tool.get("anchor") not in ANCHORS or tool["anchor"] in ("git-commit", "ci-attestation"):
            raise OpenRowError("tool %s: anchor must be upstream-sha256, authenticode, pypi-sha256 or nuget-signature" % name)
        for key, value in tool.items():
            if BINARY_PIN_KEYS.search(key) or (isinstance(value, str) and SHA256_RX.search(value)):
                raise OpenRowError("tool %s: a binary digest pinned in git is not an accepted anchor (key %s)" % (name, key))
        for key in ("url", "download_prefix", "release_api", "pypi_json"):
            if key in tool and not tool[key].startswith("https://"):
                raise OpenRowError("tool %s: %s must be https" % (name, key))
        if tool["anchor"] == "authenticode" and not tool.get("signer"):
            raise OpenRowError("tool %s: authenticode needs the expected signer" % name)
    fps = doc["nuget_trust"]["fingerprints_sha256"]
    if not fps or not all(re.fullmatch(r"[0-9A-F]{64}", f) for f in fps) or doc["nuget_trust"]["signature_validation_mode"] != "require":
        raise OpenRowError("nuget_trust: need certificate fingerprints and signatureValidationMode=require")
    for key, spec in doc["dxs_samples"].items():
        if not spec["project"].startswith(spec["checkout"].rstrip("/") + "/") or ".." in spec["checkout"] or spec["checkout"].startswith("/"):
            raise OpenRowError("dxs sample %s: checkout must be a directory that holds its project" % key)
    for rid, row in doc["rows"].items():
        if row.get("kind") not in KINDS:
            raise OpenRowError("row %s: unknown kind %r" % (rid, row.get("kind")))
        if row["kind"] in ("dx-window", "dx-console") and row["sample"] not in doc["dxs_samples"]:
            raise OpenRowError("row %s: sample %r is not in dxs_samples" % (rid, row["sample"]))
        if row["kind"] in ("cts-gl", "cts-vk") and row["binary"] not in doc["cts_build"]["targets"]:
            raise OpenRowError("row %s: binary %r is not a cts_build target" % (rid, row["binary"]))
        if row["kind"] == "not-run" and (not re.fullmatch(r"[a-z0-9-]{5,}", row.get("reason", "")) or len(row.get("detail", "")) < 20):
            raise OpenRowError("row %s: a not-run row needs a named reason token and a detail" % rid)
    return doc


def sparse_dirs(doc, name):
    """The directories a source needs, or None for a full checkout. DirectX-Graphics-Samples is 685 MB at the pinned commit (MEASURED, depth 1);
    the samples of the rows are a few MB, so only their directories are fetched (a blobless, sparse checkout of the same pinned commit)."""
    if name != "dxs":
        return None
    return sorted({spec["checkout"] for spec in doc["dxs_samples"].values()})


def resolve_commit(doc, name, trust=None):
    """The commit a source is built at: the pin, unless the user explicitly overrode it (recorded by the caller)."""
    pin = doc["sources"][name]["commit"]
    over = (trust.commit_overrides if trust else {}).get(name)
    return over or pin


def check_commit(name, actual, wanted):
    """Refuse when a clone is not at the pinned commit (the owner's integrity pin for repo-fetched third-party sources)."""
    if actual.strip() != wanted:
        raise PinMismatch("PIN_MISMATCH %s: checked out %s, pinned %s; refusing to build (use --source-commit-override %s=<sha> to build another commit, recorded)"
                          % (name, actual.strip()[:12] or "nothing", wanted[:12], name))
    return True


# ------------------------------------------------------------------------------------------------ upstream checksum anchors
def parse_checksum_table(text, asset):
    """Find the sha256 of `asset` in upstream text: a `sha256sum` file (`<hash>  <name>`), a markdown table (`| name | hash |`)
    or a `<name>: <hash>` list. Returns a lowercase hex string; raises when the asset is absent or listed twice with different hashes."""
    found = set()
    for line in text.splitlines():
        if asset not in line:
            continue
        # the whole name must match, not a prefix of another asset (MinGit-1-64-bit.zip vs MinGit-1-64-bit.zip.sig)
        if not re.search(r"(?<![A-Za-z0-9_.+-])" + re.escape(asset) + r"(?![A-Za-z0-9_.+-])", line):
            continue
        for m in SHA256_RX.finditer(line):
            found.add(m.group(0).lower())
    if not found:
        raise OpenRowError("ANCHOR_MISSING no sha256 for %s in the upstream checksum text" % asset)
    if len(found) > 1:
        raise OpenRowError("ANCHOR_AMBIGUOUS %s has %d different sha256 values in the upstream text" % (asset, len(found)))
    return found.pop()


def expected_from_github_release(release_json, asset, download_prefix):
    """The expected sha256 of a GitHub release asset from the upstream release itself: the checksum table in the notes (Git for
    Windows publishes one) and the asset's own `digest` field. They must agree when both exist; at least one must exist."""
    rel = json.loads(release_json) if isinstance(release_json, (str, bytes)) else release_json
    assets = {a["name"]: a for a in rel.get("assets", [])}
    if asset not in assets:
        raise OpenRowError("ANCHOR_MISSING release has no asset %s" % asset)
    url = assets[asset].get("browser_download_url", "")
    if not url.startswith(download_prefix) or not url.endswith("/" + asset):
        raise OpenRowError("ANCHOR_ORIGIN asset URL %s is not under the pinned release %s" % (url, download_prefix))
    table = digest = None
    try:
        table = parse_checksum_table(rel.get("body") or "", asset)
    except OpenRowError as e:
        if not str(e).startswith("ANCHOR_MISSING"):          # an ambiguous table is a refusal, not a reason to fall back on the digest field
            raise
    d = assets[asset].get("digest") or ""
    if d.startswith("sha256:") and re.fullmatch(r"[0-9a-fA-F]{64}", d[7:]):
        digest = d[7:].lower()
    if table is None and digest is None:
        raise OpenRowError("ANCHOR_MISSING release %s lists no sha256 for %s" % (rel.get("tag_name"), asset))
    if table and digest and table != digest:
        raise OpenRowError("ANCHOR_CONFLICT release notes and asset digest disagree for %s" % asset)
    return url, table or digest


def pypi_wheel_hash(pypi_json, version, python_tag="cp312", platform="win_amd64"):
    """(filename, url, sha256) of the wheel pip will take, from the PyPI JSON of the pinned version. pip --require-hashes enforces it."""
    doc = json.loads(pypi_json) if isinstance(pypi_json, (str, bytes)) else pypi_json
    if doc.get("info", {}).get("version") != version:
        raise OpenRowError("ANCHOR_MISSING PyPI JSON is for %s, pinned %s" % (doc.get("info", {}).get("version"), version))
    wheels = [u for u in doc.get("urls", []) if u.get("packagetype") == "bdist_wheel"
              and u["filename"].endswith("-%s-%s-%s.whl" % (python_tag, python_tag, platform))]
    if len(wheels) != 1:
        raise OpenRowError("ANCHOR_MISSING %d wheels for %s %s %s (need exactly one)" % (len(wheels), version, python_tag, platform))
    w = wheels[0]
    sha = (w.get("digests") or {}).get("sha256", "")
    if not re.fullmatch(r"[0-9a-f]{64}", sha):
        raise OpenRowError("ANCHOR_MISSING PyPI lists no sha256 for %s" % w["filename"])
    if not w["url"].startswith("https://files.pythonhosted.org/"):
        raise OpenRowError("ANCHOR_ORIGIN wheel URL %s is not files.pythonhosted.org" % w["url"])
    return w["filename"], w["url"], sha


def requirements_line(name, version, sha):
    return "%s==%s --hash=sha256:%s\n" % (name, version, sha)


def authenticode_check(report_json, signer):
    """Judge `Get-AuthenticodeSignature` output (JSON with Status and Subject). Valid status and the pinned signer in the subject, or refuse."""
    rep = json.loads(report_json) if isinstance(report_json, (str, bytes)) else report_json
    status = str(rep.get("Status", ""))
    # PowerShell 5.1 serialises the enum as a number (0 = Valid) unless the script converts it; accept both spellings
    if status not in ("Valid", "0"):
        raise OpenRowError("SIGNATURE_INVALID status %s" % (status or "missing"))
    subject = str(rep.get("Subject", ""))
    if not re.search(r"(?:^|, )O=\"?" + re.escape(signer) + r"\b", subject) and not re.search(r"(?:^|, )CN=\"?" + re.escape(signer) + r"\b", subject):
        raise OpenRowError("SIGNATURE_SIGNER expected %s, signed by %s" % (signer, subject[:80]))
    return True


AUTHENTICODE_PS = ("Get-AuthenticodeSignature -LiteralPath $args[0] | Select-Object @{n='Status';e={[string]$_.Status}},"
                   "@{n='Subject';e={$_.SignerCertificate.Subject}} | ConvertTo-Json -Compress")


def nuget_config_xml(trust):
    """nuget.config that makes `nuget install` require a nuget.org repository signature (NuGet docs, nuget.config trustedSigners)."""
    q = xml.sax.saxutils.quoteattr
    certs = "\n".join('      <certificate fingerprint=%s hashAlgorithm="SHA256" allowUntrustedRoot="false" />' % q(f) for f in trust["fingerprints_sha256"])
    return ('<?xml version="1.0" encoding="utf-8"?>\n<configuration>\n  <config>\n    <add key="signatureValidationMode" value=%s />\n  </config>\n'
            '  <packageSources>\n    <clear />\n    <add key="nuget.org" value=%s protocolVersion="3" />\n  </packageSources>\n'
            '  <trustedSigners>\n    <repository name=%s serviceIndex=%s>\n%s\n    </repository>\n  </trustedSigners>\n</configuration>\n'
            % (q(trust["signature_validation_mode"]), q(trust["source"]), q(trust["repository_name"]), q(trust["source"]), certs))


def verify_sha256(path_hash, expected):
    if path_hash.lower() != expected.lower():
        raise OpenRowError("HASH_MISMATCH downloaded file has sha256 %s, upstream says %s" % (path_hash[:16], expected[:16]))
    return True
