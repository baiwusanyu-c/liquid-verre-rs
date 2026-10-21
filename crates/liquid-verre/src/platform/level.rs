//! 画布窗口的层级实现（需求 F4）。
//!
//! 四种语义：
//! - `AlwaysOnTop`：`WS_EX_TOPMOST` + `HWND_TOPMOST`（**默认**，与重构前逐字一致）；
//! - `Normal`：去掉 TOPMOST，普通 popup，随其它窗口正常遮挡；
//! - `ClickToFront`：`Normal` + 点击命中玻璃块时 `HWND_TOP`（不抢焦点）；
//! - `AlwaysOnBottom`：`HWND_BOTTOM`，在所有普通窗口之下（仍在桌面之上，否则玻璃不可见）。
//!
//! 注意事项：
//! - 一律带 `SWP_NOACTIVATE`：画布带 `WS_EX_NOACTIVATE`，抬升时绝不能把焦点从宿主抢走
//!   （方案 需求 2「不抢焦点」）。
//! - 一律带 `SWP_NOMOVE | SWP_NOSIZE`：层级操作不应触碰几何，否则会白白让 DWM 重新裁剪。
//! - 画布是 `WS_EX_NOREDIRECTIONBITMAP` + DComp 承载，`SetWindowPos` 只改 Z 序，
//!   不会触发重定向位图重建，因此**不需要** `invalidate()`。

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, HWND_BOTTOM, HWND_NOTOPMOST,
    HWND_TOP, HWND_TOPMOST, SET_WINDOW_POS_FLAGS, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_SHOWWINDOW, WS_EX_TOPMOST,
};

use crate::api::config::WindowLevel;

const FLAGS: SET_WINDOW_POS_FLAGS =
    SET_WINDOW_POS_FLAGS(SWP_NOMOVE.0 | SWP_NOSIZE.0 | SWP_NOACTIVATE.0);

/// 把层级对应的 `WS_EX_TOPMOST` 扩展样式写到窗口上（不改变 Z 序）。
///
/// 返回样式是否发生了变化 —— 只有变化时才需要真的调用 `SetWindowPos`。
pub(crate) fn sync_topmost_style(hwnd: HWND, level: WindowLevel) -> bool {
    unsafe {
        // 读样式必须用 GetWindowLongPtrW：SetWindowLongPtrW 是「写并返回旧值」，
        // 拿它当 getter 会先把样式清成 0，中间任何一次 DWM 重排都会看到错误样式。
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u64;
        let has = style & u64::from(WS_EX_TOPMOST.0) != 0;
        let want = level.is_topmost();
        if has == want {
            return false;
        }
        let next = if want {
            style | u64::from(WS_EX_TOPMOST.0)
        } else {
            style & !u64::from(WS_EX_TOPMOST.0)
        };
        let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, next as isize);
        true
    }
}

/// 应用层级（Z 序 + 扩展样式）。
pub(crate) fn apply(hwnd: HWND, level: WindowLevel, visible: bool) {
    // 返回值刻意忽略：见下方注释，「样式没变」不再是跳过 SetWindowPos 的理由。
    sync_topmost_style(hwnd, level);
    let insert_after = match level {
        WindowLevel::AlwaysOnTop => HWND_TOPMOST,
        WindowLevel::AlwaysOnBottom => HWND_BOTTOM,
        WindowLevel::Normal | WindowLevel::ClickToFront => HWND_NOTOPMOST,
    };
    let mut flags = FLAGS;
    if visible {
        flags = SET_WINDOW_POS_FLAGS(flags.0 | SWP_SHOWWINDOW.0);
    }
    // 注意：**不能**按「扩展样式变没变」来决定要不要 SetWindowPos。
    // AlwaysOnBottom 与 Normal 都不带 WS_EX_TOPMOST，样式层面「没变」，
    // 但 Z 序一个在底一个在中 —— 早退会让 bottom -> normal 的恢复永远发不出去
    // （独立审查 P2-2）。调用方（commands::SetWindowLevel）已经做过
    // 「level 与上次相同」的幂等短路，这里每次真改层级都发一次 SetWindowPos 是正确的。
    unsafe {
        let _ = SetWindowPos(hwnd, insert_after, 0, 0, 0, 0, flags);
    }
}

/// 「点击到最前」：把画布抬到普通窗口层的最上面（不抢焦点、不进 TOPMOST）。
pub(crate) fn raise_on_click(hwnd: HWND) {
    unsafe {
        let _ = SetWindowPos(hwnd, HWND_TOP, 0, 0, 0, 0, FLAGS);
    }
}

#[cfg(test)]
mod tests {
    use super::WindowLevel;

    #[test]
    fn topmost_only_for_always_on_top() {
        assert!(WindowLevel::AlwaysOnTop.is_topmost());
        assert!(!WindowLevel::Normal.is_topmost());
        assert!(!WindowLevel::ClickToFront.is_topmost());
        assert!(!WindowLevel::AlwaysOnBottom.is_topmost());
    }

    #[test]
    fn click_to_front_is_the_only_raising_level() {
        let raising = [
            WindowLevel::AlwaysOnTop,
            WindowLevel::Normal,
            WindowLevel::ClickToFront,
            WindowLevel::AlwaysOnBottom,
        ];
        let count = raising.iter().filter(|l| l.raises_on_click()).count();
        assert_eq!(count, 1);
        assert!(WindowLevel::ClickToFront.raises_on_click());
    }
}
