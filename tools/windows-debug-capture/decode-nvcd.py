#!/usr/bin/env python3
"""Bounded, offline NVCD protobuf research decoder. Output is PRIVATE by default.

The input and schema are data. Compile the schema separately from trusted OGKM.
No Windows executable, guest address, embedded code or raw-byte field is executed.
"""
import argparse
import collections
import hashlib
import json
import os
from pathlib import Path
import re
import struct
import uuid

MAX_BYTES = 8 * 1024 * 1024
MAX_FIELDS = 100_000
MAX_DEPTH = 32


def varint(data, at):
    value = 0
    for index in range(10):
        if at >= len(data):
            raise ValueError("truncated varint")
        byte = data[at]
        at += 1
        if index == 9 and byte > 1:
            raise ValueError("varint exceeds 64 bits")
        value |= (byte & 127) << (7 * index)
        if not byte & 128:
            return value, at
    raise ValueError("unterminated varint")


def wire(data, at, kind):
    if kind == 0:
        return varint(data, at)
    if kind in (1, 5):
        size = 8 if kind == 1 else 4
        if size > len(data) - at:
            raise ValueError("truncated fixed-width value")
        return int.from_bytes(data[at:at + size], "little"), at + size
    if kind == 2:
        size, at = varint(data, at)
        if size > len(data) - at:
            raise ValueError("truncated length-delimited value")
        return data[at:at + size], at + size
    raise ValueError("unsupported protobuf wire kind")


def byte_summary(data):
    return {"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}


class Decoder:
    def __init__(self, schema, budget=MAX_FIELDS):
        self.schema = schema
        self.budget = budget
        self.unknown = collections.Counter()
        constants = schema["constants"]
        self.types = {value: key.removeprefix("PRB_") for key, value in constants.items()
                      if key.startswith("PRB_") and key != "PRB_IS_PACKED"}

    def consume(self):
        self.budget -= 1
        if self.budget < 0:
            raise ValueError("protobuf field budget exceeded")

    def scalar(self, value, kind, field):
        if kind == "BYTES":
            return byte_summary(value)
        if kind == "STRING":
            return value.decode("utf-8", errors="strict")
        if kind == "ENUM":
            value = ((value + 2**31) % 2**32) - 2**31
            return {"value": value, "name": field["enum"].get(str(value))}
        if kind in ("SINT32", "SINT64"):
            return (value >> 1) ^ -(value & 1)
        if kind in ("INT32", "SFIXED32"):
            return ((value + 2**31) % 2**32) - 2**31
        if kind in ("INT64", "SFIXED64"):
            return ((value + 2**63) % 2**64) - 2**63
        if kind == "BOOL":
            if value > 1:
                raise ValueError("invalid boolean")
            return bool(value)
        if kind in ("FLOAT", "DOUBLE"):
            width, fmt = (4, "<f") if kind == "FLOAT" else (8, "<d")
            return struct.unpack(fmt, value.to_bytes(width, "little"))[0]
        return value

    def message(self, data, name, depth=0):
        if depth > MAX_DEPTH:
            raise ValueError("protobuf nesting limit exceeded")
        fields = self.schema["messages"][name]
        result = {}
        at = 0
        while at < len(data):
            self.consume()
            key, at = varint(data, at)
            number, wire_kind = key >> 3, key & 7
            if not 0 < number < 2**29:
                raise ValueError("invalid protobuf field number")
            value, at = wire(data, at, wire_kind)
            field = fields.get(str(number))
            if field is None:
                self.unknown[f"{name}.{number}"] += 1
                field_name = f"unknown_{number}"
                if isinstance(value, bytes):
                    value = byte_summary(value)
            else:
                field_name = field["name"]
                kind = self.types[field["type"]]
                expected = (2 if kind in ("STRING", "BYTES", "MESSAGE") else
                            1 if kind in ("DOUBLE", "FIXED64", "SFIXED64") else
                            5 if kind in ("FLOAT", "FIXED32", "SFIXED32") else 0)
                packed = field["flags"] & self.schema["constants"]["PRB_IS_PACKED"]
                if wire_kind == 2 and packed and expected != 2:
                    index, unpacked = 0, []
                    while index < len(value):
                        self.consume()
                        item, index = wire(value, index, expected)
                        unpacked.append(self.scalar(item, kind, field))
                    result.setdefault(field_name, []).extend(unpacked)
                    continue
                if wire_kind != expected:
                    raise ValueError(f"wire mismatch in {name}.{field_name}")
                value = (self.message(value, field["message"], depth + 1)
                         if kind == "MESSAGE" else self.scalar(value, kind, field))
            result.setdefault(field_name, []).append(value)
        return result


def decode_nvcd(data, schema, offset):
    if not 0 <= offset <= len(data) or len(data) > MAX_BYTES:
        raise ValueError("input size or NVCD offset outside bounds")
    constants, offsets = schema["constants"], schema["offsets"]
    header_size = schema["header_size"]
    record_size = schema["record_size"]
    proto_size = schema["protobuf_record_size"]
    data = bytes(data[offset:])

    def number(buf, name, width):
        start = offsets[name]
        if start + width > len(buf):
            raise ValueError("truncated source-defined field")
        return int.from_bytes(buf[start:start + width], "little")

    if len(data) < header_size:
        raise ValueError("truncated NVCD header")
    if number(data, "NVCD_HEADER.dwSignature", 4) != constants["NVCD_SIGNATURE"]:
        raise ValueError("wrong NVCD signature")
    start = offsets["NVCD_HEADER.gVersion"]
    guid = str(uuid.UUID(bytes_le=data[start:start + 16]))
    if guid != schema["v1_guid"]:
        raise ValueError("unrecognized NVCD version; only source-defined V1 supported")
    declared = number(data, "NVCD_HEADER.dwSize", 4)
    if not header_size <= declared <= MAX_BYTES:
        raise ValueError("invalid NVCD declared size")
    available = min(declared, len(data))
    bounded = data[:available]
    checksum = number(data, "NVCD_HEADER.cCheckSum", 1)
    metadata = {"offset": offset, "declared_bytes": declared, "available_bytes": available,
                "missing_bytes": declared - available, "version": guid,
                "checksum": checksum, "checksum_valid": None}
    if declared <= len(data) and checksum:
        metadata["checksum_valid"] = sum(bounded) % 256 == 0
        if not metadata["checksum_valid"]:
            raise ValueError("NVCD checksum mismatch")
    decoder = Decoder(schema)
    records, decoded = [], []
    at = header_size
    while at < available:
        if len(records) >= 4096:
            raise ValueError("NVCD record budget exceeded")
        if available - at < record_size:
            metadata["truncated_record_header_bytes"] = available - at
            break
        record = bounded[at:]
        group = number(record, "NVCD_RECORD.cRecordGroup", 1)
        kind = number(record, "NVCD_RECORD.cRecordType", 1)
        size = number(record, "NVCD_RECORD.wRecordSize", 2)
        if size < record_size:
            raise ValueError("NVCD record does not advance")
        item = {"offset": at, "group": group, "type": kind, "header_size": size}
        records.append(item)
        if size > available - at:
            item["truncated"] = True
            break
        total = size
        if group == constants["RmGroup"] and kind == constants["RmProtoBuf_V2"]:
            if size != proto_size:
                raise ValueError("unexpected protobuf record header size")
            length = number(record, "RmProtoBuf_RECORD.dwSize", 4)
            total += length
            item["payload_bytes"] = length
            if total > available - at:
                item["truncated"] = True
                break
            decoded.append(decoder.message(record[size:total], schema["root"]))
        at += total
        if group == constants["NvcdGroup"] and kind == constants["EndOfData"]:
            metadata["end_marker_seen"] = True
            break
    return {"nvcd": metadata, "records": records, "decoded": decoded,
            "unknown_fields": dict(decoder.unknown)}


def extract_enumtag(text, guid):
    """Decode one KD .enumtag hex block; no guesses about its opaque outer format."""
    wanted = str(uuid.UUID(guid))
    lines = text.splitlines()
    matches = []
    for index, line in enumerate(lines):
        match = re.fullmatch(r"\s*\{([0-9A-Fa-f-]+)\} - (0x[0-9a-fA-F]+) bytes\s*", line)
        if match and str(uuid.UUID(match[1])) == wanted:
            matches.append((index + 1, int(match[2], 16)))
    if len(matches) != 1:
        raise ValueError("expected exactly one matching tag")
    index, expected = matches[0]
    if expected > MAX_BYTES:
        raise ValueError("tag exceeds size bound")
    result = bytearray()
    while len(result) < expected and index < len(lines):
        # KD's first 49 columns contain two leading spaces and up to 16 hex bytes.
        field = lines[index][2:49].strip()
        if not re.fullmatch(r"[0-9a-fA-F]{2}(?: [0-9a-fA-F]{2}){0,15}", field):
            raise ValueError("malformed KD hex row")
        result.extend(bytes.fromhex(field))
        index += 1
    if len(result) != expected:
        raise ValueError("tag length disagrees with KD declaration")
    return bytes(result)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("schema", type=Path)
    parser.add_argument("input", type=Path)
    parser.add_argument("output", type=Path, help="NEW private JSON file; never overwritten")
    parser.add_argument("--offset", type=lambda text: int(text, 0), required=True)
    parser.add_argument("--enumtag-guid", help="input is KD text instead of a raw tag")
    args = parser.parse_args()
    with args.schema.open("rb") as source:
        schema_data = source.read(MAX_BYTES + 1)
    if len(schema_data) > MAX_BYTES:
        raise ValueError("schema exceeds bound")
    schema = json.loads(schema_data)
    limit = MAX_BYTES * 8 if args.enumtag_guid else MAX_BYTES
    with args.input.open("rb") as source:
        data = source.read(limit + 1)
    if len(data) > limit:
        raise ValueError("input exceeds bound")
    if args.enumtag_guid:
        data = extract_enumtag(data.decode("utf-8", errors="strict"), args.enumtag_guid)
    result = decode_nvcd(data, schema, args.offset)
    result["input"] = byte_summary(data)
    result["schema"] = byte_summary(schema_data)
    fd = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w") as output:
        json.dump(result, output, indent=2, allow_nan=False)
        output.write("\n")
    print("Decoded private JSON saved; review before sharing. "
          f"Complete protobuf records: {len(result['decoded'])}; "
          f"missing NVCD bytes: {result['nvcd']['missing_bytes']}; "
          f"unknown field occurrences: {sum(result['unknown_fields'].values())}.")


if __name__ == "__main__":
    main()
