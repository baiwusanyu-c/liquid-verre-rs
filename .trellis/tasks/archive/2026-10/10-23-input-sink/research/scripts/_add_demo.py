import io, os

# Add the demo to playground/src/main.rs
path = os.path.join(os.getcwd(), "playground", "src", "main.rs")
with io.open(path, "r", encoding="utf-8", newline="") as f:
    content = f.read()

# Find the "fn main()" and add the sink demo code after the demo_pair block
# Add import for GlassInputSink
old_import = """use liquid_verre::{
    DiagnosticsConfig, Engine, EngineConfig, EngineEvent, GlassStyle, ResizeOptions, WindowConfig,
    WindowLevel,
};"""

new_import = """use liquid_verre::{
    DiagnosticsConfig, Engine, EngineConfig, EngineEvent, GlassInputSink, GlassStyle,
    InputDecision, PointerEvent, ResizeOptions, WindowConfig, WindowLevel,
};"""

content = content.replace(old_import, new_import)

# Add sink demo after demo_pair block
old_demo_pair_end = """        log::info!(
            "[playground] 已创建对照窗口：liquid id={} / transparent id={}",
            liquid.id(),
            transparent.id()
        );
    }"""

new_demo_pair_end = """        log::info!(
            "[playground] 已创建对照窗口：liquid id={} / transparent id={}",
            liquid.id(),
            transparent.id()
        );
    }

    // 输入路由 sink demo：创建一个带 sink 的块用于取证
    // GLASS_INPUT_SINK_DEMO=1 时启用；诊断布局模式下自动跳过
    if std::env::var("GLASS_INPUT_SINK_DEMO").ok().as_deref() == Some("1")
        && !layout_from_env()
        && !diagnostics_from_env()
    {
        wait_for_monitors(&engine);
        let [w, h] = primary_size(&engine);
        let demo_rect = [
            w * 0.1,
            h * 0.1,
            (w * 0.3).clamp(200.0, 600.0),
            (h * 0.3).clamp(150.0, 400.0),
        ];
        let demo_win = engine
            .create_window(WindowConfig {
                rect: demo_rect,
                style: Some(GlassStyle::transparent(16.0, 0.5, [0.3, 0.7, 1.0, 1.0])),
                ..Default::default()
            })
            .expect("create sink demo window");
        let sink: std::sync::Arc<dyn GlassInputSink> =
            std::sync::Arc::new(SinkDemo::new(demo_win.id(), demo_rect));
        demo_win.set_input_sink(Some(sink));
        log::info!(
            "[playground] sink demo: id={} rect={:?}",
            demo_win.id(),
            demo_rect
        );
    }"""

content = content.replace(old_demo_pair_end, new_demo_pair_end)

# Add SinkDemo struct before fn main()
old_main_fn = "\nfn main() -> ExitCode {"
sink_demo_struct = """

/// 输入路由 demo sink：记录事件日志 + 块左半区域返回 Consumed。
struct SinkDemo {
    id: u32,
    /// 块矩形 [x, y, w, h]（物理像素）
    rect: [f32; 4],
}

impl SinkDemo {
    fn new(id: u32, rect: [f32; 4]) -> Self {
        Self { id, rect }
    }
}

impl GlassInputSink for SinkDemo {
    fn on_pointer(&self, id: u32, event: &PointerEvent) -> InputDecision {
        // [verify] 前缀日志：供 verify_input_sink.py 取证
        log::info!(
            target: "verify",
            "sink_demo id={} phase={:?} btn={:?} local=[{:.0},{:.0}] wheel={}",
            id,
            event.phase,
            event.button,
            event.block_local[0],
            event.block_local[1],
            event.wheel_delta,
        );
        // 左半区域（x < w/2）返回 Consumed，右半返回 DragMove
        if event.block_local[0] < self.rect[2] * 0.5 {
            InputDecision::Consumed
        } else {
            InputDecision::DragMove
        }
    }
}
"""
content = content.replace(old_main_fn, sink_demo_struct + old_main_fn)

with io.open(path, "w", encoding="utf-8", newline="") as f:
    f.write(content)
print("playground demo added")