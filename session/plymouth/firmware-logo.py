#!/usr/bin/python3
"""Copies the firmware's boot logo into the boot splash theme (Story 6.11).

UEFI firmware leaves the logo it shows at power-on in the ACPI BGRT table, which Linux exposes in
/sys/firmware/acpi/bgrt: a BMP image and its position on the screen. The splash shows it at the same
place and fades it into the ConsoleUI logo, so the firmware logo does not just vanish.

    firmware-logo.py <theme dir>   writes <theme dir>/firmware.png, prints "<xoffset> <yoffset>"

Prints nothing (and writes nothing) when there is no logo the firmware showed upright. Only the
standard library, since it runs as root on the target system from session/install-plymouth.sh.
"""

import struct
import sys
import zlib
from pathlib import Path

BGRT = Path("/sys/firmware/acpi/bgrt")


def read_int(name):
    return int((BGRT / name).read_text().strip())


def bmp_rows(data):
    """The BMP's pixel rows from top to bottom as RGB bytes, or None if the format is unsupported."""
    if data[:2] != b"BM" or len(data) < 54:
        return None
    pixels_at = struct.unpack_from("<I", data, 10)[0]
    width, height, planes, bits, compression = struct.unpack_from("<iiHHI", data, 18)
    # Firmware logos are uncompressed 24- or 32-bit images (BI_RGB, or BI_BITFIELDS for 32 bit)
    if width <= 0 or height == 0 or planes != 1 or bits not in (24, 32) or compression not in (0, 3):
        return None
    if compression == 3 and bits != 32:
        return None
    step = bits // 8
    stride = (width * step + 3) & ~3
    count = abs(height)
    if pixels_at + stride * count > len(data):
        return None
    rows = []
    for row in range(count):
        start = pixels_at + stride * row
        line = data[start : start + width * step]
        rgb = bytearray(width * 3)
        rgb[0::3] = line[2::step]
        rgb[1::3] = line[1::step]
        rgb[2::3] = line[0::step]
        rows.append(bytes(rgb))
    if height > 0:  # stored bottom-up
        rows.reverse()
    return width, count, rows


def png(width, height, rows):
    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))

    raw = b"".join(b"\0" + row for row in rows)
    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)  # 8-bit RGB
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    try:
        # Bit 0: the logo is on screen; bits 1-2: rotated by the firmware (not handled)
        status = read_int("status")
        if read_int("type") != 0 or status & 0b111 != 1:
            return
        image = bmp_rows((BGRT / "image").read_bytes())
        xoffset, yoffset = read_int("xoffset"), read_int("yoffset")
    except OSError:
        return
    if image is None:
        return
    width, height, rows = image
    Path(sys.argv[1], "firmware.png").write_bytes(png(width, height, rows))
    print(xoffset, yoffset)


if __name__ == "__main__":
    main()
