//! 鼠标跟踪：GetCursorPos 轮询（方案 6.3）。无全局 hook，避免输入链延迟。

use windows::Win32::Foundation::POINT;
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

pub fn cursor_pos() -> POINT {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    p
}
