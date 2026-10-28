# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr

"""Fail when a Linux binary needs a newer glibc than the oldest supported one.

The release binaries are built on Ubuntu 26.04, whose glibc is newer than
Debian 13's. A binary picks up the build host's glibc symbol versions as it
links, so nothing but this check stops it from refusing to start on an older,
still supported distribution. It reads the glibc versions each file needs
from its ELF version requirements (.gnu.version_r) and exits 1 if any is
newer than FLOOR.

    python3 scripts/glibc_floor.py 2.41 target/release/agent target/release/server
"""

import struct
import sys
from pathlib import Path

SHT_GNU_VERNEED = 0x6FFFFFFE
GLIBC_PREFIX = "GLIBC_"


def parse_version(text: str) -> tuple[int, ...]:
    """'2.41' -> (2, 41)."""
    return tuple(int(part) for part in text.split("."))


def required_glibc_versions(data: bytes) -> set[tuple[int, ...]]:
    """Every GLIBC_x.y version a little-endian ELF64 file needs."""
    if data[:4] != b"\x7fELF" or data[4] != 2 or data[5] != 1:
        raise ValueError("not a little-endian 64-bit ELF file")
    (shoff,) = struct.unpack_from("<Q", data, 0x28)
    shentsize, shnum = struct.unpack_from("<HH", data, 0x3A)
    sections = [
        struct.unpack_from("<IIQQQQIIQQ", data, shoff + index * shentsize) for index in range(shnum)
    ]
    versions: set[tuple[int, ...]] = set()
    for _name, sh_type, _flags, _addr, offset, _size, link, info, _align, _entsize in sections:
        if sh_type != SHT_GNU_VERNEED:
            continue
        strtab = sections[link][4]
        entry = offset
        for _ in range(info):
            _version, aux_count, _file, aux, next_entry = struct.unpack_from("<HHIII", data, entry)
            aux_entry = entry + aux
            for _ in range(aux_count):
                _hash, _flags, _other, name, next_aux = struct.unpack_from(
                    "<IHHII", data, aux_entry
                )
                end = data.index(b"\0", strtab + name)
                symbol_version = data[strtab + name : end].decode()
                if symbol_version.startswith(GLIBC_PREFIX):
                    suffix = symbol_version[len(GLIBC_PREFIX) :]
                    if suffix.replace(".", "").isdigit():
                        versions.add(parse_version(suffix))
                aux_entry += next_aux
            entry += next_entry
    return versions


def main(argv: list[str]) -> int:
    if len(argv) < 3:
        print(f"usage: {argv[0]} FLOOR FILE...", file=sys.stderr)
        return 2
    floor = parse_version(argv[1])
    failed = False
    for path in argv[2:]:
        versions = required_glibc_versions(Path(path).read_bytes())
        newest = max(versions, default=None)
        shown = ".".join(map(str, newest)) if newest else "none"
        if newest is not None and newest > floor:
            print(f"{path}: needs glibc {shown}, newer than {argv[1]}", file=sys.stderr)
            failed = True
        else:
            print(f"{path}: needs glibc {shown}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
