# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Independent hostile-producer fixtures; only toy bytes and an in-memory store."""
import copy
import hashlib
import io
import json
import os
from pathlib import Path
from unittest.mock import patch

import helpers as H
import package_bundle as B
import bundle_iso as I
import bundle_repro as R
import bundle_tools as T
import publish_bundle as P


def digest(data):
    return dict(sha256=hashlib.sha256(data).hexdigest(), size=len(data))


class Artifact:
    def __init__(self, root):
        from test_package_bundle import manifest
        self.root = Path(root)
        self.man = manifest()
        self.man['bundle_toolchain'] = copy.deepcopy(B.load(B.HERE / 'manifest.json')['bundle_toolchain'])
        self.profile = dict(open_packages=['sevenzip'], official_packages=[])
        dl = self.root / 'downloads'
        (dl / 'sevenzip').mkdir(parents=True)
        (dl / 'sevenzip/7zr.exe').write_bytes(b'installer stays intact')
        (dl / '_compliance/sevenzip').mkdir(parents=True)
        (dl / '_compliance/sevenzip/LICENSE.txt').write_bytes(b'fixture licence')
        tools = {}
        for name in T.TOOL_NAMES + T.NOTICE_NAMES:
            path = self.root / 'built' / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b'MZ synthetic owned probe' if name.endswith('.exe') else b'synthetic compiler notice')
            tools[name] = path
        self.out = self.root / 'out'
        self.receipt = B.build_image(self.man['packages'], dl, tools, self.out,
                                     self.man['bundle_toolchain'], open_packages=['sevenzip'])
        self.iso = self.out / 'winapps-open.iso'
        self.trusted_inputs = B.input_hash(self.man, self.profile)

    def rewrite(self, edit, claimed='original'):
        """Re-sign every producer-controlled hash/marker after editing raw inputs."""
        with I.Reader(self.iso) as image:
            contents = {n: b''.join(image.chunks(n)) for n in image.files}
        embedded = json.loads(contents['bundle.inputs.json'])
        edit(contents, embedded)
        if claimed == 'derive':
            embedded['inputs'] = B.iso_input_hash(embedded['packages'], embedded['toolchain'],
                                                 embedded.get('open_packages', ['sevenzip']))
        contents['bundle.inputs.json'] = B.canonical(embedded)
        self.receipt.update(inputs=embedded['inputs'], packages=embedded['packages'], toolchain=embedded['toolchain'])
        old = self.receipt['files']
        files = {n: dict(old.get(n, dict(licenses=B.bundle_compliance.REPO_LICENSE,
                                        reproducibility=dict(mode='sha256'))), **digest(body))
                 for n, body in contents.items()}
        ident = B.identity(files)
        contents['KFAPPS.ID'] = (ident + '\n').encode('ascii')
        files['KFAPPS.ID'].update(digest(contents['KFAPPS.ID']))
        I.write_image(contents, self.iso)
        self.receipt['files'] = files
        self.receipt['image'].update(identity=ident, **digest(self.iso.read_bytes()))
        self.receipt['inventory_sha256'] = R.inventory_hash(self.receipt)
        with I.Reader(self.iso) as image:
            self.receipt['container_metadata'] = image.metadata()
        (self.out / 'winapps-open.manifest.json').write_bytes(B.canonical(self.receipt))

    def verify_cli(self):
        manifest = self.root / 'reviewed-manifest.json'
        profiles = self.root / 'reviewed-profiles.json'
        manifest.write_bytes(B.canonical(self.man)); profiles.write_bytes(B.canonical(self.profile))
        with patch.dict(os.environ, {}, clear=True), patch('sys.stdout', new_callable=io.StringIO), \
                patch('sys.stderr', new_callable=io.StringIO):
            return B.main(['verify-image', '--out', str(self.out), '--manifest', str(manifest),
                           '--profiles', str(profiles)])

    def publish(self):
        from test_bundle_publish import MemoryStore
        store = MemoryStore()
        original = B.load
        def loader(path):
            path = Path(path)
            if path == B.HERE / 'manifest.json': return copy.deepcopy(self.man)
            if path == B.HERE / 'gate_profiles.json': return copy.deepcopy(self.profile)
            return original(path)
        env = dict(GITHUB_ACTIONS='true', GITHUB_REPOSITORY='reindertpelsma/kayfabe',
                   GITHUB_REF='refs/heads/master', GITHUB_EVENT_NAME='workflow_dispatch',
                   BUNDLE_PUBLISH_APPROVED='true', BUNDLE_PUBLIC_BASE_URL='https://bundles.invalid',
                   R2_ENDPOINT='https://store.invalid', R2_BUCKET='fixture-public',
                   AWS_ACCESS_KEY_ID='fixture-key', AWS_SECRET_ACCESS_KEY='fixture-secret')
        with patch.object(B, 'load', side_effect=loader), patch.object(P, 'S3Store', return_value=store) as factory, \
                patch.dict(os.environ, env, clear=True), patch('sys.stdout', new_callable=io.StringIO), \
                patch('sys.stderr', new_callable=io.StringIO):
            rc = P.main(['--out', str(self.out)])
        return rc, store, factory


def directory_entries(raw, sector=16):
    """Tiny independent directory walker for mutation offsets, not the receiver."""
    descriptor = raw[sector*I.BLOCK:(sector+1)*I.BLOCK]
    result = {}
    def walk(prefix, start, size):
        pos, end = start*I.BLOCK, start*I.BLOCK+size
        while pos < end:
            n = raw[pos]
            if not n:
                pos = (pos//I.BLOCK+1)*I.BLOCK
                continue
            entry = raw[pos:pos+n]; name = entry[33:33+entry[32]]
            if name not in (b'\0', b'\1'):
                name = name.decode('ascii' if sector == 16 else 'utf-16-be')
                extent = int.from_bytes(entry[2:6], 'little'); length = int.from_bytes(entry[10:14], 'little')
                path = prefix+name
                result[path] = dict(entry=pos, extent=extent, size=length, directory=bool(entry[25]&2))
                if entry[25]&2: walk(path+'/', extent, length)
            pos += n
    walk('', int.from_bytes(descriptor[158:162], 'little'), int.from_bytes(descriptor[166:170], 'little'))
    return result


class ImagePair:
    def __init__(self, root):
        self.root = Path(root); self.iso = self.root/'candidate.iso'
        records = I.write_image({'a.txt':b'11111111', 'b.txt':b'22222222'}, self.iso)
        self.original = dict(files=records, image=digest(self.iso.read_bytes()), toolchain={})
        with I.Reader(self.iso) as image: self.original['container_metadata'] = image.metadata()
        self.candidate = copy.deepcopy(self.original)

    def alter(self, edit):
        raw = bytearray(self.iso.read_bytes()); edit(raw)
        self.iso.write_bytes(raw); self.candidate['image'] = digest(raw)

    def cli(self):
        reference=self.root/'reference.json'; candidate=self.root/'candidate.json'
        reference.write_bytes(B.canonical(self.original)); candidate.write_bytes(B.canonical(self.candidate))
        with patch.dict(os.environ, {}, clear=True), patch('sys.stdout', new_callable=io.StringIO) as out, \
                patch('sys.stderr', new_callable=io.StringIO):
            rc=B.main(['compare-image','--iso',str(self.iso),'--receipt',str(reference),
                       '--candidate-receipt',str(candidate),'--out',str(self.root/'report')])
        return rc,out.getvalue()

    @staticmethod
    def append(raw, payload):
        extent=len(raw)//I.BLOCK;raw.extend(payload.ljust(I.BLOCK,b'\0'))
        for sector in (16,17):raw[sector*I.BLOCK+80:sector*I.BLOCK+88]=I.both32(len(raw)//I.BLOCK)
        return extent


def add_primary_extension(raw, kind):
    """Independent SUSP/Rock Ridge encoding: expose a name or symlink override."""
    start = int.from_bytes(raw[16*I.BLOCK+158:16*I.BLOCK+162], 'little')*I.BLOCK
    rows = []; pos = start
    while raw[pos]:
        row = bytearray(raw[pos:pos+raw[pos]])
        name = row[33:33+row[32]]
        if name == b'\0':
            row += b'SP\x07\x01\xbe\xef\x00' + b'ER\x12\x01\x0a\x00\x00\x01RRIP_1991A'
        if name == b'F0000001.BIN;1':
            alternative = b'unreviewed.txt'
            if kind == 'name':
                row += b'NM' + bytes((5+len(alternative), 1, 0)) + alternative
            elif kind == 'symlink':
                row += b'PX\x2c\x01' + b''.join(I.both32(n) for n in (0o120777, 1, 0, 0, 1))
                row += b'SL' + bytes((7+len(alternative), 1, 0, 0, len(alternative))) + alternative
            else:
                raise ValueError('unknown fixture extension')
        row[0] = len(row); rows.append(bytes(row)); pos += raw[pos]
    body = I.pack_records(rows)
    assert len(body) == I.BLOCK
    raw[start:start+I.BLOCK] = body


def insert_front_blocks(raw, descriptors):
    """Relocate both complete namespaces; insert VRS records or plain padding."""
    original = bytes(raw); count = len(descriptors)
    result = bytearray(original[:19*I.BLOCK] + b''.join(d.ljust(I.BLOCK, b'\0') for d in descriptors)
                       + original[19*I.BLOCK:])
    for sector in (16, 17):
        at = sector*I.BLOCK
        result[at+80:at+88] = I.both32(len(result)//I.BLOCK)
        root = int.from_bytes(original[at+158:at+162], 'little')
        result[at+158:at+166] = I.both32(root+count)
        size = int.from_bytes(original[at+166:at+170], 'little')
        def directory(start, length):
            pos = start*I.BLOCK; end = pos+length
            while pos < end:
                n = original[pos]
                if not n:
                    pos = (pos//I.BLOCK+1)*I.BLOCK; continue
                extent = int.from_bytes(original[pos+2:pos+6], 'little')
                offset = pos+count*I.BLOCK
                result[offset+2:offset+10] = I.both32(extent+count)
                name = original[pos+33:pos+33+original[pos+32]]
                if original[pos+25]&2 and name not in (b'\0', b'\1'):
                    directory(extent, int.from_bytes(original[pos+10:pos+14], 'little'))
                pos += n
        directory(root, size)
        table_size = int.from_bytes(original[at+132:at+136], 'little')
        for pointer, endian in ((140, 'little'), (148, 'big')):
            block = int.from_bytes(original[at+pointer:at+pointer+4], endian)
            result[at+pointer:at+pointer+4] = (block+count).to_bytes(4, endian)
            pos = block*I.BLOCK; end = pos+table_size
            while pos < end:
                extent = int.from_bytes(original[pos+2:pos+6], endian)
                offset = pos+count*I.BLOCK
                result[offset+2:offset+6] = (extent+count).to_bytes(4, endian)
                pos += 8 + original[pos] + original[pos]%2
    raw[:] = result
