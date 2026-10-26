# 编辑计划 — P0-1 内容层 update_content/clear_content

> 2026-10-23 | 基于对工件 + .trellis/spec/ 指南 + gfx 代码现状的阅读

## 改动文件清单（按顺序）

### 步骤 1: api/handle.rs — Command 枚举 + GlassWindow 方法

- Command 枚举尾部（SetInputSink 之后）追加 UpdateContent / ClearContent 两变体
- GlassWindow::update_content(&self, w_phys, h_phys, buffer, dirty) — 校验+拷贝入队
- GlassWindow::clear_content(&self) — 入队 ClearContent

### 步骤 2: runtime/commands.rs — 命令消费

- apply_one 尾部追加 UpdateContent → content_store.update + request_render
  和 ClearContent → content_store.clear
- destroy 函数中追加 content_store.clear(id)

### 步骤 3: gfx/content_layer.rs — 新建

- ContentStore(HashMap<u32, ContentLayer>) + ContentLayer(tex, srv, w, h)
- 纹理 B8G8R8A8_UNORM / DYNAMIC / CPU_ACCESS_WRITE
- 上传用 Map + 逐行 memcpy（RowPitch 从 Map 返回，不用 w*4）
- 尺寸变化重建；clear 释放

### 步骤 4: Shader — content_vs.hlsl + content_ps.hlsl

- 仿 border pass 模式：全屏三角形 VS + premul 纹理直出 PS
- SdRoundBox 圆角掩码（与 border_ps 相同公式）
- ContentCb: c0=块矩形, c1=纹理尺寸+radius+AA, c2=画布尺寸

### 步骤 5: glass_renderer.rs — draw_contents 方法

- 新增 cvs/cps/ccb/csampler 字段 + 编译
- draw_contents: 仿 draw_borders 循环，查 ContentStore，逐块 Draw(6)

### 步骤 6: compositor.rs — Pass B 接线

- draw_blocks 之后、draw_borders 之前插入 draw_contents
- 层序: 玻璃 → 内容 → border（等价逐块交错）

### 步骤 7: runtime/app.rs — App.content_store

- 新增 content_store: ContentStore 字段

### 步骤 8: Playground GLASS_CONTENT_DEMO

- checker/solid 图案生成；仅构造期读 env

### 步骤 9: tools/verify_content.py — 五条 AC3 断言

### 步骤 10: 单测（校验矩阵 + 状态机）

## D3D11 Gotchas 核对

- Map RowPitch != w*4: 必须用 Map 返回的 RowPitch 做行距
- Hazard: content pass 结束后必须解绑 SRV+OM
- 无其他与内容纹理相关的坑

## 验证门禁

1. cargo test --release -p liquid-verre-rs
2. cargo clippy --workspace --all-targets --release -- -D warnings
3. cargo fmt --all -- --check
4. node scripts/check.mjs
5. GLASS_CONTENT_DEMO=off + cmp_visual.py single 四段全 0
6. verify_dirty x3
7. verify_mouse_isolation / verify_input_sink
8. gate_perf + ab_memory


---

# 二轮编辑计划（2026-10-26 回炉）

## 问题与对策

### P1 叠不等价 -> 真逐块交错
- draw_blocks 加 contents: Option<&ContentStore> 参数；z 序循环内每块 glass Draw 后立即画该块内容。
- 槽位改造避开 glass 状态：ContentCb 挪到 **b1**（glass 只用 b0），内容纹理挪到 **t9**（glass 只用 t0..t8），
  PS 改用 Load(int2) 整数取点（免采样器、1:1 精确、纹理外⇒透明）。
- has_content=false 时：不绑 b1/t9、不切管线，逐帧零新增 API 调用（FR5）。
- border 保持最后整体叠加（天然在内容之上）。
- 临时保留批量 draw_contents 方法（改 t9/b1/Load）用于「批量 vs 交错」判别力自测，采完数后删除。

### P2 verify_content 假门禁倾向 -> 硬 FAIL 五条断言
- ① checker 精确采样 + 16px 周期断言（块内部整数点，逐通道等值，容差 0）。
- ② 叠放（核心）：playground 新 mode=stack/stack_bare（确定性双块，下块内容+上块玻璃重叠 1/2×1/2），
  日志 [verify] stack lower=[..] upper=[..] radius=.. 供脚本定位；
  判据 = 重叠区内采样点 **不得等于裸 checker 纹素**（b==g==r 且 ∈{30,220}）；
  对照数据：批量实现下这些点恰好等于裸 checker 值（实测后写进脚本注释），交错实现下为混入玻璃态。
- ③ clear_seq（push->clear->稳态帧）与 off 稳态帧 **全图等像素**（非 0 即 FAIL）。
- ④ dirty：checker 帧 vs dirty 帧，全图扫描，alpha==255 且子区外差异像素数 == 0（硬），
  子区内差异像素数 > 0（硬）；加测 dirty=Some vs dirty=None 两帧全图 alpha==255 差异 == 0。
- ⑤ resize_seq(clear+重推) / resize_noclear(直接重推) / resize_direct(只推新尺寸)
  三稳态帧 alpha==255 逐点相等（无拉伸残留）。
- 所有判定无 WARN 逃生口：任何不满足 => exit(1)。

### P3 AC2 纯函数
- 新建 core/content.rs（0-unsafe）：plan_update(w,h,buf_len)->{Clear,Upload,TooLarge,LenMismatch}、
  clip_dirty(dirty,w,h)->UploadRegion{Full,Rect{x0..},Empty}；单测矩阵。
- handle.rs 用 plan_update 决策；content_layer.rs 用 clip_dirty 决定上传行区（同尺寸走
  D3D11_MAP_WRITE_NO_OVERWRITE 保内容，仅拷 dirty 行段；尺寸变重建走 WRITE_DISCARD）。
  注：DYNAMIC 资源禁 plain WRITE；UpdateSubresource 不支持 DYNAMIC——两者都是官方禁令，注释写明。

### 其它修正
- ClearContent 消费返回 true（原 false：清除后无触发源则画面不更新——行为 bug）。
- UpdateContent 去掉误置的 region_dirty（内容不改窗口区域）。
- 全文修掉 block''s 之类转义笔误。

## 判别力自测流程（写进报告）
1. 交错版全部写完 -> cargo build --release -> 拷贝 exe 到 research/vc_interleaved.exe（或直接用 target）。
2. 临时 patch 回批量结构（compositor 调 draw_contents、draw_blocks 传 None）-> build ->
   拷贝为 research/vc_batch_probe.exe。
3. 恢复交错源码，还原 patch。
4. verify_content.py VERRE_EXE=probe => ② 必须 FAIL（记录采样数值对照）。
5. VERRE_EXE=正式 => 全绿。
6. 桌面取证窗口串行执行，禁止并发构建。

## 门禁顺序
build/test/clippy/fmt/check.mjs（全部先跑完）-> 桌面取证：
verify_content（交错全绿 + probe FAIL@②）-> cmp_visual single=0 -> verify_dirty×3 ->
mouse_isolation -> input_sink -> gate_perf -> ab_memory + content 显存 delta（research/mem_delta.py）。


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
