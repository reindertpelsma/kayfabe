# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Inspect verified wheel data without extracting or executing vendor code."""
import email.parser
import hashlib
from pathlib import Path
import re
import zipfile

PERMISSIVE = frozenset(('MIT','Apache-2.0','BSD-2-Clause','BSD-3-Clause','PSF-2.0','CNRI-Python','Unlicense','0BSD','Zlib','CC0-1.0','MIT-CMU','HPND'))
NVIDIA = re.compile(rb'(?:nvcuda|cudart(?:64)?[^/\\\s\x00]*|cublas[^/\\\s\x00]*|cudnn[^/\\\s\x00]*|nvrtc[^/\\\s\x00]*|nvjitlink[^/\\\s\x00]*|cufft[^/\\\s\x00]*|curand[^/\\\s\x00]*)\.(?:dll|so|lib)', re.I)


def permissive(expression):
    tokens = re.findall(r'[A-Za-z0-9.+-]+|[()]', expression)
    if ''.join(tokens) != re.sub(r'\s+', '', expression): return False
    atom, depth, count = True, 0, 0
    for token in tokens:
        if atom:
            if token == '(': depth += 1
            elif token in PERMISSIVE: atom=False; count+=1
            else: return False
        elif token == ')' and depth: depth-=1
        elif token in ('AND','OR'): atom=True
        else: return False
    return count > 0 and not atom and depth == 0


def inspect(path, pin, component):
    if not permissive(component['license_id']): raise ValueError('wheel licence is not clearly permissive')
    binary = []
    with zipfile.ZipFile(path) as z:
        metadata = [e for e in z.infolist() if e.filename.endswith('.dist-info/METADATA')]
        if len(metadata) != 1 or metadata[0].file_size > 1024**2: raise ValueError('ambiguous/oversized wheel metadata')
        raw = z.read(metadata[0]); doc = email.parser.BytesParser().parsebytes(raw)
        if doc.get('Version') != component['version']: raise ValueError('wheel version differs from manifest')
        expression = doc.get('License-Expression')
        if expression and not permissive(expression): raise ValueError('wheel metadata licence is not permissive')
        if not expression:
            labels = [x for x in doc.get_all('Classifier', []) if x.startswith('License ::')]
            allowed = {'License :: OSI Approved :: MIT License', 'License :: OSI Approved :: Apache Software License',
                       'License :: OSI Approved :: BSD License', 'License :: OSI Approved :: Python Software Foundation License'}
            if any(x not in allowed and x != 'License :: OSI Approved' for x in labels):
                raise ValueError('unclear or nonpermissive legacy metadata licence')
            raw_license = (doc.get('License') or '').strip()
            recognized = raw_license in ('MIT','MIT License','Apache','Apache 2.0','Apache 2.0 License','Apache-2.0','BSD','BSD-3-Clause','BSD License','PSF-2.0')
            if raw_license and not recognized: raise ValueError('unclear legacy licence field')
            if not recognized and not any(x in allowed for x in labels):
                raise ValueError('legacy metadata does not establish a permissive licence')
        # Older metadata has classifiers/free-text rather than SPDX expressions;
        # bind the exact metadata bytes to the reviewed manifest audit below.
        for entry in z.infolist():
            if NVIDIA.search(entry.filename.encode('utf-8')): raise ValueError('NVIDIA library in public wheel')
            if entry.is_dir(): continue
            with z.open(entry) as source:
                prefix = source.read(2)
                if prefix != b'MZ' and not entry.filename.lower().endswith(('.dll','.pyd','.so','.lib')): continue
                binary.append(entry.filename)
                tail = prefix
                while block := source.read(1024**2):
                    data = tail + block
                    if NVIDIA.search(data): raise ValueError('NVIDIA library reference in public wheel binary')
                    tail = data[-256:]
        return dict(archive_sha256=pin['sha256'], metadata_sha256=hashlib.sha256(raw).hexdigest(),
                    name=doc.get('Name'), version=doc.get('Version'), license_expression=expression,
                    license_text=doc.get('License'), license_classifiers=doc.get_all('Classifier', []),
                    binary_members=sorted(binary), member_count=len(z.infolist()), no_nvidia_libraries=True)


def verify(path, pin, component):
    if inspect(path, pin, component) != pin.get('public_audit'):
        raise ValueError('public wheel audit differs from reviewed manifest')
