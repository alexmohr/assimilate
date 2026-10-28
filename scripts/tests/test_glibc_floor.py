# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr

"""Tests for scripts/glibc_floor.py, using only the standard library.

python3 -m unittest discover -s scripts/tests
"""

import contextlib
import importlib.util
import io
import struct
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
_SPEC = importlib.util.spec_from_file_location(
    "glibc_floor", REPO_ROOT / "scripts" / "glibc_floor.py"
)
assert _SPEC is not None
assert _SPEC.loader is not None
gf = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(gf)


def make_elf(requirements: dict[str, list[str]]) -> bytes:
    """A minimal ELF64 file whose .gnu.version_r needs `requirements`, by library."""
    strtab = bytearray(b"\0")

    def string(text: str) -> int:
        offset = len(strtab)
        strtab.extend(text.encode() + b"\0")
        return offset

    verneed = bytearray()
    libraries = list(requirements.items())
    for index, (library, versions) in enumerate(libraries):
        last = index == len(libraries) - 1
        next_entry = 0 if last else 16 + 16 * len(versions)
        verneed += struct.pack("<HHIII", 1, len(versions), string(library), 16, next_entry)
        for aux_index, version in enumerate(versions):
            next_aux = 0 if aux_index == len(versions) - 1 else 16
            verneed += struct.pack("<IHHII", 0, 0, 0, string(version), next_aux)

    header_size = 64
    strtab_offset = header_size
    verneed_offset = strtab_offset + len(strtab)
    shoff = verneed_offset + len(verneed)
    header = bytearray(header_size)
    header[:6] = b"\x7fELF\x02\x01"
    struct.pack_into("<Q", header, 0x28, shoff)
    struct.pack_into("<HH", header, 0x3A, 64, 3)
    sections = [
        struct.pack("<IIQQQQIIQQ", 0, 0, 0, 0, 0, 0, 0, 0, 0, 0),
        struct.pack("<IIQQQQIIQQ", 0, 3, 0, 0, strtab_offset, len(strtab), 0, 0, 1, 0),
        struct.pack(
            "<IIQQQQIIQQ",
            0,
            gf.SHT_GNU_VERNEED,
            0,
            0,
            verneed_offset,
            len(verneed),
            1,
            len(libraries),
            8,
            0,
        ),
    ]
    return bytes(header) + bytes(strtab) + bytes(verneed) + b"".join(sections)


class RequiredVersionsTest(unittest.TestCase):
    def test_collects_glibc_versions_of_every_library(self) -> None:
        data = make_elf(
            {
                "libc.so.6": ["GLIBC_2.2.5", "GLIBC_2.39", "GLIBC_PRIVATE"],
                "libm.so.6": ["GLIBC_2.41"],
                "libgcc_s.so.1": ["GCC_3.0"],
            }
        )
        assert gf.required_glibc_versions(data) == {(2, 2, 5), (2, 39), (2, 41)}

    def test_file_without_version_requirements_needs_nothing(self) -> None:
        assert gf.required_glibc_versions(make_elf({})) == set()

    def test_refuses_a_file_that_is_not_elf(self) -> None:
        try:
            gf.required_glibc_versions(b"#!/bin/sh\n")
        except ValueError:
            return
        raise AssertionError("ValueError not raised")


class MainTest(unittest.TestCase):
    def run_main(self, floor: str, data: bytes) -> tuple[int, str]:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "binary"
            path.write_bytes(data)
            out, err = io.StringIO(), io.StringIO()
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                code = gf.main(["glibc_floor.py", floor, str(path)])
            return code, out.getvalue() + err.getvalue()

    def test_passes_at_the_floor(self) -> None:
        code, output = self.run_main("2.41", make_elf({"libc.so.6": ["GLIBC_2.34", "GLIBC_2.41"]}))
        assert code == 0
        assert "needs glibc 2.41" in output

    def test_fails_above_the_floor(self) -> None:
        code, output = self.run_main("2.41", make_elf({"libc.so.6": ["GLIBC_2.43"]}))
        assert code == 1
        assert "needs glibc 2.43, newer than 2.41" in output

    def test_compares_versions_numerically(self) -> None:
        code, _ = self.run_main("2.41", make_elf({"libc.so.6": ["GLIBC_2.5"]}))
        assert code == 0


if __name__ == "__main__":
    unittest.main()
