# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Deterministic ISO9660 level 1 + Joliet (UCS-2, 64 characters), standard library only.

ECMA-119: 2048-byte logical blocks, both endian numeric fields, two path tables per
namespace. Primary short aliases and Joliet names point to the same file extents.
No Rock Ridge, UDF, boot records, host metadata or multi-extent files. The pinned
bundle's largest file is < 4 GiB; an oversized file fails before image creation.
All recording dates are 2000-01-01 UTC; all metadata and padding bytes are fixed.
"""
import hashlib
import io
import os
from pathlib import Path
import re
import struct

BLOCK = 2048
DATE7 = bytes((100, 1, 1, 0, 0, 0, 0))
DATE17 = b"2000010100000000\x00"
MAX_FILE = 0xFFFFFFFF


def blocks(size):
    return (size + BLOCK - 1) // BLOCK


def both16(n):
    return struct.pack("<H", n) + struct.pack(">H", n)


def both32(n):
    return struct.pack("<I", n) + struct.pack(">I", n)


def valid_path(path):
    if not isinstance(path, str) or not path or "\\" in path:
        raise ValueError(f"invalid ISO path: {path!r}")
    parts = path.split("/")
    for part in parts:
        # Windows filenames and Joliet both disallow these names/characters.
        if (not part or part in (".", "..") or part[-1] in " ." or len(part) > 64
                or any(ord(c) < 32 or ord(c) > 0xFFFF or 0xD800 <= ord(c) <= 0xDFFF
                       or c in ':;*?"<>|\x7f' for c in part)
                or part.split(".")[0].upper() in {"CON", "PRN", "AUX", "NUL", *[f"COM{i}" for i in range(1, 10)], *[f"LPT{i}" for i in range(1, 10)]}):
            raise ValueError(f"invalid ISO path: {path!r}")
    if len(parts) > 8:
        raise ValueError(f"ISO directory depth exceeds level 1: {path}")
    return parts


def record(name, extent, size, directory=False):
    n = 33 + len(name) + (len(name) % 2 == 0)
    return (bytes((n, 0)) + both32(extent) + both32(size) + DATE7
            + bytes((2 if directory else 0, 0, 0)) + both16(1) + bytes((len(name),))
            + name + (b"\x00" if len(name) % 2 == 0 else b""))


def pack_records(records):
    out = bytearray()
    for rec in records:
        remaining = BLOCK - len(out) % BLOCK
        if len(rec) > remaining:
            out.extend(bytes(remaining))
        out.extend(rec)
    out.extend(bytes((-len(out)) % BLOCK))
    return bytes(out)


class _Tree:
    def __init__(self, paths, joliet):
        self.joliet = joliet
        self.children = {"": []}
        self.names = {}
        for path in sorted(paths):
            parts = valid_path(path)
            for i in range(len(parts)):
                key = "/".join(parts[:i + 1])
                parent = "/".join(parts[:i])
                if key not in self.children[parent]:
                    self.children[parent].append(key)
                if i < len(parts) - 1:
                    if key in paths:
                        raise ValueError(f"file is also a directory: {key}")
                    self.children.setdefault(key, [])
        # Check every component, including directory case aliases.
        for parent, kids in self.children.items():
            folded = [p.rsplit("/", 1)[-1].casefold() for p in kids]
            if len(set(folded)) != len(folded):
                raise ValueError(f"case-colliding ISO paths in {parent}")
            for i, key in enumerate(sorted(kids), 1):
                base = key.rsplit("/", 1)[-1]
                if joliet:
                    self.names[key] = base.encode("utf-16-be")
                else:
                    # Aliases depend on sorted siblings, never host enumeration order.
                    self.names[key] = (f"D{i:07}" if key in self.children else f"F{i:07}.BIN;1").encode("ascii")
            kids.sort(key=lambda k: self.names[k])
        self.dirs = [""]
        for directory in self.dirs:
            self.dirs.extend(k for k in self.children[directory] if k in self.children)
        self.number = {p: i + 1 for i, p in enumerate(self.dirs)}
        self.extents = {}
        self.sizes = {}
        for p in self.dirs:
            recs = [record(b"\x00", 0, 0, True), record(b"\x01", 0, 0, True)]
            recs += [record(self.names[k], 0, 0, k in self.children) for k in self.children[p]]
            self.sizes[p] = len(pack_records(recs))

    def path_table(self, endian):
        out = bytearray()
        for path in self.dirs:
            name = self.names[path] if path else b"\x00"
            parent = path.rpartition("/")[0]
            out.extend(bytes((len(name), 0)) + struct.pack(endian + "IH", self.extents.get(path, 0), self.number[parent]) + name)
            if len(name) % 2:
                out.append(0)
        return bytes(out)

    def directory(self, path, file_extents, file_sizes):
        parent = path.rpartition("/")[0]
        recs = [record(b"\x00", self.extents[path], self.sizes[path], True),
                record(b"\x01", self.extents[parent], self.sizes[parent], True)]
        for child in self.children[path]:
            isdir = child in self.children
            recs.append(record(self.names[child], (self.extents if isdir else file_extents)[child],
                               (self.sizes if isdir else file_sizes)[child], isdir))
        return pack_records(recs)


def descriptor(tree, total, table_size, little, big, label="KFAPPS"):
    d = bytearray(BLOCK)
    d[:7] = bytes((2 if tree.joliet else 1,)) + b"CD001\x01"
    enc = "utf-16-be" if tree.joliet else "ascii"
    def text_field(start, length, text):
        pad = " " * (length // (2 if tree.joliet else 1) - len(text))
        d[start:start + length] = (text + pad).encode(enc)
    text_field(8, 32, "KAYFABE")
    text_field(40, 32, label)
    d[80:88] = both32(total)
    if tree.joliet:
        d[88:91] = b"%/E"
    d[120:124] = both16(1)
    d[124:128] = both16(1)
    d[128:132] = both16(BLOCK)
    d[132:140] = both32(table_size)
    struct.pack_into("<I", d, 140, little)
    struct.pack_into(">I", d, 148, big)
    d[156:190] = record(b"\x00", tree.extents[""], tree.sizes[""], True)
    for start in (190, 318, 446, 574):
        text_field(start, 128, "KAYFABE")
    for start in (702, 739, 776):
        # Joliet's file identifiers occupy odd length fields; leave a final zero.
        length = 36 if tree.joliet else 37
        text_field(start, length, "")
    d[813:830] = DATE17
    d[830:847] = DATE17
    d[847:864] = b"0" * 16 + b"\x00"
    d[864:881] = DATE17
    d[881] = 1
    return bytes(d)


def write_image(files, output, label="KFAPPS"):
    """Write {ISO path: bytes | local Path}. Returns the sha256/size of each input.

    Hashes are computed from the bytes actually streamed into the image. A changing
    source length aborts; the caller compares these records with its pinned inputs.
    """
    if label not in ("KFAPPS", "KFAPPSP"): raise ValueError("invalid bundle label")
    if not files:
        raise ValueError("empty ISO")
    paths = sorted(files)
    trees = [_Tree(paths, False), _Tree(paths, True)]
    sizes = {p: len(files[p]) if isinstance(files[p], bytes) else Path(files[p]).stat().st_size for p in paths}
    if any(n > MAX_FILE or n < 0 for n in sizes.values()):
        raise ValueError("ISO single-file limit is 4 GiB minus one byte")
    sector = 19  # 16 system blocks, PVD, Joliet SVD, terminator.
    tables = []
    for tree in trees:
        length = len(tree.path_table("<"))
        tables.append((length, sector, sector + blocks(length)))
        sector += 2 * blocks(length)
        for path in tree.dirs:
            tree.extents[path] = sector
            sector += blocks(tree.sizes[path])
    extents = {}
    for path in paths:
        extents[path] = sector
        sector += max(1, blocks(sizes[path]))
    if sector > 0xFFFFFFFF:
        raise ValueError("ISO volume exceeds 32-bit block count")
    output = Path(output)
    temp = output.with_name(output.name + ".part")
    records = {}
    try:
        with temp.open("wb") as f:
            f.write(bytes(16 * BLOCK))
            for tree, (length, little, big) in zip(trees, tables):
                f.write(descriptor(tree, sector, length, little, big, label))
            f.write(b"\xffCD001\x01" + bytes(BLOCK - 7))
            for tree in trees:
                for endian in ("<", ">"):
                    tab = tree.path_table(endian)
                    f.write(tab + bytes((-len(tab)) % BLOCK))
                for path in tree.dirs:
                    f.write(tree.directory(path, extents, sizes))
            for path in paths:
                if f.tell() != extents[path] * BLOCK:
                    raise ValueError("internal ISO extent mismatch")
                h, count = hashlib.sha256(), 0
                data = files[path]
                def emit(chunk):
                    nonlocal count
                    count += len(chunk)
                    if count > sizes[path]:
                        raise ValueError(f"source grew during packaging: {path}")
                    h.update(chunk); f.write(chunk)
                if isinstance(data, bytes):
                    emit(data)
                else:
                    with Path(data).open("rb") as source:
                        while chunk := source.read(1 << 20):
                            emit(chunk)
                if count != sizes[path]:
                    raise ValueError(f"source shrank during packaging: {path}")
                records[path] = dict(sha256=h.hexdigest(), size=count)
                f.write(bytes(max(1, blocks(count)) * BLOCK - count))
            if f.tell() != sector * BLOCK:
                raise ValueError("internal ISO size mismatch")
        os.replace(temp, output)
    finally:
        temp.unlink(missing_ok=True)
    return records


class Reader:
    """Read the emitted ISO subset without mounting or invoking external programs.

    Parsing is independent of the writer's trees: descriptor and directory fields
    supply the extents; no expected layout is used. Bounds, endian twins, directory
    cycles, duplicate names and overlapping extents are rejected before hashing.
    """
    def __init__(self, path, allow_any_label=False):
        self.file = Path(path).open("rb")
        try:
            self.size = Path(path).stat().st_size
            if self.size % BLOCK or self.size < 19 * BLOCK:
                raise ValueError("truncated ISO")
            pvd, svd, end = (self._read(n * BLOCK, BLOCK) for n in (16, 17, 18))
            if any(d[1:7] != b"CD001\x01" for d in (pvd, svd, end)) or (pvd[0], svd[0], end[0]) != (1, 2, 255):
                raise ValueError("invalid ISO descriptors")
            # A hybrid recognition sequence after the ISO terminator can select
            # a different filesystem in another reader. We emit only ISO/Joliet.
            # Stop at the very first post-terminator sector, before any such
            # sequence can begin (including extra CD001 descriptors).
            if self.size > 19 * BLOCK and self._read(19 * BLOCK, 7)[1:6] in (
                    b"CD001", b"BEA01", b"NSR02", b"NSR03", b"BOOT2", b"CDW02"):
                raise ValueError("unsupported hybrid volume recognition sequence")
            if svd[88:91] != b"%/E":
                raise ValueError("missing Joliet escape sequence")
            for d in (pvd, svd):
                if self._number(d, 80, 4) * BLOCK != self.size or self._number(d, 128, 2) != BLOCK:
                    raise ValueError("ISO volume size/block mismatch")
            self.label = svd[40:72].decode("utf-16-be").rstrip(" ")
            if not allow_any_label and (self.label not in ("KFAPPS", "KFAPPSP") or pvd[40:72].rstrip(b" ") != self.label.encode("ascii")):
                raise ValueError("wrong ISO volume label")
            joliet_files, joliet_dirs = self._namespace(svd, True)
            primary_files, primary_dirs = self._namespace(pvd, False)
            self._check_aliases(joliet_files, joliet_dirs, primary_files, primary_dirs)
            self.files = joliet_files
            self.primary_files = primary_files
            self.directories = joliet_dirs
        except BaseException:
            self.file.close()
            raise

    def _read(self, offset, length):
        if offset < 0 or length < 0 or offset + length > self.size:
            raise ValueError("ISO extent outside image")
        self.file.seek(offset)
        data = self.file.read(length)
        if len(data) != length:
            raise ValueError("truncated ISO extent")
        return data

    @staticmethod
    def _number(data, offset, width):
        little = int.from_bytes(data[offset:offset + width], "little")
        big = int.from_bytes(data[offset + width:offset + 2 * width], "big")
        if little != big:
            raise ValueError("ISO endian fields disagree")
        return little

    def _entry(self, data):
        if len(data) < 34 or data[0] != len(data) or data[1] or data[26:28] != b"\x00\x00" or data[25] not in (0, 2):
            raise ValueError("invalid/unsupported ISO directory record")
        # Extra System Use bytes can carry SUSP/Rock Ridge names, symlinks or
        # child-link extents. They are filesystem semantics, never L2 padding.
        if len(data) != 33 + data[32] + (data[32] % 2 == 0):
            raise ValueError("unsupported ISO directory System Use extension")
        if self._number(data, 28, 2) != 1 or 33 + data[32] > len(data):
            raise ValueError("invalid ISO directory identifier")
        return data[33:33 + data[32]], self._number(data, 2, 4), self._number(data, 10, 4), data[25] == 2

    def _walk(self, prefix, extent, length, depth, parent=None):
        if depth > 8 or extent in self._visited or extent < 19 or length % BLOCK or not 0 < length <= 64 * BLOCK:
            raise ValueError("invalid/cyclic ISO directory")
        self._visited.add(extent)
        self._directories[prefix.rstrip("/")] = (extent, length)
        self._ranges.append((extent * BLOCK, extent * BLOCK + length, prefix))
        data = self._read(extent * BLOCK, length)
        pos, names = 0, set()
        while pos < len(data):
            n = data[pos]
            if not n:
                boundary = (pos // BLOCK + 1) * BLOCK
                if any(data[pos:boundary]):
                    raise ValueError("nonzero ISO directory padding")
                pos = boundary
                continue
            if n > BLOCK - pos % BLOCK or pos + n > len(data):
                raise ValueError("ISO record crosses directory block")
            name, start, size, isdir = self._entry(data[pos:pos + n]); pos += n
            if name in (b"\x00", b"\x01"):
                target = (extent, length) if name == b"\x00" else parent or (extent, length)
                if not isdir or (start, size) != target:
                    raise ValueError("invalid ISO dot/parent directory reference")
                continue
            try:
                component = name.decode("utf-16-be" if self._joliet else "ascii")
            except UnicodeError as e:
                raise ValueError("invalid ISO namespace name") from e
            if self._joliet:
                if len(valid_path(component)) != 1:
                    raise ValueError("invalid Joliet component")
            elif not re.fullmatch(r"D[0-9]{7}" if isdir else r"F[0-9]{7}\.BIN;1", component):
                raise ValueError("invalid primary alias")
            if component.casefold() in names:
                raise ValueError("duplicate Joliet name")
            names.add(component.casefold())
            path = prefix + component
            if isdir:
                self._walk(path + "/", start, size, depth + 1, (extent, length))
            else:
                if start < 19 or (start + max(1, blocks(size))) * BLOCK > self.size:
                    raise ValueError("ISO file outside image")
                self.files[path] = (start * BLOCK, size)
                self._ranges.append((start * BLOCK, (start + max(1, blocks(size))) * BLOCK, path))

    def _namespace(self, descriptor, joliet):
        self.files, self._directories = {}, {}
        self._visited, self._ranges, self._joliet = set(), [], joliet
        name, start, size, directory = self._entry(descriptor[156:190])
        if name != b"\x00" or not directory:
            raise ValueError("invalid ISO namespace root")
        self._walk("", start, size, 0)
        self._path_tables(descriptor, joliet)
        ranges = sorted(self._ranges)
        for (_, end, _), (start, _, name) in zip(ranges, ranges[1:]):
            if start < end:
                raise ValueError(f"overlapping ISO extent: {name}")
        return self.files, self._directories

    def _path_tables(self, descriptor, joliet):
        size = self._number(descriptor, 132, 4)
        # At most 65535 directory indices, each with at most 128 name bytes.
        if not 0 < size <= 65535 * 138:
            raise ValueError("invalid ISO path table size")
        for offset, endian, required in ((140, "little", True), (144, "little", False),
                                         (148, "big", True), (152, "big", False)):
            block = int.from_bytes(descriptor[offset:offset+4], endian)
            if not block and not required:
                continue
            if block < 19:
                raise ValueError("ISO path table outside data area")
            data = self._read(block * BLOCK, size)
            rows, paths, pos = {}, [], 0
            while pos < size:
                length = data[pos]
                end = pos + 8 + length + length % 2
                if not length or end > size or data[pos+1] or len(paths) >= 65535:
                    raise ValueError("invalid ISO path table record")
                extent = int.from_bytes(data[pos+2:pos+6], endian)
                parent = int.from_bytes(data[pos+6:pos+8], endian)
                name = data[pos+8:pos+8+length]
                if not paths:
                    if name != b"\0" or parent != 1:
                        raise ValueError("invalid ISO path table root")
                    path = ""
                else:
                    if not 1 <= parent <= len(paths):
                        raise ValueError("invalid ISO path table parent")
                    try:
                        component = name.decode("utf-16-be" if joliet else "ascii")
                    except UnicodeError as error:
                        raise ValueError("invalid ISO path table name") from error
                    if joliet:
                        if len(valid_path(component)) != 1:
                            raise ValueError("invalid path table component")
                    elif not re.fullmatch(r"D[0-9]{7}", component):
                        raise ValueError("invalid primary path table alias")
                    path = (paths[parent-1] + "/" if paths[parent-1] else "") + component
                if path in rows:
                    raise ValueError("duplicate ISO path table directory")
                rows[path] = extent
                paths.append(path)
                pos = end
            if rows != {p: e[0] for p, e in self._directories.items()}:
                raise ValueError("ISO path table differs from namespace directories")
            # Optional copies may share their exact extent with the mandatory table.
            region = (block * BLOCK, (block + blocks(size)) * BLOCK, "path table")
            if region not in self._ranges:
                self._ranges.append(region)

    def _check_aliases(self, joliet_files, joliet_dirs, primary_files, primary_dirs):
        # Derive aliases independently of the writer's _Tree/layout objects.
        alias_dirs, alias_files = {"": ""}, {}
        children = {path: [] for path in joliet_dirs}
        for path in (set(joliet_dirs) - {""}) | set(joliet_files):
            children[path.rpartition("/")[0]].append(path)
        for parent in sorted(children, key=lambda p: (p.count("/"), p)):
            for number, child in enumerate(sorted(children[parent]), 1):
                directory = child in joliet_dirs
                component = f"D{number:07}" if directory else f"F{number:07}.BIN;1"
                alias = (alias_dirs[parent] + "/" if alias_dirs[parent] else "") + component
                if directory:
                    alias_dirs[child] = alias
                else:
                    alias_files[alias] = child
        if set(primary_dirs) != set(alias_dirs.values()) or set(primary_files) != set(alias_files):
            raise ValueError("primary/Joliet namespace file list differs")
        for alias, path in alias_files.items():
            expected, actual = joliet_files[path], primary_files[alias]
            if actual == expected:
                continue
            def pin(extent):
                offset, remaining = extent
                digest = hashlib.sha256()
                while remaining:
                    data = self._read(offset, min(remaining, 1 << 20))
                    digest.update(data); offset += len(data); remaining -= len(data)
                return digest.digest(), extent[1]
            if pin(actual) != pin(expected):
                raise ValueError("primary/Joliet namespace payload differs: " + path)

    def member(self, path):
        return _Member(self, path)

    def chunks(self, path):
        offset, remaining = self.files[path]
        while remaining:
            chunk = self._read(offset, min(remaining, 1 << 20))
            yield chunk
            offset += len(chunk); remaining -= len(chunk)

    def records(self):
        out = {}
        for path in sorted(self.files):
            h = hashlib.sha256()
            for chunk in self.chunks(path):
                h.update(chunk)
            out[path] = dict(sha256=h.hexdigest(), size=self.files[path][1])
        return out

    def metadata(self):
        """Describe every non-payload byte without dumping application contents."""
        descriptors={}
        for sector,label,joliet in ((16,'primary',False),(17,'joliet',True)):
            data=self._read(sector*BLOCK,BLOCK);encoding='utf-16-be'if joliet else'ascii'
            def field(at,size):return data[at:at+size].decode(encoding,errors='replace').rstrip(' \x00')
            descriptors[label]=dict(system_id=field(8,32),volume_id=field(40,32),
                publisher=field(318,128),preparer=field(446,128),application=field(574,128),
                created=data[813:830].hex(),modified=data[830:847].hex(),
                sha256=hashlib.sha256(data).hexdigest())
        h=hashlib.sha256();gaps=[];position=0
        payload = sorted(set(self.files.values()) | set(self.primary_files.values()))
        for start,size in payload:
            if start>position:
                digest=hashlib.sha256();offset=position
                while offset<start:
                    block=self._read(offset,min(start-offset,1024**2));digest.update(block);h.update(block);offset+=len(block)
                gaps.append(dict(offset=position,size=start-position,sha256=digest.hexdigest()))
            position=max(position,start+size)
        if position<self.size:
            digest=hashlib.sha256();offset=position
            while offset<self.size:
                block=self._read(offset,min(self.size-offset,1024**2));digest.update(block);h.update(block);offset+=len(block)
            gaps.append(dict(offset=position,size=self.size-position,sha256=digest.hexdigest()))
        return dict(descriptors=descriptors,non_payload_sha256=h.hexdigest(),regions=gaps,
                    extents={p:dict(offset=v[0],size=v[1])for p,v in sorted(self.files.items())},
                    primary_extents={p:dict(offset=v[0],size=v[1])for p,v in sorted(self.primary_files.items())})

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.file.close()


class _Member(io.RawIOBase):
    """Seekable read-only ISO slice for stdlib archive readers; no host extraction."""
    def __init__(self, reader, name):
        super().__init__()
        self.reader, self.iso_path = reader, name
        self.offset, self.size = reader.files[name]
        self.position = 0
        self.display_path = Path(name)
        self.name, self.parent = self.display_path.name, self.display_path.parent

    def readable(self): return True
    def seekable(self): return True
    def tell(self): return self.position

    def seek(self, offset, whence=0):
        target = offset + (self.position if whence == 1 else self.size if whence == 2 else 0)
        if whence not in (0, 1, 2) or not 0 <= target <= self.size:
            raise ValueError("ISO member seek outside payload")
        self.position = target
        return target

    def read(self, size=-1):
        if self.closed: raise ValueError("read from closed ISO member")
        size = self.size - self.position if size is None or size < 0 else size
        length = min(size, self.size - self.position)
        data = self.reader._read(self.offset + self.position, length)
        self.position += length
        return data

    def open(self, mode="rb"):
        if mode != "rb": raise ValueError("ISO members are read-only")
        return _Member(self.reader, self.iso_path)
