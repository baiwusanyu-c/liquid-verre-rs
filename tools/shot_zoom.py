"""把内部取证 BMP 的局部裁剪放大成 PNG（4x 近邻，纯标准库）。

用法: python tools/shot_zoom.py <src.bmp> <dst.png> <cx> <cy> [half=80]
border 这类 1~5 px 的细视觉，肉眼看全图分辨不出，必须放大取证。
"""
import io, struct, sys, zlib


def load(p):
    d = io.open(p, 'rb').read()
    off = int.from_bytes(d[10:14], 'little')
    w = int.from_bytes(d[18:22], 'little', signed=True)
    h = abs(int.from_bytes(d[22:26], 'little', signed=True))
    return d[off:], w, h


def write_png(path, rows, w, h):
    raw = b''.join(b'\x00' + b''.join(bytes(px) for px in row) for row in rows)
    def chunk(tag, data):
        c = struct.pack('>I', len(data)) + tag + data
        return c + struct.pack('>I', zlib.crc32(tag + data) & 0xffffffff)
    ihdr = struct.pack('>IIBBBBB', w, h, 8, 2, 0, 0, 0)
    out = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', ihdr) + chunk(b'IDAT', zlib.compress(raw, 6)) + chunk(b'IEND', b'')
    io.open(path, 'wb').write(out)


def zoom(src_path, dst_path, cx, cy, half, factor=4):
    px, w, h = load(src_path)
    rows = []
    for y in range(cy - half, cy + half):
        row = []
        for x in range(cx - half, cx + half):
            i = (y * w + x) * 4
            row.append((px[i + 2], px[i + 1], px[i + 0]))
        rows.append(row)
    big = []
    for row in rows:
        for _ in range(factor):
            bigrow = []
            for px3 in row:
                for _ in range(factor):
                    bigrow.append(px3)
            big.append(bigrow)
    write_png(dst_path, big, len(big[0]), len(big))
    print(dst_path, len(big[0]), len(big))


if __name__ == '__main__':
    zoom(sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4]), int(sys.argv[5]))
