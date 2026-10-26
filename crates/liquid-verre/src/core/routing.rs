//! 输入路由决策状态机：把手带/块命中/sink 决策 → 动作 + 按压归属。
//!
//! 0 unsafe、0 Win32 依赖，因此全部可单测。
//!
//! # 决策顺序（硬规则，来自父 design §2）
//!
//! 1. 缩放把手带（`hit_resize`）优先级恒高于 sink —— 命中即库缩放会话，不询问 sink。
//! 2. 块矩形内命中（`hit_test`，z 序最高）→ 询问该块 sink 的 Down 决策。
//! 3. 无 sink → 默认移动会话（DragMove）。
//!
//! # 按压粘滞
//!
//! Down 后的同一按压序列 Move/Up 粘滞转发给 owner 块；
//! 未按压时事件转发光标所在最上层块。

use crate::core::handle::ResizeEdges;

/// 按压归属：Down 后同一序列的 Move/Up 粘滞转发给此块。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct PressOwner {
    /// 归属块 id。
    pub block: u32,
    /// Down 时 sink 是否返回 Consumed。true 时不建拖拽会话、不 SetCapture。
    pub consumed: bool,
}

/// 路由决策动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RouteAction {
    /// 开始移动会话。
    StartMove { block: u32 },
    /// 开始缩放会话。
    StartResize { block: u32, edges: ResizeEdges },
    /// 转发事件给 sink（sink_block 为目标块 id）。
    Forward { sink_block: u32 },
    /// 消费事件（不产生会话，不 SetCapture）。
    Consume,
    /// 无操作（块外点击等）。
    Noop,
}

/// 路由状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RouteState {
    /// 没有按压归属。
    Idle,
    /// 有按压归属（序列粘滞）。
    Pressing(PressOwner),
}

/// 指针阶段（core 内部简化版，不含按钮细节）。
///
/// 状态机只关心阶段类别；按钮/滚轮值由 platform 层在 sink 调用时注入。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PhaseKind {
    Down,
    Up,
    Move,
    DoubleClick,
    RightDown,
    Wheel,
}

/// sink 对 Down 的决策（core 内部简化版）。
///
/// `None` = 未询问 sink（无 sink、或把手命中、或非 Down 阶段）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SinkDecision {
    /// 消费：不建会话。
    Consumed,
    /// 启动移动会话。
    DragMove,
    /// 交给库默认（= DragMove）。
    Ignored,
}

/// 输入路由决策的输入参数。
#[derive(Debug, Clone, Copy)]
pub(crate) struct RouteInput {
    /// `hit_resize` 的结果：`(block_id, edges)` 或 None。
    pub hit_resize: Option<(u32, ResizeEdges)>,
    /// `hit_test` 的结果：z 序最高的命中块 id 或 None。
    pub hit_test: Option<u32>,
    /// sink 决策（仅当 hit_test 命中且块有 sink 且阶段为 Down 时有值）。
    pub sink_decision: Option<SinkDecision>,
    /// 事件阶段。
    pub phase: PhaseKind,
}

/// 路由决策结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct RouteResult {
    /// 要执行的动作。
    pub action: RouteAction,
    /// 新状态。
    pub state: RouteState,
}

/// 路由决策主函数。
///
/// # 参数
/// - `state`：当前按压归属状态。
/// - `input`：命中测试结果与 sink 决策。
///
/// # 返回
/// 动作 + 新状态。调用方据此执行动作并更新状态。
pub(crate) fn route(state: RouteState, input: RouteInput) -> RouteResult {
    match state {
        RouteState::Idle => route_idle(input),
        RouteState::Pressing(owner) => route_pressing(owner, input),
    }
}

/// 空闲态路由：无按压归属时的决策。
fn route_idle(input: RouteInput) -> RouteResult {
    match input.phase {
        PhaseKind::Down => route_down_idle(input),
        PhaseKind::Up | PhaseKind::RightDown => {
            // 右键和未按压时的 Up：转发给光标所在块（如果有的话）
            match input.hit_test {
                Some(block) => RouteResult {
                    action: RouteAction::Forward { sink_block: block },
                    state: RouteState::Idle,
                },
                None => RouteResult {
                    action: RouteAction::Noop,
                    state: RouteState::Idle,
                },
            }
        }
        PhaseKind::Move | PhaseKind::DoubleClick | PhaseKind::Wheel => {
            // 转发给光标所在最上层块
            match input.hit_test {
                Some(block) => RouteResult {
                    action: RouteAction::Forward { sink_block: block },
                    state: RouteState::Idle,
                },
                None => RouteResult {
                    action: RouteAction::Noop,
                    state: RouteState::Idle,
                },
            }
        }
    }
}

/// 空闲态 Down 事件路由。
fn route_down_idle(input: RouteInput) -> RouteResult {
    // 规则 1：把手带优先
    if let Some((block, edges)) = input.hit_resize {
        return RouteResult {
            action: RouteAction::StartResize { block, edges },
            state: RouteState::Idle, // 缩放不设 press owner
        };
    }

    // 规则 2：块内命中
    if let Some(block) = input.hit_test {
        match input.sink_decision {
            Some(SinkDecision::Consumed) => {
                // 消费：记 press owner，不开移动会话
                return RouteResult {
                    action: RouteAction::Consume,
                    state: RouteState::Pressing(PressOwner {
                        block,
                        consumed: true,
                    }),
                };
            }
            Some(SinkDecision::DragMove) | Some(SinkDecision::Ignored) | None => {
                // 移动会话（含无 sink 的默认行为）
                return RouteResult {
                    action: RouteAction::StartMove { block },
                    state: RouteState::Pressing(PressOwner {
                        block,
                        consumed: false,
                    }),
                };
            }
        }
    }

    // 规则 3：块外
    RouteResult {
        action: RouteAction::Noop,
        state: RouteState::Idle,
    }
}

/// 按压态路由：有 owner 时的决策。
fn route_pressing(owner: PressOwner, input: RouteInput) -> RouteResult {
    match input.phase {
        PhaseKind::Down => {
            // 按压期间又来 Down：应该不发生（Win32 语义），
            // 但为安全起见保持现有归属。
            RouteResult {
                action: RouteAction::Consume,
                state: RouteState::Pressing(owner),
            }
        }
        PhaseKind::Up => {
            // Up 必回 owner，转发 sink 事件
            RouteResult {
                action: RouteAction::Forward {
                    sink_block: owner.block,
                },
                state: RouteState::Idle, // 序列结束
            }
        }
        PhaseKind::Move => {
            if owner.consumed {
                // Consumed：转发但不建会话
                RouteResult {
                    action: RouteAction::Forward {
                        sink_block: owner.block,
                    },
                    state: RouteState::Pressing(owner),
                }
            } else {
                // DragMove：拖拽会话由 drag.rs 驱动，这里只保持状态
                RouteResult {
                    action: RouteAction::Forward {
                        sink_block: owner.block,
                    },
                    state: RouteState::Pressing(owner),
                }
            }
        }
        PhaseKind::DoubleClick => {
            // DoubleClick 不产生会话，只转发
            RouteResult {
                action: RouteAction::Forward {
                    sink_block: owner.block,
                },
                state: RouteState::Pressing(owner),
            }
        }
        PhaseKind::RightDown => {
            // 右键不参与会话仲裁，只转发
            RouteResult {
                action: RouteAction::Forward {
                    sink_block: owner.block,
                },
                state: RouteState::Pressing(owner),
            }
        }
        PhaseKind::Wheel => {
            // 滚轮转发光标块（不粘滞到 owner）
            match input.hit_test {
                Some(block) => RouteResult {
                    action: RouteAction::Forward { sink_block: block },
                    state: RouteState::Pressing(owner), // 保持按压
                },
                None => RouteResult {
                    action: RouteAction::Noop,
                    state: RouteState::Pressing(owner),
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::handle::ResizeEdges;

    // ============================================================
    // 空闲态 Down
    // ============================================================

    #[test]
    fn idle_down_resize_handle_always_wins() {
        let r = route(
            RouteState::Idle,
            RouteInput {
                hit_resize: Some((1, ResizeEdges::LEFT)),
                hit_test: Some(1), // 同时命中块内
                sink_decision: Some(SinkDecision::Consumed),
                phase: PhaseKind::Down,
            },
        );
        assert_eq!(
            r.action,
            RouteAction::StartResize {
                block: 1,
                edges: ResizeEdges::LEFT
            }
        );
        // 缩放不记 press owner（缩放状态机在 drag.rs 管理）
        assert_eq!(r.state, RouteState::Idle);
    }

    #[test]
    fn idle_down_resize_handle_wins_even_without_sink() {
        let r = route(
            RouteState::Idle,
            RouteInput {
                hit_resize: Some((3, ResizeEdges::BOTTOM_RIGHT)),
                hit_test: Some(3),
                sink_decision: None, // 无 sink
                phase: PhaseKind::Down,
            },
        );
        assert_eq!(
            r.action,
            RouteAction::StartResize {
                block: 3,
                edges: ResizeEdges::BOTTOM_RIGHT
            }
        );
    }

    #[test]
    fn idle_down_sink_consumed() {
        let r = route(
            RouteState::Idle,
            RouteInput {
                hit_resize: None,
                hit_test: Some(2),
                sink_decision: Some(SinkDecision::Consumed),
                phase: PhaseKind::Down,
            },
        );
        assert_eq!(r.action, RouteAction::Consume);
        assert_eq!(
            r.state,
            RouteState::Pressing(PressOwner {
                block: 2,
                consumed: true
            })
        );
    }

    #[test]
    fn idle_down_sink_dragmove() {
        let r = route(
            RouteState::Idle,
            RouteInput {
                hit_resize: None,
                hit_test: Some(2),
                sink_decision: Some(SinkDecision::DragMove),
                phase: PhaseKind::Down,
            },
        );
        assert_eq!(r.action, RouteAction::StartMove { block: 2 });
        assert_eq!(
            r.state,
            RouteState::Pressing(PressOwner {
                block: 2,
                consumed: false
            })
        );
    }

    #[test]
    fn idle_down_sink_ignored_defaults_to_dragmove() {
        let r = route(
            RouteState::Idle,
            RouteInput {
                hit_resize: None,
                hit_test: Some(2),
                sink_decision: Some(SinkDecision::Ignored),
                phase: PhaseKind::Down,
            },
        );
        assert_eq!(r.action, RouteAction::StartMove { block: 2 });
    }

    #[test]
    fn idle_down_no_sink_defaults_to_startmove() {
        let r = route(
            RouteState::Idle,
            RouteInput {
                hit_resize: None,
                hit_test: Some(5),
                sink_decision: None,
                phase: PhaseKind::Down,
            },
        );
        assert_eq!(r.action, RouteAction::StartMove { block: 5 });
        assert_eq!(
            r.state,
            RouteState::Pressing(PressOwner {
                block: 5,
                consumed: false
            })
        );
    }

    #[test]
    fn idle_down_outside_block_is_noop() {
        let r = route(
            RouteState::Idle,
            RouteInput {
                hit_resize: None,
                hit_test: None,
                sink_decision: None,
                phase: PhaseKind::Down,
            },
        );
        assert_eq!(r.action, RouteAction::Noop);
        assert_eq!(r.state, RouteState::Idle);
    }

    // ============================================================
    // 空闲态非 Down 事件
    // ============================================================

    #[test]
    fn idle_move_forwards_to_hit_block() {
        let r = route(
            RouteState::Idle,
            RouteInput {
                hit_resize: None,
                hit_test: Some(3),
                sink_decision: None,
                phase: PhaseKind::Move,
            },
        );
        assert_eq!(r.action, RouteAction::Forward { sink_block: 3 });
        assert_eq!(r.state, RouteState::Idle);
    }

    #[test]
    fn idle_move_outside_block_is_noop() {
        let r = route(
            RouteState::Idle,
            RouteInput {
                hit_resize: None,
                hit_test: None,
                sink_decision: None,
                phase: PhaseKind::Move,
            },
        );
        assert_eq!(r.action, RouteAction::Noop);
    }

    #[test]
    fn idle_doubleclick_forwards_no_session() {
        let r = route(
            RouteState::Idle,
            RouteInput {
                hit_resize: None,
                hit_test: Some(1),
                sink_decision: None,
                phase: PhaseKind::DoubleClick,
            },
        );
        assert_eq!(r.action, RouteAction::Forward { sink_block: 1 });
        assert_eq!(r.state, RouteState::Idle); // 不产生按压
    }

    #[test]
    fn idle_rightdown_forwards_no_session() {
        let r = route(
            RouteState::Idle,
            RouteInput {
                hit_resize: None,
                hit_test: Some(2),
                sink_decision: None,
                phase: PhaseKind::RightDown,
            },
        );
        assert_eq!(r.action, RouteAction::Forward { sink_block: 2 });
        assert_eq!(r.state, RouteState::Idle);
    }

    #[test]
    fn idle_wheel_forwards() {
        let r = route(
            RouteState::Idle,
            RouteInput {
                hit_resize: None,
                hit_test: Some(4),
                sink_decision: None,
                phase: PhaseKind::Wheel,
            },
        );
        assert_eq!(r.action, RouteAction::Forward { sink_block: 4 });
    }

    // ============================================================
    // 按压态粘滞
    // ============================================================

    #[test]
    fn pressing_move_sticky_to_owner() {
        let owner = PressOwner {
            block: 7,
            consumed: false,
        };
        let r = route(
            RouteState::Pressing(owner),
            RouteInput {
                hit_resize: None,
                hit_test: Some(9), // 光标在另一块上
                sink_decision: None,
                phase: PhaseKind::Move,
            },
        );
        // Move 粘滞到 owner 块 7
        assert_eq!(r.action, RouteAction::Forward { sink_block: 7 });
        assert_eq!(r.state, RouteState::Pressing(owner));
    }

    #[test]
    fn pressing_up_must_return_to_owner() {
        let owner = PressOwner {
            block: 5,
            consumed: true,
        };
        let r = route(
            RouteState::Pressing(owner),
            RouteInput {
                hit_resize: None,
                hit_test: Some(3), // 光标已离开
                sink_decision: None,
                phase: PhaseKind::Up,
            },
        );
        // Up 必须回 owner
        assert_eq!(r.action, RouteAction::Forward { sink_block: 5 });
        // 序列结束
        assert_eq!(r.state, RouteState::Idle);
    }

    #[test]
    fn pressing_up_ends_sequence_even_when_consumed() {
        let owner = PressOwner {
            block: 2,
            consumed: true,
        };
        let r = route(
            RouteState::Pressing(owner),
            RouteInput {
                hit_resize: None,
                hit_test: None,
                sink_decision: None,
                phase: PhaseKind::Up,
            },
        );
        assert_eq!(r.state, RouteState::Idle);
    }

    #[test]
    fn pressing_consumed_move_keeps_owner() {
        let owner = PressOwner {
            block: 10,
            consumed: true,
        };
        let r = route(
            RouteState::Pressing(owner),
            RouteInput {
                hit_resize: None,
                hit_test: Some(1),
                sink_decision: None,
                phase: PhaseKind::Move,
            },
        );
        assert_eq!(r.action, RouteAction::Forward { sink_block: 10 });
        assert_eq!(
            r.state,
            RouteState::Pressing(PressOwner {
                block: 10,
                consumed: true
            })
        );
    }

    #[test]
    fn pressing_doubleclick_sticky_to_owner() {
        let owner = PressOwner {
            block: 3,
            consumed: false,
        };
        let r = route(
            RouteState::Pressing(owner),
            RouteInput {
                hit_resize: None,
                hit_test: Some(8),
                sink_decision: None,
                phase: PhaseKind::DoubleClick,
            },
        );
        assert_eq!(r.action, RouteAction::Forward { sink_block: 3 });
        assert_eq!(r.state, RouteState::Pressing(owner));
    }

    #[test]
    fn pressing_rightdown_sticky_to_owner() {
        let owner = PressOwner {
            block: 3,
            consumed: false,
        };
        let r = route(
            RouteState::Pressing(owner),
            RouteInput {
                hit_resize: None,
                hit_test: Some(8),
                sink_decision: None,
                phase: PhaseKind::RightDown,
            },
        );
        assert_eq!(r.action, RouteAction::Forward { sink_block: 3 });
    }

    #[test]
    fn pressing_wheel_goes_to_cursor_not_owner() {
        // 滚轮不粘滞：转发当前光标所在块
        let owner = PressOwner {
            block: 3,
            consumed: false,
        };
        let r = route(
            RouteState::Pressing(owner),
            RouteInput {
                hit_resize: None,
                hit_test: Some(9), // 光标在块 9
                sink_decision: None,
                phase: PhaseKind::Wheel,
            },
        );
        assert_eq!(r.action, RouteAction::Forward { sink_block: 9 });
        // 保持按压状态
        assert_eq!(r.state, RouteState::Pressing(owner));
    }

    #[test]
    fn pressing_wheel_outside_is_noop_but_keeps_state() {
        let owner = PressOwner {
            block: 3,
            consumed: false,
        };
        let r = route(
            RouteState::Pressing(owner),
            RouteInput {
                hit_resize: None,
                hit_test: None,
                sink_decision: None,
                phase: PhaseKind::Wheel,
            },
        );
        assert_eq!(r.action, RouteAction::Noop);
        assert_eq!(r.state, RouteState::Pressing(owner));
    }

    // ============================================================
    // 全矩阵覆盖率：把手/块内/块外 x 左/右/滚轮 x 有无 sink
    // ============================================================

    #[test]
    fn matrix_idle_left_down_with_sink_consumed() {
        let r = route(
            RouteState::Idle,
            RouteInput {
                hit_resize: None,
                hit_test: Some(1),
                sink_decision: Some(SinkDecision::Consumed),
                phase: PhaseKind::Down,
            },
        );
        assert_eq!(r.action, RouteAction::Consume);
    }

    #[test]
    fn matrix_idle_left_down_no_sink() {
        let r = route(
            RouteState::Idle,
            RouteInput {
                hit_resize: None,
                hit_test: Some(1),
                sink_decision: None,
                phase: PhaseKind::Down,
            },
        );
        assert_eq!(r.action, RouteAction::StartMove { block: 1 });
    }

    #[test]
    fn matrix_idle_left_down_outside() {
        let r = route(
            RouteState::Idle,
            RouteInput {
                hit_resize: None,
                hit_test: None,
                sink_decision: None,
                phase: PhaseKind::Down,
            },
        );
        assert_eq!(r.action, RouteAction::Noop);
    }

    #[test]
    fn matrix_idle_right_down_inside() {
        let r = route(
            RouteState::Idle,
            RouteInput {
                hit_resize: None,
                hit_test: Some(2),
                sink_decision: None,
                phase: PhaseKind::RightDown,
            },
        );
        assert_eq!(r.action, RouteAction::Forward { sink_block: 2 });
        assert_eq!(r.state, RouteState::Idle); // 右键不设 owner
    }

    #[test]
    fn matrix_idle_right_down_outside() {
        let r = route(
            RouteState::Idle,
            RouteInput {
                hit_resize: None,
                hit_test: None,
                sink_decision: None,
                phase: PhaseKind::RightDown,
            },
        );
        assert_eq!(r.action, RouteAction::Noop);
    }

    #[test]
    fn matrix_idle_wheel_inside() {
        let r = route(
            RouteState::Idle,
            RouteInput {
                hit_resize: None,
                hit_test: Some(3),
                sink_decision: None,
                phase: PhaseKind::Wheel,
            },
        );
        assert_eq!(r.action, RouteAction::Forward { sink_block: 3 });
    }

    #[test]
    fn matrix_pressing_move_consumed_owner_sticky() {
        let owner = PressOwner {
            block: 1,
            consumed: true,
        };
        let r = route(
            RouteState::Pressing(owner),
            RouteInput {
                hit_resize: None,
                hit_test: Some(2), // 光标在块 2
                sink_decision: None,
                phase: PhaseKind::Move,
            },
        );
        assert_eq!(r.action, RouteAction::Forward { sink_block: 1 });
    }

    #[test]
    fn matrix_pressing_move_dragmove_owner_sticky() {
        let owner = PressOwner {
            block: 1,
            consumed: false,
        };
        let r = route(
            RouteState::Pressing(owner),
            RouteInput {
                hit_resize: None,
                hit_test: Some(2),
                sink_decision: None,
                phase: PhaseKind::Move,
            },
        );
        assert_eq!(r.action, RouteAction::Forward { sink_block: 1 });
    }

    #[test]
    fn matrix_pressing_up_ends_state() {
        let owner = PressOwner {
            block: 1,
            consumed: false,
        };
        let r = route(
            RouteState::Pressing(owner),
            RouteInput {
                hit_resize: None,
                hit_test: None,
                sink_decision: None,
                phase: PhaseKind::Up,
            },
        );
        assert_eq!(r.state, RouteState::Idle);
        assert_eq!(r.action, RouteAction::Forward { sink_block: 1 });
    }

    #[test]
    fn press_owner_debug_display() {
        let o = PressOwner {
            block: 42,
            consumed: true,
        };
        assert_eq!(
            format!("{:?}", o),
            "PressOwner { block: 42, consumed: true }"
        );
    }

    #[test]
    fn route_action_equality() {
        assert_eq!(RouteAction::Noop, RouteAction::Noop);
        assert_ne!(RouteAction::Noop, RouteAction::Consume);
        assert_eq!(
            RouteAction::StartMove { block: 1 },
            RouteAction::StartMove { block: 1 }
        );
        assert_ne!(
            RouteAction::StartMove { block: 1 },
            RouteAction::StartMove { block: 2 }
        );
    }
}
