# -*- coding: utf-8 -*-
"""patch playground main.rs: wire SinkDemo (attach + capture log + state heartbeat)."""
import io

P = r"D:/github-project/liquid-verre-rs/playground/src/main.rs"
t = io.open(P, encoding="utf-8", newline="\n").read()

# 1) imports: add PointerPhase
old_imp = "    InputDecision, PointerEvent, ResizeOptions, WindowConfig, WindowLevel,"
new_imp = "    InputDecision, PointerEvent, PointerPhase, ResizeOptions, WindowConfig, WindowLevel,"
assert old_imp in t, "import anchor missing"
t = t.replace(old_imp, new_imp, 1)

# 2) replace the (dead) SinkDemo section
i = t.index("/// \u8f93\u5165\u8def\u7531 demo sink")
j = t.index("fn main() -> ExitCode {")
NEW_DEMO = '''/// \u8f93\u5165\u8def\u7531 demo sink\uff1a\u8bb0\u5f55\u4e8b\u4ef6\u65e5\u5fd7 + \u5757\u5de6\u534a\u8fd4\u56de Consumed\u3001\u53f3\u534a\u8fd4\u56de DragMove\u3002
/// \u4ec5\u5728 GLASS_INPUT_SINK_DEMO=1 \u65f6\u6302\u8f7d\uff08\u4f9b tools/verify_input_sink.py \u53d6\u8bc1\uff09\uff0c\u9ed8\u8ba4\u4e0d\u5b58\u5728\u3002
struct SinkDemo {
    id: u32,
    /// \u5224\u5b9a\u8fb9\u754c\uff08\u7269\u7406\u50cf\u7d20\uff09\uff1a\u5757\u5bbd\u4e00\u534a\u4ee5\u5de6 = Consumed\u3002\u6302\u8f7d\u65f6\u7528\u903b\u8f91 rect \u00d7 scale \u6362\u7b97\u3002
    half_w_phys: f32,
}

impl SinkDemo {
    fn new(id: u32, half_w_phys: f32) -> Self {
        Self { id, half_w_phys }
    }
}

// GetCapture \u53ea\u5728\u62e5\u6709\u6355\u83b7\u7684\u7ebf\u7a0b\u91cc\u6709\u610f\u4e49\uff08\u8de8\u7ebf\u7a0b\u8fd4\u56de NULL\uff09\u3002sink \u56de\u8c03\u606f\u5728\u6e32\u67d3\u7ebf\u7a0b\uff08canvas
// wnd_proc \u4e0a\u4e0b\u6587\uff09\u6267\u884c\u2014\u2014\u8fd9\u6b63\u662f\u201c\u6309\u538b\u540e\u5e93\u6709\u6ca1\u6709 SetCapture\u201d\u7684\u6b63\u786e\u53d6\u8bc1\u4f4d\u7f6e\u3002
// playground \u4e0d\u662f\u5e93\uff0c\u8fd9\u91cc\u76f4\u63a5\u7ed1 user32 \u800c\u4e0d\u5f15\u5165 windows crate\uff08\u53d6\u8bc1\u4e13\u7528\u7684\u552f\u4e00 unsafe \u70b9\uff09\u3002
extern "system" {
    fn GetCapture() -> isize;
}

impl GlassInputSink for SinkDemo {
    fn on_pointer(&self, id: u32, event: &PointerEvent) -> InputDecision {
        let cap = unsafe { GetCapture() };
        // \u5de6\u534a\u533a\u57df\u8fd4\u56de Consumed\uff0c\u53f3\u534a\u8fd4\u56de DragMove\uff08\u5750\u6807\u662f\u5757\u5185\u7269\u7406\u50cf\u7d20\uff09
        let decision = if event.block_local[0] < self.half_w_phys {
            InputDecision::Consumed
        } else {
            InputDecision::DragMove
        };
        // [verify] \u524d\u7f00\u65e5\u5fd7\uff1a\u4f9b verify_input_sink.py \u53d6\u8bc1\u3002Down \u624d\u8bb0\u51b3\u7b56\uff08\u5176\u4f59\u9636\u6bb5\u8fd4\u56de\u503c
        // \u65e0\u8bed\u4e49\uff0c\u5237\u51b3\u7b56\u5b57\u6bb5\u4f1a\u668f\u65e5\u5fd7\u9762\u79ef\uff09\uff1bcap= \u8bb0\u5f55\u4e8b\u4ef6\u77ac\u95f4\u7684 capture \u5f52\u5c5e\u3002
        let dec_tag = match event.phase {
            PointerPhase::Down => format!(" dec={:?}", decision),
            _ => String::new(),
        };
        log::info!(
            target: "verify",
            "sink_demo id={} phase={:?} btn={:?} local=[{:.0},{:.0}] wheel={} cap={}{}",
            id,
            event.phase,
            event.button,
            event.block_local[0],
            event.block_local[1],
            event.wheel_delta,
            cap,
            dec_tag,
        );
        decision
    }
}

'''
t = t[:i] + NEW_DEMO + t[j:]

# 3) attach block before panel::start
anchor = "    let panel = match panel::start(Arc::clone(&engine), opts.port, opts.no_browser) {"
assert anchor in t, "panel anchor missing"
ATTACH = '''    // GlassInputSink demo \u6302\u8f7d\uff1aGLASS_INPUT_SINK_DEMO=1 \u65f6\u7ed9\u300c\u552f\u4e00\u9ed8\u8ba4\u5757\u300d\u63a5 sink \u5e76\u8d77 rect \u5fc3\u8df3\u3002
    // \u53d6\u8bc1\u811a\u672c\u7528 GLASS_DEFAULT_BLOCK=1\uff08layout_from_env \u547d\u4e2d \u21d2 playground \u81ea\u5df1\u4e0d\u518d\u5efa\uff0c\u5168\u7a0b\u6070\u597d\u4e00\u5757\uff09\uff1b
    // \u51fa\u73b0\u591a\u7a97\u53e3\u5c31\u62d2\u7edd\u6302\u8f7d\u2014\u2014\u6302\u9519\u5757\u7684\u53d6\u8bc1\u662f\u5047\u8bc1\u636e\u3002
    if std::env::var_os("GLASS_INPUT_SINK_DEMO").is_some() {
        wait_for_monitors(&engine);
        let wins = engine.windows();
        if wins.len() != 1 {
            log::warn!(
                "[playground] GLASS_INPUT_SINK_DEMO \u9700\u8981\u5355\u5757\u5e03\u5c40\uff0c\u5f53\u524d wins={}\uff0c\u4e0d\u6302\u8f7d",
                wins.len()
            );
        } else if let Some(s) = wins[0].snapshot() {
            let half = s.rect[2] * s.scale * 0.5;
            wins[0].set_input_sink(Some(Arc::new(SinkDemo::new(s.id, half))));
            log::info!(
                target: "verify",
                "sink_demo_attached id={} half_w_phys={:.0} wins=1 scale={:.3}",
                s.id,
                half,
                s.scale
            );
            // rect \u5fc3\u8df3\uff1a\u811a\u672c\u636e\u6b64\u5224\u5b9a\u300c\u5757\u52a8\u6ca1\u52a8/\u5c3a\u5bf8\u53d8\u6ca1\u53d8\u300d\u2014\u2014\u8fd9\u662f\u8def\u7531\u2192\u4f1a\u8bdd\u2192\u72b6\u6001
            // \u94fe\u8def\u7684\u884c\u4e3a\u8bc1\u636e\uff1b\u50cf\u7d20\u4fa7\u7531 cmp_visual/verify_dirty \u7684\u96f6\u5dee\u5f02\u95e8\u7981\u515c\u5e95\u3002
            let w2 = wins[0].clone();
            std::thread::spawn(move || loop {
                if let Some(s) = w2.snapshot() {
                    log::info!(
                        target: "verify",
                        "sink_demo_state id={} rect=[{:.1},{:.1},{:.1},{:.1}] scale={:.3} visible={}",
                        s.id,
                        s.rect[0],
                        s.rect[1],
                        s.rect[2],
                        s.rect[3],
                        s.scale,
                        s.visible
                    );
                }
                std::thread::sleep(Duration::from_millis(400));
            });
        } else {
            log::warn!("[playground] GLASS_INPUT_SINK_DEMO: \u9ed8\u8ba4\u5757 snapshot \u4e0d\u53ef\u7528\uff0c\u4e0d\u6302\u8f7d");
        }
    }

'''
t = t.replace(anchor, ATTACH + anchor, 1)

io.open(P, "w", encoding="utf-8", newline="\n").write(t)
print("patched playground/src/main.rs, bytes=", len(t))
