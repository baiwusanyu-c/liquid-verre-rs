//! D3D11 GPU timestamp 计时（《折射渲染方案》7.6 明确要求的测量手段）。
//!
//! 旧的 `last_stage_us` 只是 CPU 提交耗时，无法解释 100 ms 级主循环长尾；
//! 这里用 `D3D11_QUERY_TIMESTAMP_DISJOINT` + `D3D11_QUERY_TIMESTAMP` 打点，
//! 延迟 DEPTH 帧回读，回读时用 `DONOTFLUSH`，因此**永不阻塞渲染线程**。
//! 仅在 `GLASS_METRICS=1` 时创建，不参与任何渲染结果。

use windows::Win32::Graphics::Direct3D11::*;

/// 打点位置：0=帧开始 1=Pass A 之后 2=Pass B 之后 3=Present/Commit 之后
pub const MARKS: usize = 4;
/// 环形深度：GPU 通常允许 1-3 帧在飞，取 4 保证回读时数据已就绪
const DEPTH: usize = 4;

struct Frame {
    disjoint: ID3D11Query,
    stamps: [Option<ID3D11Query>; MARKS],
}

pub struct GpuTimer {
    frames: Vec<Frame>,
    index: usize,
    started: bool,
    /// 最近一次成功回读的 GPU 毫秒数：[Pass A, Pass B, 帧总计]
    pub last_ms: [f64; 3],
    /// 历史最大值，用于抓长尾
    pub max_ms: [f64; 3],
    pub samples: u64,
    acc_ms: [f64; 3],
}

fn make_query(
    device: &ID3D11Device,
    kind: D3D11_QUERY,
) -> crate::error::Result<Option<ID3D11Query>> {
    let desc = D3D11_QUERY_DESC {
        Query: kind,
        MiscFlags: 0,
    };
    let mut q = None;
    unsafe {
        device
            .CreateQuery(&desc, Some(&mut q))
            .map_err(|e| format!("CreateQuery: {e}"))?;
    }
    Ok(q)
}

impl GpuTimer {
    pub fn new(device: &ID3D11Device) -> crate::error::Result<Self> {
        let mut frames = Vec::with_capacity(DEPTH);
        for _ in 0..DEPTH {
            let disjoint =
                make_query(device, D3D11_QUERY_TIMESTAMP_DISJOINT)?.ok_or("disjoint query")?;
            let mut stamps: [Option<ID3D11Query>; MARKS] = Default::default();
            for slot in stamps.iter_mut() {
                *slot = make_query(device, D3D11_QUERY_TIMESTAMP)?;
            }
            frames.push(Frame { disjoint, stamps });
        }
        Ok(Self {
            frames,
            index: 0,
            started: false,
            last_ms: [0.0; 3],
            max_ms: [0.0; 3],
            samples: 0,
            acc_ms: [0.0; 3],
        })
    }

    /// 帧开始：先尝试回收 DEPTH-1 帧之前的结果，再开新的一段。
    pub fn begin_frame(&mut self, ctx: &ID3D11DeviceContext) {
        let resolve_at = (self.index + 1) % DEPTH;
        self.resolve(ctx, resolve_at);
        self.index = resolve_at;
        unsafe {
            ctx.Begin(&self.frames[self.index].disjoint);
        }
        self.started = true;
    }

    pub fn mark(&mut self, ctx: &ID3D11DeviceContext, slot: usize) {
        if !self.started || slot >= MARKS {
            return;
        }
        if let Some(q) = &self.frames[self.index].stamps[slot] {
            unsafe {
                ctx.End(q);
            }
        }
    }

    pub fn end_frame(&mut self, ctx: &ID3D11DeviceContext) {
        if !self.started {
            return;
        }
        unsafe {
            ctx.End(&self.frames[self.index].disjoint);
        }
    }

    fn resolve(&mut self, ctx: &ID3D11DeviceContext, idx: usize) {
        let frame = &self.frames[idx];
        let mut disjoint = D3D11_QUERY_DATA_TIMESTAMP_DISJOINT::default();
        let hr = unsafe {
            ctx.GetData(
                &frame.disjoint,
                Some(&mut disjoint as *mut _ as *mut _),
                std::mem::size_of::<D3D11_QUERY_DATA_TIMESTAMP_DISJOINT>() as u32,
                D3D11_ASYNC_GETDATA_DONOTFLUSH.0 as u32,
            )
        };
        if hr.is_err() || disjoint.Disjoint.as_bool() || disjoint.Frequency == 0 {
            return;
        }
        let mut t = [0u64; MARKS];
        // 循环里既要读 frame.stamps[i] 又要写 t[i]，并行走两个数组用下标最直白。
        #[allow(clippy::needless_range_loop)]
        for i in 0..MARKS {
            let Some(q) = &frame.stamps[i] else { return };
            let mut v: u64 = 0;
            let hr = unsafe {
                ctx.GetData(
                    q,
                    Some(&mut v as *mut u64 as *mut _),
                    std::mem::size_of::<u64>() as u32,
                    D3D11_ASYNC_GETDATA_DONOTFLUSH.0 as u32,
                )
            };
            if hr.is_err() {
                return;
            }
            t[i] = v;
        }
        if t[0] == 0 || t[3] <= t[0] {
            return;
        }
        let freq = disjoint.Frequency as f64;
        let ms = |a: u64, b: u64| (b.saturating_sub(a)) as f64 * 1000.0 / freq;
        let a = ms(t[0], t[1]);
        let b = ms(t[1], t[2]);
        let total = ms(t[0], t[3]);
        self.last_ms = [a, b, total];
        self.max_ms[0] = self.max_ms[0].max(a);
        self.max_ms[1] = self.max_ms[1].max(b);
        self.max_ms[2] = self.max_ms[2].max(total);
        self.acc_ms[0] += a;
        self.acc_ms[1] += b;
        self.acc_ms[2] += total;
        self.samples += 1;
    }

    /// 取出并清空「上次调用以来」的 GPU 均值与峰值。
    pub fn take_stats(&mut self) -> ([f64; 3], [f64; 3], u64) {
        let n = self.samples.max(1) as f64;
        let avg = [self.acc_ms[0] / n, self.acc_ms[1] / n, self.acc_ms[2] / n];
        let out = (avg, self.max_ms, self.samples);
        self.acc_ms = [0.0; 3];
        self.max_ms = [0.0; 3];
        self.samples = 0;
        out
    }
}
