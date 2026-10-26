import io, py_compile
p = r"D:\github-project\liquid-verre-rs\tools\verify_content.py"
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()

old = "def assert4_dirty(px_d, px_dn, px_c, w, h, rects):\n    x0, y0, _x1, _y1, rad = rects[\"lower\"]"
new = "def assert4_dirty(px_d, px_dn, px_c, w, h, rects):\n    x0, y0, x1, y1, rad = rects[\"lower\"]"
assert old in c, "assert4 unpack not found"
c = c.replace(old, new, 1)

# dump BEFORE the leak raise so the probe run still prints its numbers
old = """    cx, cy = lx0 + lrad + 20, ly0 + lrad + 20
    pc = px_s[cy * w + cx]
    if not is_raw_texel(pc):
        raise Fail("断言2：对照点 %s=%s 不是裸 checker —— 内容未生效，叠放断言为空断言" % ((cx, cy), pc))
    if leaked:
        raise Fail("断言2：叠放违例——重叠区 %d 个采样点露出裸内容纹素（批量 pass 特征），例 %s" % (len(leaked), leaked[:4]))
    same_as_bare = sum(1 for (x, y) in pts if px_b[y * w + x] == px_s[y * w + x])
    if same_as_bare == len(pts):
        raise Fail("断言2：重叠区与无内容基线逐位相同——内容混色贡献为零，断言失效")
    if os.environ.get("VC_DUMP") == "1":
        for xy, p, e in dump:
            print("  DUMP %s stack=%s bare=%s raw_expect=%s" % (xy, p, px_b[xy[1] * w + xy[0]], e))
    print("  PASS 断言2：重叠区 %d 点均为玻璃混入态（对照点确认内容生效；与 bare 有差异）" % len(pts))"""
new = """    if os.environ.get("VC_DUMP") == "1":
        for xy, p, e in dump:
            print("  DUMP %s stack=%s bare=%s raw_expect=%s" % (xy, p, px_b[xy[1] * w + xy[0]], e))
    cx, cy = lx0 + lrad + 20, ly0 + lrad + 20
    pc = px_s[cy * w + cx]
    if not is_raw_texel(pc):
        raise Fail("断言2：对照点 %s=%s 不是裸 checker —— 内容未生效，叠放断言为空断言" % ((cx, cy), pc))
    if leaked:
        raise Fail("断言2：叠放违例——重叠区 %d 个采样点露出裸内容纹素（批量 pass 特征），例 %s" % (len(leaked), leaked[:4]))
    same_as_bare = sum(1 for (x, y) in pts if px_b[y * w + x] == px_s[y * w + x])
    if same_as_bare == len(pts):
        raise Fail("断言2：重叠区与无内容基线逐位相同——内容混色贡献为零，断言失效")
    print("  PASS 断言2：重叠区 %d 点均为玻璃混入态（对照点确认内容生效；与 bare 有差异）" % len(pts))"""
assert old in c, "assert2 tail not found"
c = c.replace(old, new, 1)

with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
py_compile.compile(p, doraise=True)
print("verify_content.py fixed + compiles")
