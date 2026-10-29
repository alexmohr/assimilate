#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr

# Builds the libxml2.so.2 the Linux desktop installers ship next to the
# bundled PostgreSQL, which links it. Ubuntu 26.04 replaced libxml2.so.2 with
# libxml2.so.16, so the library can't come from the system there; PostgreSQL's
# RUNPATH ($ORIGIN/../lib) finds this copy first everywhere.
#
# Built from the source tarball pinned in desktop/bundle.toml, without the
# optional dependencies (ICU, liblzma, zlib, Python), for the same baseline
# CPU as the rest of the bundle.
#
#     scripts/build-libxml2.sh SOURCE.tar.xz OUT/libxml2.so.2
set -euo pipefail

if [ $# -ne 2 ]; then
  echo "usage: $0 SOURCE.tar.xz OUT/libxml2.so.2" >&2
  exit 2
fi
source_tarball=$(realpath "$1")
out=$(realpath -m "$2")

# The baseline CPU flags cargo passes to every C dependency, from
# .cargo/config.toml, so the two can't drift apart.
cargo_config="$(dirname "$(realpath "$0")")/../.cargo/config.toml"
march=$(python3 -c '
import sys, tomllib
with open(sys.argv[1], "rb") as file:
    flags = tomllib.load(file)["env"].get(f"CFLAGS_{sys.argv[2]}_unknown_linux_gnu")
if flags is None:
    sys.exit(f"error: .cargo/config.toml has no baseline CPU flags for {sys.argv[2]}")
print(flags)
' "$cargo_config" "$(uname -m)")

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
tar -xJf "$source_tarball" -C "$work" --strip-components=1
cd "$work"
./configure --quiet --disable-static --without-python --without-icu --without-lzma --without-zlib \
  CFLAGS="-O2 $march"
make --quiet -j"$(nproc)"

# The real file behind the libxml2.so.2 symlink, under the name PostgreSQL
# asks for.
mkdir -p "$(dirname "$out")"
cp "$(realpath .libs/libxml2.so.2)" "$out"
strip --strip-unneeded "$out"
