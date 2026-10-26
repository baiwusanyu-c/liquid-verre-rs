import io, py_compile
p = r"D:\github-project\liquid-verre-rs\tools\verify_content.py"
with io.open(p, "r", encoding="utf-8") as f:
    c = f.read()
old = """  OVERLAP_DATA: <待首轮 VC_DUMP 实测回填>"""
new = """  OVERLAP_DATA（2026-10-26 实测，重叠内区 5x5 网格，节选 (x,y) 处 stack 值）：
    probe 批量: (1652,1110)=(220,220,220,255) (1860,1110)=(30,30,30,255)
                (1964,1110)=(30,30,30,255)   (2068,1350)=(30,30,30,255)
                —— 25/25 全部命中裸纹素（b==g==r∈{30,220}）=> 断言2 FAIL
    interleaved: (1652,1110)=(94,108,125,255) (1860,1110)=(142,98,129,255)
                 (1964,1110)=(104,120,105,255) (2068,1350)=(134,126,105,255)
                —— 0/25 裸纹素（通道互异、非两档灰）=> 断言2 PASS
    两版 bare 基线值一致（如 (1652,1110)=(94,108,125,255)）：说明上层玻璃
    base_alpha≈1 时重叠区 stack==bare 是正确物理结果，判据只用「非裸纹素」，
    空断言护栏 = 对照点（上块外内容必为裸纹素）+ 断言1 的 378 点全等。
    原始 dump：research/overlap-dump-probe.txt / overlap-dump-interleaved.txt"""
assert old in c
c = c.replace(old, new, 1)
with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
py_compile.compile(p, doraise=True)
print("OVERLAP_DATA filled")
