import io
p = r"D:\github-project\liquid-verre-rs\tools\verify_content.py"
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()
old = """def assert1_checker(px, w, rects):
    x0, y0, x1, y1, rad = rects["lower"]"""
new = """def assert1_checker(px, w, rects, px_off):
    x0, y0, x1, y1, rad = rects["lower"]"""
assert old in c
c = c.replace(old, new, 1)
old = """    # 块外（阴影外扩之外）必须是全透明画布
    ox, oy = min(x1 + 60, w - 1), y0 + inset
    p = px[oy * w + ox]
    if p[3] != 0:
        raise Fail("断言1：块外点 (%d,%d) alpha=%d != 0（内容渗出？）" % (ox, oy, p[3]))
    print("  PASS 断言1：checker 精确采样 25 点 + 128px 周期行 + 块外透明")"""
new = """    # 块外（阴影外扩之外）像素必须与无内容基线逐位一致：
    # composed 图 = capture+overlay 合成（alpha 恒 255），「渗出」的判据是
    # 块外出现了 checker 灰阶 / 与 off 基线不一致。
    for (ox, oy) in [(x1 + 60, y0 + inset), (x0 - 60, y1 - inset)]:
        if not (0 <= ox < w):
            continue
        if px[oy * w + ox] != px_off[oy * w + ox]:
            raise Fail(
                "断言1：块外点 (%d,%d) 与 off 基线不一致（%s vs %s，内容渗出？）"
                % (ox, oy, px[oy * w + ox], px_off[oy * w + ox])
            )
    print("  PASS 断言1：checker 精确采样 25 点 + 128px 周期行 + 块外与基线一致")"""
assert old in c, "assert1 tail"
c = c.replace(old, new, 1)
old = '        assert1_checker(parsed["checker"][2], w, rects["checker"])'
new = '        assert1_checker(parsed["checker"][2], w, rects["checker"], parsed["off"][2])'
assert old in c
c = c.replace(old, new, 1)
with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
import py_compile
py_compile.compile(p, doraise=True)
print("assert1 fixed, compiles")
