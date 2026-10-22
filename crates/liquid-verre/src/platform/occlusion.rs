//! Win32 遮挡扫描层：枚举 z 序在自家画布**之上**的不透明窗口矩形。
//!
//! 为什么单独开一个模块（而不是塞进 `display.rs` / `window.rs`）：这是 `platform/` 里唯一
//! 「每 125 ms 扫一遍全桌面 z 序」的调用点，成本模型与那两个「启动期查一次 / 事件驱动」的
//! 模块完全不同 —— 回调里的判定顺序（便宜的在前、跨进程的 `DwmGetWindowAttribute` 在最后）
//! 本身就是性能契约，值得独立成文。
//!
//! ## 成本模型（R-B1-3：检测不得进每帧热路径）
//!
//! `opaque_rects_above` 由 `runtime::occlusion::update` 以 **8 Hz**（125 ms）调用，每屏一次。
//! 默认层级 `AlwaysOnTop` 下 `EnumWindows` 几乎立刻撞上自家画布（`stop_at`）并终止，扫描成本
//! 接近 0；真正会枚举到几十个窗口的是 `Normal` / `ClickToFront` / `AlwaysOnBottom`。
//! 回调里的关卡顺序是**契约**，不要为了「看起来更整齐」而重排：
//!
//! 1. `hwnd == stop_at` → 终止枚举（z 序再往下的窗口在我们之下，盖不住画布的任何像素）
//! 2. 自家另一块屏的画布 → 跳过但**继续**枚举
//! 3. `GetWindowRect`（一次内核调用）+ 零面积 / 相交测试（纯算术）
//! 4. `IsWindowVisible` / `IsIconic` / `GetWindowLongPtrW`（都只是读窗口字段，最廉价的一档）
//! 5. `GetWindowRgn`（要先 `CreateRectRgn` 造一个 HRGN，用完 `DeleteObject`）
//! 6. `DwmGetWindowAttribute`（**唯一的跨进程调用**，走到 DWM 进程，最贵 ⇒ 放最后；
//!    调用点是 `display::dwm_cloaked`，全仓只有那一处 —— R-E4 单一来源）
//!
//! ## 保守方向（错判方向的取舍）
//!
//! 一切「不确定」都判「不遮挡」（宁可多渲染，不可少渲染，R-B1-4）：查询失败、探测用 HRGN
//! 造不出来、DWM 拒绝回答 —— 全部当作「这窗口可能是半透明的」。错判成「遮挡」会直接停掉
//! 该块的桌面采集，症状是遮挡移开后玻璃里露出 ≤125 ms 的陈旧桌面；错判成「不遮挡」只是
//! 白干一点活。前者难查得多，所以判据一律往保守那边倒。
//!
//! ## panic 安全（为什么这里**不装** `catch_unwind`）
//!
//! 根 `Cargo.toml` 的 release profile 是 `panic = "abort"`：回调里一旦 panic，
//! `catch_unwind` 根本不会返回 `Err`，进程直接 abort —— 装它等于写一段骗人的死代码。
//! 所以这里改成让回调**结构上不可能 panic**：无数组索引、无 `unwrap` / `expect`、
//! 无切片截取、无格式化、无 `i32` 算术；所有来自 Win32 的整数一律 widen 到 `i64`
//! 且**不再窄化回去**（debug 构建下裸 `i32` 加减溢出即 panic，而窗口坐标是不可信输入：
//! 最小化窗口给 `-32000` 这类还原位置，多屏虚拟桌面能给到 ±2^31 量级）。
//!
//! 于是复用 `core::geometry` 的判据时也走 i64 版（[`rect_is_empty_i64`]），而不是先把
//! 坐标窄化回 `[i32; 4]` —— R-D2 的「矩形运算单一来源 = `core/geometry.rs`」与
//! 本模块的「不窄化」两条规矩同时满足。注意「widen 防溢出」这个理由只对**算术**成立：
//! 纯比较本来就不会溢出，那里保持 i64 只是为了纪律一致、少一次转换。
//!
//! 本模块**不打日志**：8 Hz 轮询下任何周期性输出都是噪声，翻转沿日志由 `runtime::occlusion`
//! 负责（前缀即接口，见 `logging-guidelines.md`；本模块不新增 `RAW_TARGETS`）。

use windows::Win32::Foundation::{BOOL, FALSE, HWND, LPARAM, RECT, TRUE};
use windows::Win32::Graphics::Gdi::{CreateRectRgn, DeleteObject, GetWindowRgn, RGN_ERROR};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowLongPtrW, GetWindowRect, IsIconic, IsWindowVisible, GWL_EXSTYLE,
    WS_EX_LAYERED, WS_EX_TRANSPARENT,
};

use crate::core::geometry::rect_is_empty_i64;

use super::display::dwm_cloaked;
use super::window::is_own_window;

/// 「不可算不透明遮挡者」的扩展样式位掩码。
///
/// - `WS_EX_LAYERED`：可能是 `SetLayeredWindowAttributes` 的整窗 alpha < 255，也可能是
///   逐像素 alpha 的位图窗口 —— 两种都**不是**不透明矩形。
///   ⚠ 已知取舍（design §4.2）：alpha = 255 的 layered 窗口同样被否决，代价是漏剔一部分
///   本可以省的（例如某些播放器的 OSD）。方向保守，可接受。
/// - `WS_EX_TRANSPARENT`：点击穿透窗口通常也不画满（我们自己的画布就带这一位）。
const OCCLUDER_VETO_EXSTYLE: u32 = WS_EX_LAYERED.0 | WS_EX_TRANSPARENT.0;

/// 由窗口样式位与几个已查得的布尔量判定「能否算不透明遮挡者」。
///
/// 抽成纯函数是为了单测：真实 HWND 无法在 CI 里构造，而判据本身是 AC8 要覆盖的逻辑。
///
/// 任一条件命中即否决：
/// - `!visible`：不可见窗口不参与合成。
/// - `iconic`：最小化窗口的 `GetWindowRect` 给的是 `-32000` 附近的还原位置，几何上毫无意义。
/// - `has_rgn`：异形窗口（`SetWindowRgn`）的不透明区**不是**它的窗口矩形；用矩形判「完整覆盖」
///   会造成**假遮挡** —— 矩形里那些被裁掉的洞其实看得见玻璃，我们会因此停掉采集。
/// - `cloaked`：UWP 挂起 / 被 DWM 隐藏的窗口仍然 `IsWindowVisible == true` 却一个像素都不画，
///   这是 `DWMWA_CLOAKED` 存在的唯一理由。
/// - `exstyle` 命中 [`OCCLUDER_VETO_EXSTYLE`]：见该常量的注释。
///
/// 本函数**不看**几何（相交与「完整覆盖」分别由回调的相交测试和
/// `core::geometry::rect_fully_covered` 负责），也**不看** z 序
/// （`EnumWindows` 的枚举顺序就是 z 序，上界由 `opaque_rects_above` 的 `stop_at` 给出）。
#[must_use]
pub(crate) fn is_opaque_occluder(
    exstyle: u32,
    visible: bool,
    iconic: bool,
    has_rgn: bool,
    cloaked: bool,
) -> bool {
    if !visible || iconic || has_rgn || cloaked {
        return false;
    }
    exstyle & OCCLUDER_VETO_EXSTYLE == 0
}

/// `EnumWindows` 回调的上下文。
///
/// 字段全是标量与引用：`out` 由调用方持有并**跨帧复用**（热路径零分配 —— 8 Hz × 每屏一次，
/// 每次新建 `Vec` 就是纯浪费，还会让分配器在渲染线程上抖）。
struct ScanCtx<'a> {
    /// 本屏自家画布：z 序下界，撞到它就终止枚举。
    stop_at: HWND,
    /// 本屏所有块绘制矩形的并集（虚拟屏绝对物理像素，已 widen 到 `i64`）。
    bounds: [i64; 4],
    out: &'a mut Vec<[i32; 4]>,
}

/// `EnumWindows` 回调：按「便宜的在前、跨进程的在最后」的顺序过关卡（见模块文档）。
///
/// SAFETY（不变量由谁保证）：
/// - `lparam` 是 [`opaque_rects_above`] 里那个栈上 `ScanCtx` 的地址。`EnumWindows` 是**同步**的，
///   回调不会在它返回之后被调用，因此该引用的生命周期完整覆盖全部回调调用。
/// - 回调体不含索引 / `unwrap` / 窄整数加减，不会 panic（panic 穿越 FFI 边界 +
///   `panic = "abort"` 会直接带走整个渲染进程，见模块文档「panic 安全」）。
unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut ScanCtx<'_>);

    // 关卡 1：撞到本屏自家画布 → z 序下界，**终止**枚举。
    if hwnd == ctx.stop_at {
        return FALSE;
    }
    // 关卡 2：自家**另一块屏**的画布可能排在本屏画布之上。语义上必须跳过（它不是遮挡者），
    // 但**不能**终止枚举 —— 别的 topmost 窗口可能还在更上面。
    // `is_own_window` 是**零分配**的：直接比宽字符常量 `CANVAS_CLASS_NAME_W`，
    // 不走 `String::from_utf16_lossy`。本回调对桌面上**每个**窗口都要调它一次
    // （8 Hz × 每屏），任务硬约束「`EnumWindows` 回调里不分配 `String` / `Vec`」
    // 卡的就是这里。它内部对 `GetClassNameW` 的返回值做了 clamp，切片也不会越界。
    if is_own_window(hwnd) {
        return TRUE;
    }

    // 关卡 3：几何。一次 `GetWindowRect` + 纯算术，查询失败就跳过（不确定 ⇒ 不遮挡）。
    let mut wr = RECT::default();
    if GetWindowRect(hwnd, &mut wr).is_err() {
        return TRUE;
    }
    // 全部 widen 到 i64 再算：外部窗口坐标是不可信输入，i32 减法在 debug 下溢出即 panic。
    let l = i64::from(wr.left);
    let t = i64::from(wr.top);
    let r = i64::from(wr.right);
    let b = i64::from(wr.bottom);
    let [bl, bt, br, bb] = ctx.bounds;
    // 零面积（含 right < left 的退化矩形）不可能覆盖任何东西。
    // 判据**不在本文件手写**：走 `core::geometry::rect_is_empty_i64`。R-D2 是硬规矩
    // （矩形运算单一来源 = `core/geometry.rs`，禁止在 `platform/` 或 `runtime/` 再写一份），
    // 而 `rect_is_empty` 的文档自己也写着「别处不要再手写 `x1 <= x0 || y1 <= y0`」。
    // 用 i64 版而不是先窄化回 `[i32; 4]`：窄化一次就把「结构上不可能 panic」戳了个洞。
    if rect_is_empty_i64([l, t, r, b]) {
        return TRUE;
    }
    // 半开区间相交测试：相邻边恰好接触（r == bl）**不**算相交，与 `core::geometry` 同口径。
    // 先做相交测试再查样式位，是因为这一步只花几条比较指令，却能筛掉绝大多数桌面窗口。
    if r <= bl || l >= br || b <= bt || t >= bb {
        return TRUE;
    }

    // 关卡 4：可见性 / 最小化 / 扩展样式位（都只是读窗口字段）。
    let visible = IsWindowVisible(hwnd).as_bool();
    let iconic = IsIconic(hwnd).as_bool();
    let exstyle = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;

    // 同一个判据函数分三次调用，是为了「贵的查询只在便宜关卡全过之后才付」（R-B1-3）。
    // 传 `false` 的含义是「该项尚未查询，不参与否决」，判据本身仍然只有一份（单一来源）。
    if !is_opaque_occluder(exstyle, visible, iconic, false, false) {
        return TRUE;
    }

    // 关卡 5：窗口区域。
    // ⚠ windows 0.58 的签名是 `GetWindowRgn(hwnd, hrgn) -> GDI_REGION_TYPE`（照搬 Win32：
    //   区域**由调用方提供**，函数把窗口的区域拷进去），所以必须先 `CreateRectRgn(0,0,0,0)`
    //   造一个空 HRGN 当探针，用完**必须** `DeleteObject`。
    // ⚠ 静默 GDI 句柄泄漏：每个候选窗口漏一个，8 Hz 下几分钟就把进程 GDI 配额（默认 10000）
    //   打满；症状是后续 `CreateRectRgn` / 建窗口全部失败，而且不会报成我们的错误。
    let probe = CreateRectRgn(0, 0, 0, 0);
    let has_rgn = if probe.is_invalid() {
        // 连探针 HRGN 都造不出来（GDI 配额吃紧）⇒ 保守判「有区域」= 不算遮挡者。
        true
    } else {
        // SAFETY: probe 是上一行刚由 CreateRectRgn 造出的有效 HRGN，只在本分支内使用，
        // 且两条控制流都在 DeleteObject 之后才离开（没有早退路径），因此不会泄漏也不会双重释放。
        let kind = GetWindowRgn(hwnd, probe);
        let _ = DeleteObject(probe);
        // RGN_ERROR(0) = 该窗口没有区域；NULLREGION / SIMPLEREGION / COMPLEXREGION 都算异形。
        // （NULLREGION 理论上意味着窗口整个不可见，同样保守否决，不省这一次判断。）
        kind != RGN_ERROR
    };
    if !is_opaque_occluder(exstyle, visible, iconic, has_rgn, false) {
        return TRUE;
    }

    // 关卡 6：唯一的跨进程调用 —— 走到 DWM 进程问「这窗口是不是被 cloak 了」。
    // 放最后是因为前面 5 关已经筛掉了绝大多数窗口，而一次 DWM 往返比前面所有调用加起来都贵。
    // `DwmGetWindowAttribute` 的调用点在 `display::dwm_cloaked`（全仓唯一，R-E4），它**故意**
    // 返回 `Option<bool>` 而不是 `bool`：查询失败时两个调用方的保守方向**相反** ——
    // 本回调要「失败 ⇒ 当 cloaked ⇒ 不算遮挡者」（宁可多渲染，见模块文档「保守方向」），
    // 而 `display::fullscreen_app_on` 要「失败 ⇒ 当未 cloaked ⇒ 维持既有判定」（不误伤真游戏）。
    // 语义与内联版本逐字一致。`unwrap_or(true)` 不是 `unwrap()`：它不引入任何 panic 面，
    // 「回调结构上不可能 panic」这条性质保持不变。
    let cloaked = dwm_cloaked(hwnd).unwrap_or(true);
    if !is_opaque_occluder(exstyle, visible, iconic, has_rgn, cloaked) {
        return TRUE;
    }

    // 通过全部关卡：追加进调用方复用的缓冲区（唯一的写入点）。
    ctx.out.push([wr.left, wr.top, wr.right, wr.bottom]);
    TRUE
}

/// 枚举 z 序在 `stop_at` **之上**、且与 `bounds` 相交的**不透明**窗口矩形
/// （虚拟屏绝对物理像素，半开区间 `[x0, y0, x1, y1]`）。返回本次调用追加进 `out` 的矩形数。
///
/// ⚠ 本函数**只追加、不清空** `out`：调用方每屏 `clear()` 一次再传进来（design §2.6），
/// 这样多屏扫描能共用同一个跨帧缓冲区。清空后调用时，返回值等于 `out.len()`。
///
/// 返回 0 的三种情况都表示「没有遮挡者」，语义等价，调用方不必区分：
/// 1. `stop_at` 无效：拿不到自家画布 HWND 时**根本不扫** —— z 序没有下界，枚举全桌面既贵
///    又语义错（会把画布之下的窗口也算成遮挡者，进而错停采集）。
/// 2. `bounds` 退化（零面积 / 反序）：本屏没有块，没有可被覆盖的目标。
/// 3. 真的没有符合条件的窗口 —— 默认 `AlwaysOnTop` 层级下的常态（design §4.4）。
pub(crate) fn opaque_rects_above(
    stop_at: HWND,
    bounds: [i32; 4],
    out: &mut Vec<[i32; 4]>,
) -> usize {
    if stop_at.is_invalid() {
        return 0;
    }
    let [x0, y0, x1, y1] = bounds;
    // widen 到 i64 之后**全程不再窄化**：bounds 是若干绘制矩形并集 + 阴影外扩的结果，
    // 极端多屏布局下这些数可以贴近 i32 边界。⚠ 这里只有比较、比较不会溢出，
    // 所以 widen 是为了「本模块不出现窄化转换」这条纪律的一致性，**不是**防溢出
    // （真正必须 widen 的是加减法，见 `runtime::occlusion::to_abs`）。别把这条注释读歪。
    let wb = [i64::from(x0), i64::from(y0), i64::from(x1), i64::from(y1)];
    // bounds 退化（零面积 / 反序）⇒ 本屏没有可被覆盖的目标，根本不进 EnumWindows。
    // 判据同样走 core 的单一来源（R-D2）；本模块刻意不出现索引表达式，
    // `rect_is_empty_i64` 内部用解构，「不可能 panic」仍然是结构性质而不只是注释里的承诺。
    if rect_is_empty_i64(wb) {
        return 0;
    }
    let before = out.len();
    let mut ctx = ScanCtx {
        stop_at,
        bounds: wb,
        out,
    };
    // SAFETY: enum_cb 只在本次 EnumWindows 调用内被同步回调；ctx 是栈上局部变量，
    // 生命周期完整覆盖这次调用，lparam 也只被 enum_cb 还原成同一个 &mut ScanCtx。
    let _ = unsafe { EnumWindows(Some(enum_cb), LPARAM(&mut ctx as *mut ScanCtx<'_> as isize)) };
    // 只会 push，不会缩短，因此这个减法不会下溢。
    ctx.out.len() - before
}

#[cfg(test)]
mod tests {
    use super::{is_opaque_occluder, OCCLUDER_VETO_EXSTYLE};
    use windows::Win32::UI::WindowsAndMessaging::{
        WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
    };

    /// 「干净」的普通窗口：非 layered、非 transparent。
    const PLAIN: u32 = 0;

    #[test]
    fn a_plain_visible_window_is_an_occluder() {
        assert!(
            is_opaque_occluder(PLAIN, true, false, false, false),
            "可见 + 无区域 + 未 cloak + 无否决样式位 ⇒ 算不透明遮挡者"
        );
    }

    #[test]
    fn invisible_or_minimised_windows_never_occlude() {
        assert!(
            !is_opaque_occluder(PLAIN, false, false, false, false),
            "不可见窗口不参与合成"
        );
        assert!(
            !is_opaque_occluder(PLAIN, true, true, false, false),
            "最小化窗口的 GetWindowRect 是 -32000 附近的还原位置，几何上无意义"
        );
    }

    #[test]
    fn windows_with_a_region_never_occlude() {
        assert!(
            !is_opaque_occluder(PLAIN, true, false, true, false),
            "异形窗口的矩形里有洞，用矩形判完整覆盖会假遮挡"
        );
    }

    #[test]
    fn cloaked_windows_never_occlude() {
        assert!(
            !is_opaque_occluder(PLAIN, true, false, false, true),
            "UWP 挂起窗口仍然 IsWindowVisible，但一个像素都不画"
        );
    }

    #[test]
    fn layered_windows_never_occlude_even_when_fully_opaque() {
        // AC5（半透明窗口覆盖时不许错剔）在纯逻辑层的对应断言：
        // layered 一律否决，即使它 alpha = 255（design §4.2 的已知取舍）。
        assert!(!is_opaque_occluder(
            WS_EX_LAYERED.0,
            true,
            false,
            false,
            false
        ));
    }

    #[test]
    fn click_through_windows_never_occlude() {
        assert!(!is_opaque_occluder(
            WS_EX_TRANSPARENT.0,
            true,
            false,
            false,
            false
        ));
        // 我们自己的画布就是这两位一起（create_canvas: NOACTIVATE | TOOLWINDOW | TRANSPARENT）
        assert!(!is_opaque_occluder(
            WS_EX_LAYERED.0 | WS_EX_TRANSPARENT.0,
            true,
            false,
            false,
            false
        ));
    }

    #[test]
    fn harmless_exstyle_bits_do_not_veto() {
        // TOPMOST / TOOLWINDOW / NOACTIVATE 都不影响「不透明」这个几何事实，
        // 否则默认层级下连任务栏这类 topmost 窗口都会被漏剔（AC4 的正向用例就废了）。
        let ex = WS_EX_TOPMOST.0 | WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0;
        assert!(is_opaque_occluder(ex, true, false, false, false));
    }

    #[test]
    fn veto_mask_is_exactly_layered_and_transparent() {
        // 掩码漂移的后果：半透明窗口被错判成遮挡者 → 停采 → 遮挡移开后露出旧桌面。
        // 这类 bug 没有编译错误、没有崩溃，只有一个极难复现的间歇现象，所以钉死数值。
        assert_eq!(OCCLUDER_VETO_EXSTYLE, WS_EX_LAYERED.0 | WS_EX_TRANSPARENT.0);
        assert_eq!(WS_EX_LAYERED.0, 0x0008_0000, "WS_EX_LAYERED 位值");
        assert_eq!(WS_EX_TRANSPARENT.0, 0x0000_0020, "WS_EX_TRANSPARENT 位值");
        assert_eq!(
            OCCLUDER_VETO_EXSTYLE & WS_EX_TOPMOST.0,
            0,
            "topmost 不得被否决"
        );
    }
}
