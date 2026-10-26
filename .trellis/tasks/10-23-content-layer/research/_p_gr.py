import io

path = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\gfx\glass_renderer.rs"
with io.open(path, "r", encoding="utf-8") as f:
    c = f.read()

# ---- 1. struct fields: drop csampler ----
old = "    ccb: ID3D11Buffer,\n    csampler: ID3D11SamplerState,\n"
assert old in c, "csampler field not found"
c = c.replace(old, "    ccb: ID3D11Buffer,\n", 1)
print("1: csampler field dropped")

# ---- 2. new(): drop csampler creation ----
old = "        let ccb = make_dynamic_cbuffer(device, (CONTENT_CB_FLOATS * 4) as u32)?;\n        let csampler = make_linear_sampler(device)?;"
assert old in c, "csampler creation not found"
c = c.replace(old, "        let ccb = make_dynamic_cbuffer(device, (CONTENT_CB_FLOATS * 4) as u32)?;", 1)
print("2: csampler creation dropped")

# ---- 3. Ok(Self): drop csampler ----
old = "            ccb,\n            csampler,\n"
assert old in c, "Self csampler not found"
c = c.replace(old, "            ccb,\n", 1)
print("3: Self csampler dropped")

# ---- 4. draw_blocks signature: append contents param ----
old = "        press: f32,\n        hover_id: Option<u32>,\n    ) {"
assert old in c, "signature not found"
new = "        press: f32,\n        hover_id: Option<u32>,\n        contents: Option<&crate::gfx::content_layer::ContentStore>,\n    ) {"
c = c.replace(old, new, 1)
print("4: draw_blocks signature updated")

# ---- 5. prologue: bind ContentCb at CB slot 1 only when store has layers ----
old = "            ctx.PSSetShaderResources(0, Some(&srvs));\n        }\n\n        for &index in order {"
assert old in c, "prologue not found"
new = """            ctx.PSSetShaderResources(0, Some(&srvs));
        }
        // 内容层（P0-1）：只有 store 里真的有层时才多绑一个 CB 槽位（b1）；
        // 空 store => 本帧与重构前逐位一致，零新增 API 调用（FR5 / 第一门禁前提）。
        let store = contents.filter(|s| !s.is_empty());
        if store.is_some() {
            unsafe {
                ctx.VSSetConstantBuffers(1, Some(&[Some(self.ccb.clone())]));
                ctx.PSSetConstantBuffers(1, Some(&[Some(self.ccb.clone())]));
            }
        }

        for &index in order {"""
c = c.replace(old, new, 1)
print("5: prologue conditional b1 bind added")

# ---- 6. per-block: interleave content draw right after glass Draw ----
old = """            unsafe {
                ctx.Draw(6, 0);
            }
            self.drawn.set(self.drawn.get() + 1);
        }

        unsafe {
            let null: [Option<ID3D11ShaderResourceView>; 9] =
                [None, None, None, None, None, None, None, None, None];
            ctx.PSSetShaderResources(0, Some(&null));
            ctx.OMSetRenderTargets(None, None);
        }
    }"""
assert old in c, "per-block tail not found"
new = """            unsafe {
                ctx.Draw(6, 0);
            }
            self.drawn.set(self.drawn.get() + 1);
            // 内容必须**在本块玻璃之后、下一块玻璃之前**画（FR3）。
            // 为什么不是「全部玻璃画完再统一画内容」的批量 pass：那样下层块的内容会
            // 浮在上层块玻璃之上。具体实例：两块 50% 重叠（下块先画），批量结构下重叠区
            // 采到的像素 = 裸 checker 纹素（内容覆盖了上层玻璃的混色结果）；交错结构下
            // = 上层玻璃 premul 混在内容上。verify_content.py 断言 2 就是拿这个对照
            // 数值做的硬判据（2026-10-26 实测数据见该脚本头部注释）。
            // 无内容的块在这里只付一次 HashMap 查找（且仅当 store 非空）；空 store
            // 时 store=None，连查找都没有。
            if let Some(store) = store {
                if let Some(layer) = store.get(block.id) {
                    self.draw_content_for_block(ctx, block, p.corner_radius, layer);
                }
            }
        }

        unsafe {
            let null: [Option<ID3D11ShaderResourceView>; 9] =
                [None, None, None, None, None, None, None, None, None];
            ctx.PSSetShaderResources(0, Some(&null));
            if store.is_some() {
                // t9 只在有内容时被写过；解绑防 hazard（gotchas #3）。
                let null9: [Option<ID3D11ShaderResourceView>; 1] = [None];
                ctx.PSSetShaderResources(9, Some(&null9));
            }
            ctx.OMSetRenderTargets(None, None);
        }
    }

    /// 画单块内容（紧接该块玻璃的 Draw 之后）。
    ///
    /// 与玻璃 pass 共享的东西**都不重绑**：RT、viewport（整画布）、raster（scissor 开）、
    /// premul blend、depth、CB 槽位 0（玻璃 cbuffer）、t0..t8（玻璃纹理）、s0（linear
    /// clamp）。内容只用自己的 b1 cbuffer 与 t9 纹理槽位——PS 用整数 `Load` 取点，
    /// 不经过采样器，所以连 sampler 都不占。切换成本 = 2 次 SetShader + 1 次 SetSRV +
    /// 1 次 CB 更新 + 1 次 scissor + 1 次 Draw，且只对**有内容的块**发生。
    fn draw_content_for_block(
        &self,
        ctx: &ID3D11DeviceContext,
        block: &DrawBlock,
        radius: f32,
        layer: &crate::gfx::content_layer::ContentLayer,
    ) {
        // 纹理锚定块矩形左上角 1:1；锚点取整保证纹素边界与设备像素对齐
        // （取证断言 1 的「精确 checker 值」依赖这一点）。scissor 用同一取整矩形。
        let rx0 = block.x.round() as i32;
        let ry0 = block.y.round() as i32;
        let rw = (block.width.round() as i32).max(1);
        let rh = (block.height.round() as i32).max(1);
        let sx = rx0.max(0);
        let sy = ry0.max(0);
        let ex = rx0.saturating_add(rw).max(0);
        let ey = ry0.saturating_add(rh).max(0);
        let mut cb = ContentCb {
            v: [0.0; CONTENT_CB_FLOATS],
        };
        let set4 = |cb: &mut ContentCb, slot: usize, a: f32, b: f32, c: f32, d: f32| {
            let i = slot * 4;
            cb.v[i] = a;
            cb.v[i + 1] = b;
            cb.v[i + 2] = c;
            cb.v[i + 3] = d;
        };
        set4(&mut cb, 0, rx0 as f32, ry0 as f32, rw as f32, rh as f32);
        // c1.z = 该块本次绘制用的**最终** corner radius（与玻璃 c3.y 同一个
        // p.corner_radius，C5 单一来源——不是第二处半径推导）。
        set4(
            &mut cb,
            1,
            layer.w as f32,
            layer.h as f32,
            radius,
            1.25,
        );
        if update_cbuffer(ctx, &self.ccb, &cb).is_err() {
            return;
        }
        unsafe {
            ctx.PSSetShaderResources(9, Some(&[Some(layer.srv.clone())]));
            ctx.RSSetScissorRects(Some(&[WinRect {
                left: sx,
                top: sy,
                right: ex,
                bottom: ey,
            }]));
            ctx.VSSetShader(&self.cvs, None);
            ctx.PSSetShader(&self.cps, None);
            ctx.Draw(6, 0);
            // 切回玻璃管线：下一块玻璃的 cbuffer(t0..t8/s0/b0) 绑定原样在位，
            // 只需要恢复两个 shader 指针。
            ctx.VSSetShader(&self.vs, None);
            ctx.PSSetShader(&self.ps, None);
        }
    }"""
c = c.replace(old, new, 1)
print("6: per-block interleave + helper added")

with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("glass_renderer.rs written")
