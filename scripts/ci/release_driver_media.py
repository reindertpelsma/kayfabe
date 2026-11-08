#!/usr/bin/env python3
"""Fetch only a reviewed official installer pin into a disposable box cache.
No version-to-URL guesses, mirror, alternate-version or checksum fallback.
"""
import argparse
import hashlib
import os
from pathlib import Path
import sys
import urllib.error
import urllib.request

sys.path.insert(0, str(Path(__file__).resolve().parent))
import release_driver_pins


class MediaDrift(Exception):
    def __init__(self, reason):
        self.reason = reason
        super().__init__(reason)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def checked_file(pin, target):
    digest = hashlib.sha256()
    size = 0
    with target.open('rb') as f:
        while data := f.read(1024 * 1024):
            digest.update(data)
            size += len(data)
    if size != pin['size']:
        raise MediaDrift('SIZE')
    if digest.hexdigest() != pin['sha256']:
        raise MediaDrift('HASH')


def fetch_installer(pin, target, *, opener=None):
    if not release_driver_pins.valid_driver_pin(pin):
        raise MediaDrift('INVALID_PIN')
    if pin['status'] != 'PINNED':
        raise MediaDrift('UNPINNED')
    target = Path(target)
    if target.exists():
        checked_file(pin, target)
        return target
    target.parent.mkdir(parents=True, exist_ok=True)
    part = target.with_name(target.name + '.part')
    open_url = opener or (lambda request: urllib.request.build_opener(NoRedirect()).open(request, timeout=60))
    try:
        request = urllib.request.Request(pin['url'], headers={'Accept-Encoding': 'identity'})
        with open_url(request) as response:
            if response.status != 200:
                raise MediaDrift('HTTP_' + str(response.status))
            if response.geturl() != pin['url']:
                raise MediaDrift('REDIRECT')
            declared = response.headers.get('Content-Length')
            if declared is not None and int(declared) != pin['size']:
                raise MediaDrift('SIZE')
            size = 0
            with part.open('wb') as out:
                while data := response.read(1024 * 1024):
                    size += len(data)
                    if size > pin['size']:
                        raise MediaDrift('SIZE')
                    out.write(data)
        checked_file(pin, part)
        part.replace(target)
        return target
    except urllib.error.HTTPError as error:
        raise MediaDrift('HTTP_' + str(error.code)) from None
    except (OSError, ValueError):
        raise MediaDrift('DOWNLOAD') from None
    finally:
        part.unlink(missing_ok=True)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--os', choices=['linux', 'windows'], required=True)
    parser.add_argument('--version', required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        pins = release_driver_pins.decode_driver_pins(os.environ.get('KF_RELEASE_DRIVER_PINS'))
        if pins is None:
            raise MediaDrift('INVALID_PIN')
        selected = [p for p in pins if p['os'] == args.os and p['version'] == args.version]
        if len(selected) != 1:
            raise MediaDrift('MISSING_PIN')
        fetch_installer(selected[0], args.out)
    except (MediaDrift, OSError, ValueError) as error:
        reason = error.reason if isinstance(error, MediaDrift) else 'FILESYSTEM'
        print('DRIVER_MEDIA_DRIFT reason=' + reason)
        return 3
    print('DRIVER_MEDIA_OK')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
