#!/usr/bin/env python3
"""Generates the mesh/texture fixtures used by rendering tests.
Run: python3 tests/fixtures/make_fixtures.py
Formats follow the public Roblox mesh layouts (v1 text, v2 binary)."""
import struct, zlib, os

OUT = os.path.dirname(os.path.abspath(__file__))

def write_png(path, w, h, pixel_fn):
    raw = b""
    for y in range(h):
        raw += b"\x00" + b"".join(bytes(pixel_fn(x, y)) for x in range(w))
    def chunk(tag, data):
        c = struct.pack(">I", len(data)) + tag + data
        return c + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
    png = (b"\x89PNG\r\n\x1a\n"
           + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
           + chunk(b"IDAT", zlib.compress(raw))
           + chunk(b"IEND", b""))
    open(path, "wb").write(png)

# halves.png: top half red, bottom half blue (UV orientation probe).
write_png(os.path.join(OUT, "halves.png"), 64, 64,
          lambda x, y: (255, 40, 30, 255) if y < 32 else (30, 60, 255, 255))
# checker.png: 4x4 checker black/white (texture presence probe).
write_png(os.path.join(OUT, "checker.png"), 64, 64,
          lambda x, y: (245, 245, 245, 255) if ((x // 16) + (y // 16)) % 2 == 0 else (25, 25, 25, 255))

# quad_v2.mesh: 2x2 quad in XY plane facing +Z, Roblox UVs (0,0 = bottom-left).
verts = [  # pos, normal, uv
    ((-1.0, -1.0, 0.0), (0, 0, 1), (0.0, 0.0)),
    (( 1.0, -1.0, 0.0), (0, 0, 1), (1.0, 0.0)),
    (( 1.0,  1.0, 0.0), (0, 0, 1), (1.0, 1.0)),
    ((-1.0,  1.0, 0.0), (0, 0, 1), (0.0, 1.0)),
]
faces = [(0, 1, 2), (0, 2, 3)]
data = b"version 2.00\n" + struct.pack("<HBBII", 12, 40, 12, len(verts), len(faces))
for pos, norm, uv in verts:
    data += struct.pack("<3f3f2f4b4B", *pos, *norm, *uv, 0, 0, 0, 0, 255, 255, 255, 255)
for f in faces:
    data += struct.pack("<3I", *f)
open(os.path.join(OUT, "quad_v2.mesh"), "wb").write(data)

# cube_v1.mesh: text-format unit cube, one [pos][normal][uv] per vertex.
def v1_vertex(p, n, t):
    return "[%g,%g,%g][%g,%g,%g][%g,%g,%g]" % (*p, *n, *t)
faces_v1 = []
h = 0.5
quads = [  # (normal, 4 corners CCW from outside)
    ((0, 0, 1), [(-h, -h, h), (h, -h, h), (h, h, h), (-h, h, h)]),
    ((0, 0, -1), [(h, -h, -h), (-h, -h, -h), (-h, h, -h), (h, h, -h)]),
    ((1, 0, 0), [(h, -h, h), (h, -h, -h), (h, h, -h), (h, h, h)]),
    ((-1, 0, 0), [(-h, -h, -h), (-h, -h, h), (-h, h, h), (-h, h, -h)]),
    ((0, 1, 0), [(-h, h, h), (h, h, h), (h, h, -h), (-h, h, -h)]),
    ((0, -1, 0), [(-h, -h, -h), (h, -h, -h), (h, -h, h), (-h, -h, h)]),
]
for normal, corners in quads:
    for tri in [(0, 1, 2), (0, 2, 3)]:
        for idx in tri:
            faces_v1.append(v1_vertex(corners[idx], normal, (0, 0, 0)))
text = "version 1.01\n%d\n" % (len(faces_v1) // 3) + "".join(v + "\n" for v in faces_v1)
open(os.path.join(OUT, "cube_v1.mesh"), "w").write(text)
print("fixtures written to", OUT)


# cube variants in v3/v4/v5 encodings of the SAME unit cube, so parsers can
# be regression-tested against identical geometry.
CUBE_FACES = [
    ((0, 0, 1), [(-h, -h, h), (h, -h, h), (h, h, h), (-h, h, h)]),
    ((0, 0, -1), [(h, -h, -h), (-h, -h, -h), (-h, h, -h), (h, h, -h)]),
    ((1, 0, 0), [(h, -h, h), (h, -h, -h), (h, h, -h), (h, h, h)]),
    ((-1, 0, 0), [(-h, -h, -h), (-h, -h, h), (-h, h, h), (-h, h, -h)]),
    ((0, 1, 0), [(-h, h, h), (h, h, h), (h, h, -h), (-h, h, -h)]),
    ((0, -1, 0), [(-h, -h, -h), (h, -h, -h), (h, -h, h), (-h, -h, h)]),
]

def cube_verts40():
    out = b""
    for normal, corners in CUBE_FACES:
        for tri in [(0, 1, 2), (0, 2, 3)]:
            for idx in tri:
                p = corners[idx]
                out += struct.pack("<3f3f2f4b4B", *p, *normal, 0.0, 0.0, 0, 0, 0, 0, 255, 255, 255, 255)
    return out

def cube_faces_bin(count_tris=12):
    return b"".join(struct.pack("<3I", i, i + 1, i + 2) for i in range(0, count_tris * 3, 3))

# v3: header(2s H B B H H I I) verts 40B, faces, then LOD u32(0)
v3 = b"version 3.00\n" + struct.pack("<HBBHHII", 14, 40, 12, 4, 0, 36, 12) + cube_verts40() + cube_faces_bin() + struct.pack("<I", 0)
open(os.path.join(OUT, "cube_v3.mesh"), "wb").write(v3)

# v4.01: 24-byte header, verts, faces, lod(1 u32), no bones/subsets
v4 = b"version 4.01\n" + struct.pack("<HHIIHHIHB B".replace(" ", ""), 24, 0, 36, 12, 1, 0, 0, 0, 0, 0) + cube_verts40() + cube_faces_bin() + struct.pack("<I", 36)
open(os.path.join(OUT, "cube_v4.mesh"), "wb").write(v4)

# v5.00: 32-byte header, verts, faces, lod(1 u32), no bones, no FACS payload
v5 = b"version 5.00\n" + struct.pack("<HHIIHHIHB BII".replace(" ", ""), 32, 0, 36, 12, 1, 0, 0, 0, 0, 0, 1, 0) + cube_verts40() + cube_faces_bin() + struct.pack("<I", 36)
open(os.path.join(OUT, "cube_v5.mesh"), "wb").write(v5)

# v5 skinned variant: one bone, envelopes per vertex, minimal bone table.
env = b"".join(struct.pack("<4B4B", 0, 0, 0, 0, 255, 0, 0, 0) for _ in range(36))
bone = struct.pack("<IHHf", 0, 0xFFFF, 0xFFFF, 1e30) + struct.pack("<12f", 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0)
v5s = b"version 5.00\n" + struct.pack("<HHIIHHIHB BII".replace(" ", ""), 32, 0, 36, 12, 1, 1, 4, 0, 0, 0, 1, 0) + cube_verts40() + env + cube_faces_bin() + struct.pack("<I", 36) + bone + b"Root"
open(os.path.join(OUT, "cube_v5_skinned.mesh"), "wb").write(v5s)
print("v3/v4/v5 cube fixtures written")
