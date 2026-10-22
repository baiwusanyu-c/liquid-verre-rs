//! 上层命令在渲染线程上的消费点。
//!
//! 全部命令在**帧首一次性**消费（方案 12.2.3「输入合并」的同一套思路）：
//! 上层在一帧内连发 100 条 `set_style`，也只有最后一条生效，不会渲染 100 次。
//!
//! 返回 `true` 表示「本批命令改变了需要重画的东西」，主循环据此触发一帧。

use crate::api::config::GlassStyle;
use crate::api::event::EngineEvent;
use crate::api::handle::{Command, StylePatch};
use crate::error::ErrorKind;
use crate::platform::window;
use crate::runtime::app::{spawn_block, sync_block_monitor, App, SpawnSpec};

/// 消费待处理命令。
pub(crate) fn apply(app: &mut App) -> bool {
    app.shared.drain_into(&mut app.cmd_buf);
    if app.cmd_buf.is_empty() {
        return false;
    }
    // 先把缓冲区整个取出来：`app.cmd_buf.drain(..)` 会在遍历期间一直借用 app，
    // 而 apply_one 需要 &mut app。取走再放回，热路径依然零分配。
    let mut buf = std::mem::take(&mut app.cmd_buf);
    let mut changed = false;
    for cmd in buf.drain(..) {
        changed |= apply_one(app, cmd);
    }
    app.cmd_buf = buf;
    changed
}

fn apply_one(app: &mut App, cmd: Command) -> bool {
    match cmd {
        Command::Shutdown => {
            app.shutdown = true;
            false
        }
        Command::SetDefaultStyle(style) => {
            let prev_mode = app.default_style.mode();
            app.default_style = style;
            // 继承默认样式的块必须换 epoch，否则样式缓存会返回旧参数。
            let mut bumped = false;
            for b in app.manager.blocks_mut() {
                if b.inherits_style {
                    b.style_epoch = b.style_epoch.wrapping_add(1);
                    bumped = true;
                }
            }
            app.snapshot_dirty |= bumped;
            // 默认样式变化可能不改任何块的 draw_rect（例如只改 ior / tint_color），
            // 于是脏区三来源全部判不出差异、render() 在 dirty==None 处早退 ——
            // 静止桌面上拖面板滑杆会「不生效」（独立审查 P1-1b，此前靠浏览器自己
            // 重绘桌面顺手刷帧才没暴露）。另外 mode 翻转（透明<->液态）还会让
            // 金字塔账本过期，两种情况统一用 invalidate 兜住。
            if bumped || prev_mode != style.mode() {
                invalidate_all(app);
            } else {
                invalidate_present_all(app);
            }
            true
        }
        Command::SetWindowLevel(level) => {
            if app.level == level {
                return false;
            }
            app.level = level;
            window::apply_level(&app.monitors, level);
            app.shared.emit(EngineEvent::LevelApplied { level });
            app.snapshot_dirty = true;
            // 层级变化会改变遮挡关系，桌面采集内容随之变化，金字塔必须重算。
            for m in app.monitors.iter_mut() {
                m.compositor.invalidate();
            }
            true
        }
        Command::SetDefaultResize(options) => {
            app.default_resize = options;
            app.snapshot_dirty = true;
            false
        }
        Command::Create { id, config } => {
            spawn_block(app, SpawnSpec::new(id, config));
            // 新建的块可能需要桌面背景（液态模式），金字塔账本必须作废，
            // 否则第一帧会沿用「透明屏时期」的陈旧内容。
            invalidate_all(app);
            true
        }
        Command::Destroy { id } => destroy(app, id),
        Command::DestroyAll => {
            let ids: Vec<u32> = app.manager.blocks().iter().map(|b| b.id).collect();
            if ids.is_empty() {
                return false;
            }
            app.manager.clear();
            app.style_cache.clear();
            app.hover = None;
            app.hover_handle = Default::default();
            app.drag = None;
            app.press = 0.0;
            for m in app.monitors.iter_mut() {
                m.region_dirty = true;
            }
            app.snapshot_dirty = true;
            for id in ids {
                app.shared.emit(EngineEvent::WindowDestroyed { id });
            }
            true
        }
        Command::SetRect { id, rect } => {
            let Some(scale) = app.manager.get(id).map(|b| b.scale.max(1e-3)) else {
                return warn_missing(app, id, "set_rect");
            };
            if let Some(b) = app.manager.get_mut(id) {
                b.x = rect[0];
                b.y = rect[1];
                b.width = rect[2] * scale;
                b.height = rect[3] * scale;
                b.clamp();
            }
            finish_geometry(app, id)
        }
        Command::SetSize { id, width, height } => {
            let Some(scale) = app.manager.get(id).map(|b| b.scale.max(1e-3)) else {
                return warn_missing(app, id, "set_size");
            };
            if !width.is_finite() || !height.is_finite() {
                return false;
            }
            if let Some(b) = app.manager.get_mut(id) {
                b.width = (width * scale).max(crate::core::block::MIN_SIZE);
                b.height = (height * scale).max(crate::core::block::MIN_SIZE);
                b.clamp();
            }
            finish_geometry(app, id)
        }
        Command::SetPosition { id, x, y } => {
            if !x.is_finite() || !y.is_finite() {
                return false;
            }
            if let Some(b) = app.manager.get_mut(id) {
                b.x = x;
                b.y = y;
                b.clamp();
            } else {
                return warn_missing(app, id, "set_position");
            }
            finish_geometry(app, id)
        }
        Command::SetStyle { id, style } => {
            let Some(b) = app.manager.get_mut(id) else {
                return warn_missing(app, id, "set_style");
            };
            match style {
                Some(mut s) => {
                    s.sanitize();
                    b.style = s;
                    b.inherits_style = false;
                }
                None => {
                    b.style = app.default_style;
                    b.inherits_style = true;
                }
            }
            b.style_epoch = b.style_epoch.wrapping_add(1);
            // 模式切换可能让「本屏是否需要桌面采集」翻转，金字塔账本必须作废。
            invalidate_all(app);
            app.snapshot_dirty = true;
            true
        }
        Command::SetResizable { id, enabled } => {
            let Some(b) = app.manager.get_mut(id) else {
                return warn_missing(app, id, "set_resizable");
            };
            if b.resize.enabled == enabled {
                return false;
            }
            b.resize.enabled = enabled;
            for m in app.monitors.iter_mut() {
                m.region_dirty = true;
            }
            app.snapshot_dirty = true;
            true
        }
        Command::PatchStyle { id, patch } => {
            let Some(b) = app.manager.get_mut(id) else {
                return warn_missing(app, id, "patch_style");
            };
            // 在**渲染线程的权威状态**上做读-改-写：上层连续调
            // set_corner_radius + set_opacity 时，两条命令按入队顺序叠加，
            // 不会像「从快照读基线」那样互相覆盖（独立审查 P2-4）。
            let mut style = *b.effective_style(&app.default_style);
            match (&mut style, patch) {
                (GlassStyle::Liquid(p), StylePatch::CornerRadius(v)) => p.corner_radius = v,
                (GlassStyle::Liquid(p), StylePatch::Opacity(v)) => p.base_alpha = v,
                (GlassStyle::Liquid(p), StylePatch::TintColor(c)) => p.tint_color = c,
                (GlassStyle::Transparent(t), StylePatch::CornerRadius(v)) => t.corner_radius = v,
                (GlassStyle::Transparent(t), StylePatch::Opacity(v)) => t.opacity = v,
                (GlassStyle::Transparent(t), StylePatch::TintColor(c)) => t.color = c,
            }
            style.sanitize();
            // 单独 patch 过某个字段 = 上层要这块窗口独立于引擎默认样式，
            // 因此固化并摘掉继承（这个副作用写进了 GlassWindow 的文档）。
            b.style = style;
            b.inherits_style = false;
            b.style_epoch = b.style_epoch.wrapping_add(1);
            invalidate_all(app);
            app.snapshot_dirty = true;
            true
        }
        Command::SetResize { id, options } => {
            let Some(b) = app.manager.get_mut(id) else {
                return warn_missing(app, id, "set_resize_options");
            };
            b.resize = options;
            for m in app.monitors.iter_mut() {
                m.region_dirty = true;
            }
            app.snapshot_dirty = true;
            true
        }
        Command::SetZ { id, order } => {
            if !app.manager.contains(id) {
                return warn_missing(app, id, "set_z_order");
            }
            let changed = app.manager.set_z(id, order);
            if changed {
                app.snapshot_dirty = true;
                // z 序不改任何块的 draw_rect()，脏区三来源判不出差异 -> 不重画。
                // 必须显式作废 present 账本（独立审查 P0-1：此前 bring_to_front()
                // 端到端无效，只有等下一次别的触发源顺手刷帧才看得见）。
                invalidate_present_all(app);
            }
            changed
        }
        Command::SetVisible { id, visible } => {
            let Some(b) = app.manager.get_mut(id) else {
                return warn_missing(app, id, "set_visible");
            };
            if b.visible == visible {
                return false;
            }
            b.visible = visible;
            // 遮挡状态必须跟着复位：不可见块**不进** `mon.blocks`（投影阶段就跳过了），
            // 因此 8 Hz 扫描根本不会更新它。不复位的话 `set_visible(true)` 之后会有
            // ≤125 ms 沿用陈旧的 `occluded = true` —— 那段时间该块不驱动采集，
            // 玻璃里是旧桌面。这正是本项目最忌讳的「间歇、难复现」类缺陷
            // （只在「隐藏期间遮挡者恰好移走」时出现），必须在源头掐掉。
            b.occluded = false;
            b.occl_streak = 0;
            // 隐藏 / 显示都会改变「本屏是否需要采集」，金字塔账本作废。
            invalidate_all(app);
            for m in app.monitors.iter_mut() {
                m.region_dirty = true;
            }
            app.snapshot_dirty = true;
            true
        }
        Command::SetMovable { id, movable } => {
            let Some(b) = app.manager.get_mut(id) else {
                return warn_missing(app, id, "set_movable");
            };
            if b.movable == movable {
                return false;
            }
            b.movable = movable;
            // 正在移动中的块被设为不可移动：立刻结束会话，否则它会继续跟光标
            if !movable
                && app.drag.as_ref().map(|d| d.id()) == Some(id)
                && !app.drag.as_ref().is_some_and(|d| d.is_resize())
            {
                app.drag = None;
                app.press = 0.0;
                app.geometry_changed = true;
            }
            app.snapshot_dirty = true;
            false
        }
        Command::SetInteractive { id, interactive } => {
            let Some(b) = app.manager.get_mut(id) else {
                return warn_missing(app, id, "set_interactive");
            };
            if b.interactive == interactive {
                return false;
            }
            b.interactive = interactive;
            app.snapshot_dirty = true;
            false
        }
    }
}

fn destroy(app: &mut App, id: u32) -> bool {
    if !app.manager.remove(id) {
        return warn_missing(app, id, "close");
    }
    app.style_cache.forget(id);
    if app.hover == Some(id) {
        app.hover = None;
        app.hover_handle = Default::default();
    }
    if app.drag.as_ref().map(|d| d.id()) == Some(id) {
        app.drag = None;
        app.press = 0.0;
    }
    for m in app.monitors.iter_mut() {
        m.region_dirty = true;
    }
    app.snapshot_dirty = true;
    app.shared.emit(EngineEvent::WindowDestroyed { id });
    true
}

/// 几何变化后统一收尾：跨屏归位 + 区域重建 + 快照失效。
fn finish_geometry(app: &mut App, id: u32) -> bool {
    sync_block_monitor(app, id);
    for m in app.monitors.iter_mut() {
        m.region_dirty = true;
    }
    app.snapshot_dirty = true;
    true
}

fn invalidate_all(app: &mut App) {
    for m in app.monitors.iter_mut() {
        m.compositor.invalidate();
    }
}

/// 只作废 present 账本（整屏重画一帧），不动金字塔账本。
fn invalidate_present_all(app: &mut App) {
    for m in app.monitors.iter_mut() {
        m.compositor.invalidate_present();
    }
}

/// 句柄指向的窗口已经不存在：打一次告警（可被上层捕获），不 panic、不返回 Err。
fn warn_missing(app: &App, id: u32, op: &str) -> bool {
    app.shared.emit(EngineEvent::Warning {
        kind: ErrorKind::InvalidConfig,
        message: format!("{op}: 窗口 {id} 不存在（可能已被销毁）"),
    });
    false
}

#[cfg(test)]
mod tests {
    // 命令消费需要完整的 GPU / 窗口环境，无法在纯单测里跑；
    // 纯逻辑部分（z 序、命中测试、样式展开、缩放几何）分别在
    // core::manager / core::handle / core::style / input::drag 里覆盖。
    // 这里只钉住「命令枚举本身可比较、可克隆」这个契约。
    use crate::api::config::{ResizeOptions, WindowLevel, ZOrder};
    use crate::api::handle::Command;

    #[test]
    fn commands_are_clone_and_comparable() {
        let c = Command::SetZ {
            id: 1,
            order: ZOrder::Front,
        };
        assert_eq!(c.clone(), c);
        assert_ne!(c, Command::SetWindowLevel(WindowLevel::Normal));
        assert_ne!(
            Command::SetResize {
                id: 1,
                options: ResizeOptions::default()
            },
            Command::SetResize {
                id: 1,
                options: ResizeOptions {
                    edges: false,
                    ..Default::default()
                }
            }
        );
    }
}
