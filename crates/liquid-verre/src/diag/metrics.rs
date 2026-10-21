//! 帧预算统计与 `[metrics]` 行输出（方案 7.6 口径）。
//!
//! `tools/perf_probe.py` / `tools/gate_perf.py` 依赖这里的**行格式与字段名**，
//! 改字段必须同步改探针（见 `.trellis/spec/backend/logging-guidelines.md`）。

use std::time::Instant;

use crate::runtime::app::App;

/// 帧预算统计（方案 7.6）。全部按 5 秒墙钟窗口结算，与「这一窗口渲染了多少帧」
/// 无关 —— 否则静止时窗口会被无限拉长，让 avg/max 失去可比性。
pub(crate) struct Metrics {
    epoch: Instant,
    loops: u64,
    frames: u64,
    idle_wakes: u64,
    max_loop_us: u128,
    render_sum_us: u128,
    render_max_us: u128,
    dirty_sum: i64,
    dirty_max: i64,
    blocks_sum: i64,
    pass_a: u64,
    pass_a_px: i64,
    pass_a_throttled: u64,
    copied_px: i64,
    acquired: u64,
    content: u64,
    /// 主循环分段 CPU 耗时累加：消息泵 / 采集 / 渲染前准备
    msg_sum_us: u128,
    cap_sum_us: u128,
    cap_max_us: u128,
    prep_sum_us: u128,
    /// cap 段细分：AcquireNextFrame / 脏矩形 API / 像素搬运 / 取帧前置 / ReleaseFrame
    acq_sum_us: u128,
    acq_max_us: u128,
    dirtyapi_sum_us: u128,
    copy_sum_us: u128,
    pre_sum_us: u128,
    rel_sum_us: u128,
    rel_max_us: u128,
    /// 逐次循环采样 [总耗时, 采集段, 渲染段]。
    /// 每个窗口只有几十次循环时，一次几百毫秒的离群值就能把均值彻底带偏
    /// （实测出现过"跳过工作反而更慢"的因果倒置），所以改报分位数。
    samples: Vec<[u128; 3]>,
}

impl Metrics {
    pub(crate) fn new() -> Self {
        Self {
            epoch: Instant::now(),
            loops: 0,
            frames: 0,
            idle_wakes: 0,
            max_loop_us: 0,
            render_sum_us: 0,
            render_max_us: 0,
            dirty_sum: 0,
            dirty_max: 0,
            blocks_sum: 0,
            pass_a: 0,
            pass_a_px: 0,
            pass_a_throttled: 0,
            copied_px: 0,
            acquired: 0,
            content: 0,
            msg_sum_us: 0,
            cap_sum_us: 0,
            cap_max_us: 0,
            prep_sum_us: 0,
            acq_sum_us: 0,
            acq_max_us: 0,
            dirtyapi_sum_us: 0,
            copy_sum_us: 0,
            pre_sum_us: 0,
            rel_sum_us: 0,
            rel_max_us: 0,
            samples: Vec::new(),
        }
    }
}

/// `stage_us` = [消息泵, 采集, 渲染前准备]
pub(crate) fn report_metrics(
    app: &mut App,
    m: &mut Metrics,
    loop_us: u128,
    render_us: u128,
    rendered: bool,
    stage_us: [u128; 3],
) {
    m.loops += 1;
    m.max_loop_us = m.max_loop_us.max(loop_us);
    m.msg_sum_us += stage_us[0];
    m.cap_sum_us += stage_us[1];
    m.cap_max_us = m.cap_max_us.max(stage_us[1]);
    m.prep_sum_us += stage_us[2];
    if m.samples.len() < 20000 {
        m.samples.push([loop_us, stage_us[1], render_us]);
    }
    if rendered {
        m.frames += 1;
        m.render_sum_us += render_us;
        m.render_max_us = m.render_max_us.max(render_us);
    } else {
        m.idle_wakes += 1;
    }
    for mon in app.monitors.iter_mut() {
        if rendered {
            m.dirty_sum += mon.compositor.last_dirty_px;
            m.dirty_max = m.dirty_max.max(mon.compositor.last_dirty_px);
            m.blocks_sum += mon.compositor.last_drew_blocks as i64;
            if mon.compositor.last_pass_a {
                m.pass_a += 1;
            }
            m.pass_a_throttled += mon.compositor.pass_a_throttled;
            mon.compositor.pass_a_throttled = 0;
            m.pass_a_px += mon.compositor.last_pass_a_px;
        }
        m.copied_px += mon.copied_px;
        mon.copied_px = 0;
        if let Some(cap) = mon.capture.as_mut() {
            let (a, c) = cap.take_stats();
            m.acquired += a;
            m.content += c;
            let t = cap.take_timings();
            m.acq_sum_us += t[0];
            m.dirtyapi_sum_us += t[1];
            m.copy_sum_us += t[2];
            m.pre_sum_us += t[3];
            m.rel_sum_us += t[4];
            m.acq_max_us = m.acq_max_us.max(t[5]);
            m.rel_max_us = m.rel_max_us.max(t[6]);
        }
    }

    let secs = m.epoch.elapsed().as_secs_f64();
    if secs < 5.0 {
        return;
    }
    // 分位数：对离群值稳健
    fn pct(v: &mut [u128], p: f64) -> f64 {
        if v.is_empty() {
            return 0.0;
        }
        v.sort_unstable();
        let i = ((v.len() as f64 - 1.0) * p).round() as usize;
        v[i.min(v.len() - 1)] as f64 / 1000.0
    }
    let mut s_loop: Vec<u128> = m.samples.iter().map(|x| x[0]).collect();
    let mut s_cap: Vec<u128> = m.samples.iter().map(|x| x[1]).collect();
    let mut s_ren: Vec<u128> = m.samples.iter().map(|x| x[2]).filter(|x| *x > 0).collect();
    let loop_p50 = pct(&mut s_loop, 0.50);
    let loop_p95 = pct(&mut s_loop, 0.95);
    let cap_p50 = pct(&mut s_cap, 0.50);
    let cap_p95 = pct(&mut s_cap, 0.95);
    let ren_p50 = pct(&mut s_ren, 0.50);
    let ren_p95 = pct(&mut s_ren, 0.95);
    let n = m.frames.max(1) as f64;
    let nl = m.loops.max(1) as f64;
    let screens = app.monitors.len().max(1) as f64;
    let stage = app.monitors[0].compositor.last_stage_us;
    let (gpu_avg, gpu_max, gpu_n) = match app.monitors[0].compositor.gpu.as_mut() {
        Some(gpu) => gpu.take_stats(),
        None => ([0.0; 3], [0.0; 3], 0),
    };
    log::info!(target: "metrics", "win_s={:.2} screens={} blocks={} loops={} frames={} fps={:.1} idle={} \
         avg_render_ms={:.3} max_render_ms={:.3} max_loop_ms={:.3} \
         stage_us=A{} B{} P{} C{} \
         loop_ms_avg=msg{:.3}/cap{:.3}/prep{:.3} cap_max_ms={:.3} \
         p50_ms=loop{:.2}/cap{:.2}/ren{:.2} p95_ms=loop{:.2}/cap{:.2}/ren{:.2} \
         cap_split_ms=acq{:.3}/dirtyApi{:.3}/copy{:.3}/pre{:.3}/rel{:.3} \
         acq_max_ms={:.3} rel_max_ms={:.3} \
         dirty_mpx_avg={:.3} dirty_mpx_max={:.3} \
         blocks_avg={:.1} pass_a={:.2}/frame pass_a_throttled={:.2}/frame \
         pass_a_mpx_avg={:.3} \
         dda_acq={:.1}/s dda_changed={:.1}/s copy_mpx_avg={:.3} \
         gpu_ms_avg=A{:.3}/B{:.3}/T{:.3} gpu_ms_max=A{:.3}/B{:.3}/T{:.3} gpu_n={}",
        secs,
        app.monitors.len(),
        app.manager.len(),
        m.loops,
        m.frames,
        m.frames as f64 / secs,
        m.idle_wakes,
        m.render_sum_us as f64 / n / 1000.0,
        m.render_max_us as f64 / 1000.0,
        m.max_loop_us as f64 / 1000.0,
        stage[0],
        stage[1],
        stage[2],
        stage[3],
        m.msg_sum_us as f64 / nl / 1000.0,
        m.cap_sum_us as f64 / nl / 1000.0,
        m.prep_sum_us as f64 / nl / 1000.0,
        m.cap_max_us as f64 / 1000.0,
        loop_p50,
        cap_p50,
        ren_p50,
        loop_p95,
        cap_p95,
        ren_p95,
        m.acq_sum_us as f64 / nl / 1000.0,
        m.dirtyapi_sum_us as f64 / nl / 1000.0,
        m.copy_sum_us as f64 / nl / 1000.0,
        m.pre_sum_us as f64 / nl / 1000.0,
        m.rel_sum_us as f64 / nl / 1000.0,
        m.acq_max_us as f64 / 1000.0,
        m.rel_max_us as f64 / 1000.0,
        m.dirty_sum as f64 / n / screens / 1e6,
        m.dirty_max as f64 / 1e6,
        m.blocks_sum as f64 / n / screens,
        m.pass_a as f64 / n,
        m.pass_a_throttled as f64 / n,
        m.pass_a_px as f64 / n / screens / 1e6,
        m.acquired as f64 / secs,
        m.content as f64 / secs,
        m.copied_px as f64 / n / screens / 1e6,
        gpu_avg[0],
        gpu_avg[1],
        gpu_avg[2],
        gpu_max[0],
        gpu_max[1],
        gpu_max[2],
        gpu_n,
    );
    *m = Metrics::new();
}
