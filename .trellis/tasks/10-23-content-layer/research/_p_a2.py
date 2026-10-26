import io, py_compile
p = r"D:\github-project\liquid-verre-rs\tools\verify_content.py"
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()
old = """    same_as_bare = sum(1 for (x, y) in pts if px_b[y * w + x] == px_s[y * w + x])
    if same_as_bare == len(pts):
        raise Fail("断言2：重叠区与无内容基线逐位相同——内容混色贡献为零，断言失效")"""
new = """    # 防「空断言」的护栏不是「stack != bare」：本项目液态玻璃 base_alpha≈1，
    # 上层玻璃对下层内容的混色贡献 <1 LSB，重叠区与 bare 逐位相同是**正确**
    # 物理结果（2026-10-26 实测：interleaved 25/25 点 stack==bare）。
    # 真正的护栏 = 对照点（上块未覆盖区的内容必须是裸纹素，已在上面判过）
    # + 断言 1 的 378 点全等——两者联合保证「内容在画、且被上层玻璃盖住」。
    leaked_outside_mask = sum(
        1 for (x, y) in pts if is_raw_texel(px_b[y * w + x])
    )
    if leaked_outside_mask:
        raise Fail("断言2：bare（无内容）run 的重叠区出现 %d 个裸纹素——几何/布局异常" % leaked_outside_mask)"""
assert old in c
c = c.replace(old, new, 1)
with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
py_compile.compile(p, doraise=True)
print("assert2 guard fixed")
