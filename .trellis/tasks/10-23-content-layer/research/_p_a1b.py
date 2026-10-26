import io, py_compile
p = r"D:\github-project\liquid-verre-rs\tools\verify_content.py"
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()
old = """def assert1_checker(px, w, rects):
    x0, y0, x1, y1, rad = rects["lower"]"""
new = """def assert1_checker(px, w, rects, px_off):
    x0, y0, x1, y1, rad = rects["lower"]"""
assert old in c
c = c.replace(old, new, 1)
old = """    ox, oy = x1 + 60, y0 + lin
    if ox < w and px[oy * w + ox][3] != 0:
        raise Fail("断言1：块外点 (%d,%d) alpha=%d != 0" % (ox, oy, px[oy * w + ox][3]))
    print("  PASS 断言1：checker 精确采样 25 点 + 128px 周期行 + 块外透明")"""
new = """    # 块外像素必须与无内容基线逐位一致：composed 图 = capture+overlay 合成
    # （alpha 恒 255），「渗出」的判据是与 off 基线不一致，而非 alpha==0。
    for (ox, oy) in [(x1 + 60, y0 + lin), (x0 - 60, y1 - lin)]:
        if not (0 <= ox < w):
            continue
        if px[oy * w + ox] != px_off[oy * w + ox]:
            raise Fail(
                "断言1：块外点 (%d,%d) 与 off 基线不一致（%s vs %s，内容渗出？）"
                % (ox, oy, px[oy * w + ox], px_off[oy * w + ox])
            )
    print("  PASS 断言1：checker 精确采样 25 点 + 128px 周期行 + 块外与基线一致")"""
assert old in c, "tail"
c = c.replace(old, new, 1)
old = '        assert1_checker(parsed["checker"][2], w, rects["checker"])'
new = '        assert1_checker(parsed["checker"][2], w, rects["checker"], parsed["off"][2])'
assert old in c
c = c.replace(old, new, 1)
with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
py_compile.compile(p, doraise=True)
print("assert1 fixed")
