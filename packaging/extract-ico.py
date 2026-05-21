#!/usr/bin/env python3
"""Extract embedded PNG entries from a Windows .ico into individual files.

Stardeck's icon source lives as `icon.ico` (Windows build also consumes it).
The .ico already contains PNG-encoded entries at multiple sizes, so we just
slice them out — no Pillow or ImageMagick required.
"""
import os
import struct
import sys


def main(ico_path: str, out_dir: str) -> None:
    with open(ico_path, "rb") as f:
        data = f.read()
    _reserved, ico_type, count = struct.unpack_from("<HHH", data, 0)
    if ico_type != 1:
        raise SystemExit(f"not an ICO file (type={ico_type})")
    os.makedirs(out_dir, exist_ok=True)
    wrote = 0
    for i in range(count):
        off = 6 + 16 * i
        w, _h, _cc, _r, _planes, _bits, size, ofs = struct.unpack_from(
            "<BBBBHHII", data, off
        )
        w = w or 256  # zero-byte width means 256 in ICO encoding
        blob = data[ofs : ofs + size]
        if blob[:8] != b"\x89PNG\r\n\x1a\n":
            print(f"skip {w}x non-PNG entry", file=sys.stderr)
            continue
        out = os.path.join(out_dir, f"icon-{w}.png")
        with open(out, "wb") as g:
            g.write(blob)
        print(f"wrote {out} ({size} bytes)")
        wrote += 1
    if wrote == 0:
        raise SystemExit("no PNG entries found in ICO")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit("usage: extract-ico.py <input.ico> <out-dir>")
    main(sys.argv[1], sys.argv[2])
