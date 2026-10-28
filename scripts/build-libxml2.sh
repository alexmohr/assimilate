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

case "$(uname -m)" in
  x86_64) march="-march=x86-64 -mtune=generic" ;;
  aarch64) march="-march=armv8-a" ;;
  *)
    echo "error: no baseline CPU flags for $(uname -m)" >&2
    exit 1
    ;;
esac

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
