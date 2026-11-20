#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Fetch the repo-selected PUBLIC bundle from the public bundles custom domain.

No run token, Worker route, credential or private-media fallback is used. The base URL is not a secret: it
defaults to PUBLIC_BASE and may only ever be PUBLIC_BASE (a Worker/payload env value cannot point the fetch at
another host); integrity is the sha256 pin in bundle.pin, checked against the manifest and the image.

A pin or a published manifest that records other recipe inputs than this checkout derives is refused BEFORE the download.

A failure prints one line, `PUBLIC_BUNDLE_FAIL reason=<R>`, with R from the closed set REASONS and never the exception text
(a provider body, a signed URL or a credential could be in it). RECIPE_INPUT_DRIFT and PIN_MISMATCH mean the pin names a bundle
that this checkout can no longer accept: deterministic, so windows_apps.sh refuses instead of renting another box (2026-11-19/20:
apps.json and win_cuda_equiv.py changed after the pinned ISO was built). NETWORK is any OSError, which also covers a local disk
error; transport failures that package_bundle.fetch_file has retried and re-raised as ValueError are OTHER.
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
REASONS = ('RECIPE_INPUT_DRIFT', 'PIN_MISMATCH', 'NETWORK', 'OTHER')
# The exact messages verify_image() and fetch() raise (package_bundle.py is a hashed recipe input, so they are repeated here;
# test_the_phrases_are_the_ones_verify_image_raises pins them). Exact match: a message that merely contains one is OTHER.
RECIPE_INPUT_DRIFT = frozenset(('embedded recipe input identity mismatch', 'artifact recipe differs from reviewed checkout inputs',
                                'recipe source audit differs from reviewed checkout'))
PIN_MISMATCH = frozenset(('public manifest differs from repo pin', 'image size/sha256 mismatch'))


def reason_of(error):
    """The closed-set reason for a failed fetch; never derived from anything but the exception's class and exact message."""
    if isinstance(error, CI.RecipeInputDrift): return 'RECIPE_INPUT_DRIFT'
    if isinstance(error, OSError): return 'NETWORK'
    if isinstance(error, ValueError) and str(error) in RECIPE_INPUT_DRIFT: return 'RECIPE_INPUT_DRIFT'
    if isinstance(error, ValueError) and str(error) in PIN_MISMATCH: return 'PIN_MISMATCH'
    return 'OTHER'


def resolve_base(value):
    """None means the default; anything else must be exactly the bundles domain (a trailing '/' is tolerated)."""
    if value is None: return PUBLIC_BASE
    if value.rstrip('/') != PUBLIC_BASE: raise ValueError('the public bundle base must be ' + PUBLIC_BASE)
    return PUBLIC_BASE


def fetch(base, pin_text, output, tree_inputs=None):
    """tree_inputs: this checkout's input hash (main always passes it). When given, a pin or a published manifest of other inputs is
    RecipeInputDrift, raised before any request or before the 2.7 GB download (2026-11-20, measured: every fresh box downloaded the
    ISO and then failed verify_image)."""
    base = P.public_base({'BUNDLE_PUBLIC_BASE_URL':base})
    if tree_inputs is not None: CI.check_pin_inputs(pin_text, tree_inputs)
    digest = CI.read_pin(pin_text).get('sha256')
    if not digest: raise ValueError('missing public bundle pin')
    iso_key, receipt_key = P.keys(digest)
    request = urllib.request.Request(base.rstrip('/')+'/'+receipt_key, headers={'User-Agent':B.UA})
    with urllib.request.urlopen(request, timeout=60) as response:
        raw = response.read(2*1024**2+1)
    if len(raw)>2*1024**2: raise ValueError('public manifest exceeds limit')
    receipt = json.loads(raw); P.require_public(receipt)
    if receipt['image']['sha256'] != digest: raise ValueError('public manifest differs from repo pin')
    if tree_inputs is not None and receipt.get('inputs') != tree_inputs:
        raise CI.RecipeInputDrift('the public manifest records other recipe inputs than this checkout derives; ' + CI.PIN_DRIFT_HINT)
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
        fetch(base,a.pin.read_text(),a.out,tree_inputs=CI.tree_inputs(HERE));print('PUBLIC_BUNDLE_OK');return 0
    except (OSError, ValueError, KeyError, TypeError) as error:
        print('PUBLIC_BUNDLE_FAIL reason='+reason_of(error),file=sys.stderr);return 1


if __name__=='__main__':sys.exit(main())
