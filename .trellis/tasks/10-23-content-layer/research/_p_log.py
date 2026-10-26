import io
p = r"D:\github-project\liquid-verre-rs\.trellis\tasks\10-23-content-layer\research\edit-plan.md"
c = io.open(p, encoding="utf-8").read()
add = """

---

# bring-up 记录（2026-10-26 取证窗口实录）

「内容未渲染」假象的真实根因链（三条，全部已修）：
1. verify_content.py 重写版 read_bmp 的 row_bytes 公式把位当字节（8× 行距）
   → 采样窗口整体错位，读到的全是 backdrop 颜色；
2. content_ps.hlsl 被红屏实验版污染（.keep 备份建立于污染之后，restore 恢复的
   是污染版）→ 块内整片纯蓝；
3. run_shot 清理漏了 .overlay/.capture/.frost/.soft 派生图 → 大小稳定轮询命中
   上一轮陈旧文件。

真实修好的库侧 bug（由硬断言抓到）：
- UpdateSubresource 带 box 时 pSrcData 必须指向子矩形左上角（断言 4 抓到
  「39632 != 65536」）；
- ClearContent 消费返回 false → 静止桌面 clear 不触发重绘（断言 3 会抓）；
- UpdateContent 误置 region_dirty（每帧 SetWindowRgn 重建）；
- shot 帧竞态：静态桌面下渲染门控跳过 shot 之后的帧 → playground 加
  steady_ready + 心跳（no-op SetVisible 触发 request_render）。

判别力自测结论（probe = 批量结构 exe，research/vc_batch_probe.exe）：
- probe：断言 2 FAIL，重叠区 25/25 采样点 = 裸 checker 纹素；
- 交错（正式）：0/25 裸纹素，全 5 条 PASS；
- 上层玻璃 base_alpha≈1 时 stack==bare 是正确物理结果，空断言护栏改用
  「对照点（上块外内容=裸纹素）+ 断言 1 的 378 点全等」。

AC4 显存：off 127.7MB vs checker 144.2MB，delta +16.48MB（3 rep 波动 <0.5MB）；
纹理本体 1200×760×4 = 3.48MiB，其余为 WDDM 系统内存镜像 + 驱动分配，量级可读。
"""
io.open(p, "w", encoding="utf-8", newline="\n").write(c.rstrip() + "\n" + add)
print("edit-plan bring-up logged")
