#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Fetch the repo-selected PUBLIC bundle from the public bundles custom domain.

No run token, Worker route, credential or private-media fallback is used. The base URL is not a secret: it
defaults to PUBLIC_BASE and may only ever be PUBLIC_BASE (a Worker/payload env value cannot point the fetch at
another host); integrity is the sha256 pin in bundle.pin, checked against the manifest and the image.
"""
import argparse
import json
from pathlib import Path
import sys
import urllib.request

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import package_bundle as B
import bundle_ci as CI
import bundle_compliance as C
import publish_bundle as P

PUBLIC_BASE = 'https://bundles.kayfabe.dev'


def resolve_base(value):
    """None means the default; anything else must be exactly the bundles domain (a trailing '/' is tolerated)."""
    if value is None: return PUBLIC_BASE
    if value.rstrip('/') != PUBLIC_BASE: raise ValueError('the public bundle base must be ' + PUBLIC_BASE)
    return PUBLIC_BASE


def fetch(base, pin_text, output):
    base = P.public_base({'BUNDLE_PUBLIC_BASE_URL':base})
    digest = CI.read_pin(pin_text).get('sha256')
    if not digest: raise ValueError('missing public bundle pin')
    iso_key, receipt_key = P.keys(digest)
    request = urllib.request.Request(base.rstrip('/')+'/'+receipt_key, headers={'User-Agent':B.UA})
    with urllib.request.urlopen(request, timeout=60) as response:
        raw = response.read(2*1024**2+1)
    if len(raw)>2*1024**2: raise ValueError('public manifest exceeds limit')
    receipt = json.loads(raw); P.require_public(receipt)
    if receipt['image']['sha256'] != digest: raise ValueError('public manifest differs from repo pin')
    pin = dict(name='winapps-open.iso', url=base.rstrip('/')+'/'+iso_key,
               sha256=digest, size=receipt['image']['size'])
    B.fetch_file('public-bundle', pin, output, image=True)
    B.verify_image(output, receipt, public_only=True)
    (Path(output).parent/'winapps-open.manifest.json').write_bytes(B.canonical(receipt))


def main(argv=None):
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--base', default=None, help='only '+PUBLIC_BASE+' is accepted; default: that');ap.add_argument('--pin',type=Path,required=True);ap.add_argument('--out',type=Path,required=True)
    a=ap.parse_args(argv)
    try:
        base = resolve_base(a.base)
    except ValueError:
        print('PUBLIC_BUNDLE_FAIL base-is-not-the-bundles-domain (the only accepted base is '+PUBLIC_BASE+')',file=sys.stderr);return 1
    try:
        urllib.request.install_opener(urllib.request.build_opener(B._HTTPSRedirect()))
        fetch(base,a.pin.read_text(),a.out);print('PUBLIC_BUNDLE_OK');return 0
    except (OSError, ValueError, KeyError, TypeError):
        print('PUBLIC_BUNDLE_FAIL (no credentials or provider body logged)',file=sys.stderr);return 1


if __name__=='__main__':sys.exit(main())
