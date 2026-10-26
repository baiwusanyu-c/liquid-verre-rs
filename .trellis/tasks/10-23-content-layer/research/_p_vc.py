import io, os
p = r"D:\github-project\liquid-verre-rs\tools\verify_content.py"
with io.open(p, "r", encoding="utf-8-sig") as f:
    c = f.read()

# remove junk placeholder loop in assert4
old = """    lin = rad + 8
    outer = 0
    bad = []
    for y in range(y0 + lin, y0 + (h if False else 0) + 10 ** 9):  # 占位防误改，见下一行
        pass
    for y in range(y0 + lin, y1 - lin):"""
new = """    lin = rad + 8
    outer = 0
    bad = []
    for y in range(y0 + lin, y1 - lin):"""
assert old in c, "junk loop not found"
c = c.replace(old, new, 1)

# header comparison data: remove fabricated example numbers, mark pending actual dump
old_hdr = """判别力对照数据（2026-10-26 实测，stack 场景：下块带 checker 内容 + 上块纯玻璃、
各偏移半块 50% 重叠；重叠内区 5x5 采样）：
  - 批量结构（research/vc_batch_probe.exe，全部玻璃后统一画内容）：
    重叠区采样点命中裸 checker 纹素，例 (1573,1077) BGRA=(30,30,30,255)、
    (1867,1077)=(220,220,220,255) —— 下层内容浮在上层玻璃之上，FR3 违例，
    断言 2 FAIL（脚本抓到了自己声称要防的 bug）。
  - 交错结构（正式实现）：同点为玻璃 premul 混入态，通道互不相等、
    不等于 30/220 两档灰，例 (1573,1077)=(11,10,10,255)、(1867,1077)=(36,41,66,255)，
    断言 2 PASS。
  （数值由 VC_DUMP=1 模式对两个 exe 各跑一遍取得，见 research/overlap-dump.txt。）"""
new_hdr = """判别力对照数据（stack 场景：下块带 checker 内容 + 上块纯玻璃、各偏移半块
50% 重叠；重叠内区 5x5 采样点。实测数值由 VC_DUMP=1 对两个 exe 各跑一遍取得，
回填于下；原始 dump 存 research/overlap-dump.txt）：
  - 批量结构（research/vc_batch_probe.exe，全部玻璃画完后统一画内容）：
    重叠区采样点命中裸 checker 纹素（b==g==r 且 ==30/220）——下层内容
    浮在上层玻璃之上，FR3 违例，断言 2 FAIL。
  - 交错结构（正式实现）：同点为玻璃 premul 混入态（通道互不相等、
    不等于两档灰），断言 2 PASS。
  OVERLAP_DATA: <待首轮 VC_DUMP 实测回填>"""
assert old_hdr in c, "header block not found"
c = c.replace(old_hdr, new_hdr, 1)

with io.open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("verify_content.py cleaned")
import py_compile
py_compile.compile(p, doraise=True)
print("py_compile OK")
