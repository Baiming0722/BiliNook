use gpui::{prelude::*, *};
use std::time::Instant;

/// 單行滾輪對應的平滑移動像素（Windows 預設一格滾輪為 3 行 ≈ 108px）。
const WHEEL_LINE_PX: f32 = 36.0;
/// 指數平滑收斂速率（1/s），以真實幀間隔 `dt` 計算，確保 60Hz / 120Hz / 144Hz 曲線一致。
const SMOOTH_DECAY_RATE: f32 = 18.0;
/// 尾段最小線性步進（px / 60Hz 等效幀），避免指數收斂尾端拖泥帶水。
const MIN_LINEAR_STEP_PX: f32 = 0.9;
/// 單次累積最大上限，避免高速連滾造成過度滑行。
const MAX_PENDING_PX: f32 = 1800.0;
const SETTLE_EPSILON_PX: f32 = 0.25;

#[derive(Default)]
struct WheelAnimator {
    pending_px: f32,
    last_step: Option<Instant>,
}

pub(crate) fn compute_smooth_step(pending_px: f32, dt_secs: f32) -> (f32, f32) {
    if pending_px.abs() < SETTLE_EPSILON_PX {
        return (pending_px, 0.0);
    }
    let dt = dt_secs.clamp(0.001, 0.05);
    let exp_factor = 1.0 - (-SMOOTH_DECAY_RATE * dt).exp();
    let min_step = MIN_LINEAR_STEP_PX * (dt * 60.0);
    let raw_mag = (pending_px.abs() * exp_factor).max(min_step);
    if raw_mag >= pending_px.abs() {
        (pending_px, 0.0)
    } else {
        let step = pending_px.signum() * raw_mag;
        let next = pending_px - step;
        if next.abs() < SETTLE_EPSILON_PX {
            (pending_px, 0.0)
        } else {
            (step, next)
        }
    }
}

impl WheelAnimator {
    fn push_wheel_lines(&mut self, delta_lines_y: f32) {
        let impulse = -delta_lines_y * WHEEL_LINE_PX;
        if impulse.abs() < 0.01 {
            return;
        }
        // 反向滾動時立即切換方向，避免殘留反向慣性造成遲滯感。
        if self.pending_px * impulse < 0.0 {
            self.pending_px = impulse;
        } else {
            self.pending_px = (self.pending_px + impulse).clamp(-MAX_PENDING_PX, MAX_PENDING_PX);
        }
        if self.last_step.is_none() {
            self.last_step = Some(Instant::now());
        }
    }

    fn take_step(&mut self, now: Instant, reduce_motion: bool) -> Option<f32> {
        if self.pending_px.abs() < SETTLE_EPSILON_PX {
            self.pending_px = 0.0;
            self.last_step = None;
            return None;
        }
        if reduce_motion {
            let full = self.pending_px;
            self.pending_px = 0.0;
            self.last_step = None;
            return Some(full);
        }
        let dt = self
            .last_step
            .map(|prev| now.saturating_duration_since(prev).as_secs_f32())
            .unwrap_or(1.0 / 60.0);
        self.last_step = Some(now);
        let (step, remaining) = compute_smooth_step(self.pending_px, dt);
        self.pending_px = remaining;
        if remaining == 0.0 {
            self.last_step = None;
        }
        Some(step)
    }

    fn stop(&mut self) {
        self.pending_px = 0.0;
        self.last_step = None;
    }

    fn is_active(&self) -> bool {
        self.pending_px.abs() >= SETTLE_EPSILON_PX
    }
}

/// 為 GPUI `ListState` 虛擬列表加上 VSync 對齊的線性／指數平滑滑鼠滾輪控制。
/// 精確觸控板 (`ScrollDelta::Pixels`) 保持原生逐像素跟手，步進滾輪 (`ScrollDelta::Lines`) 轉為平滑滑行。
pub fn smooth_list(
    id: impl Into<ElementId>,
    view_id: EntityId,
    list_state: ListState,
    render_item: impl Fn(usize, &mut Window, &mut App) -> AnyElement + 'static,
) -> Div {
    let key: ElementId = id.into();
    let step_state = list_state.clone();
    let wheel_state = list_state.clone();

    let controller = canvas(
        move |bounds, window, cx| {
            let animator = window.use_keyed_state(key.clone(), cx, |_, _| WheelAnimator::default());
            let reduce_motion = cx.reduce_motion();
            let keep_animating = animator.update(cx, |anim, _| {
                if let Some(step) = anim.take_step(Instant::now(), reduce_motion) {
                    let before = step_state.scroll_px_offset_for_scrollbar().y;
                    step_state.scroll_by(px(step));
                    let after = step_state.scroll_px_offset_for_scrollbar().y;
                    // 觸頂或觸底無法再捲動時立即清空殘餘位移，避免反向滾動需要等待排空。
                    if step.abs() > 0.5 && (after - before).abs() < px(0.05) {
                        anim.stop();
                    }
                }
                anim.is_active()
            });
            if keep_animating {
                window.request_animation_frame();
            }
            let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
            (animator, hitbox)
        },
        move |_, (animator, hitbox), window, _| {
            let _ = &wheel_state;
            window.on_mouse_event(move |event: &ScrollEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture || !hitbox.should_handle_scroll(window) {
                    return;
                }
                if let ScrollDelta::Lines(lines) = event.delta
                    && lines.y.abs() > 0.001
                {
                    animator.update(cx, |anim, _| anim.push_wheel_lines(lines.y));
                    cx.stop_propagation();
                    cx.notify(view_id);
                }
            });
        },
    )
    .absolute()
    .inset_0();

    div()
        .relative()
        .flex_1()
        .min_h_0()
        .w_full()
        .child(controller)
        .child(list(list_state, render_item).size_full())
}

/// 為一般 `ScrollHandle` 縱向捲動容器加上 VSync 對齊的線性平滑滾輪控制。
pub fn smooth_scroll_container(
    id: impl Into<ElementId>,
    view_id: EntityId,
    handle: &ScrollHandle,
    content: Div,
) -> Div {
    let id_elem: ElementId = id.into();
    let anim_key = ElementId::NamedChild(std::sync::Arc::new(id_elem.clone()), "smooth-wheel".into());
    let step_handle = handle.clone();

    let controller = canvas(
        move |bounds, window, cx| {
            let animator =
                window.use_keyed_state(anim_key.clone(), cx, |_, _| WheelAnimator::default());
            let reduce_motion = cx.reduce_motion();
            let keep_animating = animator.update(cx, |anim, _| {
                if let Some(step) = anim.take_step(Instant::now(), reduce_motion) {
                    let current = step_handle.offset();
                    let max_y = step_handle.max_offset().y.max(px(0.));
                    let next_y = (current.y - px(step)).clamp(-max_y, px(0.));
                    if step.abs() > 0.5 && (next_y - current.y).abs() < px(0.05) {
                        anim.stop();
                    } else {
                        step_handle.set_offset(point(current.x, next_y));
                    }
                }
                anim.is_active()
            });
            if keep_animating {
                window.request_animation_frame();
            }
            let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
            (animator, hitbox)
        },
        move |_, (animator, hitbox), window, _| {
            window.on_mouse_event(move |event: &ScrollEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture || !hitbox.should_handle_scroll(window) {
                    return;
                }
                if let ScrollDelta::Lines(lines) = event.delta
                    && lines.y.abs() > 0.001
                {
                    animator.update(cx, |anim, _| anim.push_wheel_lines(lines.y));
                    cx.stop_propagation();
                    cx.notify(view_id);
                }
            });
        },
    )
    .absolute()
    .inset_0();

    div().relative().flex_1().min_h_0().w_full().child(controller).child(
        content
            .id(id_elem)
            .size_full()
            .overflow_y_scroll()
            .track_scroll(handle),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn smooth_step_converges_monotonically_to_exact_target() {
        let mut remaining = 108.0_f32;
        let mut total_applied = 0.0_f32;
        let mut frames = 0;
        while remaining.abs() > 0.0 {
            let (step, next) = compute_smooth_step(remaining, 1.0 / 60.0);
            assert!(step > 0.0);
            total_applied += step;
            remaining = next;
            frames += 1;
            assert!(frames < 60, "smooth scroll should converge well under 1s");
        }
        assert!((total_applied - 108.0).abs() < 1e-4);
        assert!(frames >= 6, "should interpolate across multiple frames");
    }

    #[test]
    fn wheel_animator_reverses_direction_immediately() {
        let mut anim = WheelAnimator::default();
        anim.push_wheel_lines(-3.0);
        assert!(anim.pending_px > 100.0);
        anim.push_wheel_lines(3.0);
        assert!(anim.pending_px < -100.0);
    }
}
