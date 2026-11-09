//! 位移只改繪製座標，清單與抽屜不逐幀重排 Taffy。
use gpui::*;
use std::time::{Duration, Instant};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};
pub const NORMAL: Duration = Duration::from_millis(220);
pub const SCENE: Duration = Duration::from_millis(260);
#[derive(Default)]
pub struct PressState {
    at: Option<Instant>,
    point: Point<Pixels>,
    bounds: Bounds<Pixels>,
}
pub type PressCache = Rc<RefCell<BTreeMap<u64, PressState>>>;
/// 保留原按鈕的鍵盤與無障礙語意；波紋只回應滑鼠按下並只在繪製階段更新。
pub fn press_feedback(
    child: impl IntoElement,
    id: u64,
    cache: PressCache,
    color: Hsla,
) -> impl IntoElement {
    let down = cache.clone();
    let paint = cache.clone();
    div()
        .relative()
        .rounded_md()
        .overflow_hidden()
        .child(child)
        .capture_any_mouse_down(move |event, window, cx| {
            if event.button == MouseButton::Left && !cx.reduce_motion() {
                let mut entries = down.borrow_mut();
                let state = entries.entry(id).or_default();
                state.at = Some(Instant::now());
                state.point = event.position - state.bounds.origin;
                // 滑鼠事件不在繪製階段；先要求重繪，再由 canvas 的 paint 排後續影格。
                window.refresh();
            }
        })
        .child(
            canvas(
                move |bounds, _, _| {
                    cache.borrow_mut().entry(id).or_default().bounds = bounds;
                },
                move |bounds, _, window, cx| {
                    let entries = paint.borrow();
                    let Some(state) = entries.get(&id) else {
                        return;
                    };
                    let Some(at) = state.at else {
                        return;
                    };
                    let t = at.elapsed().as_secs_f32() / NORMAL.as_secs_f32();
                    if t >= 1. || cx.reduce_motion() {
                        return;
                    }
                    let radius = f32::from(bounds.size.width).hypot(f32::from(bounds.size.height))
                        * (1. - (1. - t).powi(3));
                    let circle = Bounds {
                        origin: bounds.origin + state.point - point(px(radius), px(radius)),
                        size: size(px(radius * 2.), px(radius * 2.)),
                    };
                    let mut ink = color;
                    ink.a *= 0.18 * (1. - t);
                    window.paint_quad(fill(circle, ink).corner_radii(px(radius)));
                    window.request_animation_frame();
                },
            )
            .absolute()
            .inset_0(),
        )
}
#[derive(Clone, Debug)]
pub struct MotionValue {
    from: f32,
    to: f32,
    at: Instant,
    duration: Duration,
}
impl MotionValue {
    pub fn settled(value: f32) -> Self {
        Self {
            from: value,
            to: value,
            at: Instant::now(),
            duration: NORMAL,
        }
    }
    pub fn target(&mut self, to: f32, duration: Duration, instant: bool) {
        if self.to == to && !instant {
            return;
        }
        self.from = if instant { to } else { self.value() };
        self.to = to;
        self.at = Instant::now();
        self.duration = duration;
    }
    pub fn value(&self) -> f32 {
        let x = (self.at.elapsed().as_secs_f32() / self.duration.as_secs_f32().max(0.001))
            .clamp(0., 1.);
        let cubic = |t: f32, a: f32, b: f32| {
            3. * (1. - t) * (1. - t) * t * a + 3. * (1. - t) * t * t * b + t * t * t
        };
        let (mut lo, mut hi) = (0., 1.);
        for _ in 0..12 {
            let t = (lo + hi) * 0.5;
            if cubic(t, 0.23, 0.32) < x {
                lo = t
            } else {
                hi = t
            }
        }
        self.from + (self.to - self.from) * cubic((lo + hi) * 0.5, 1., 1.)
    }
    pub fn active(&self) -> bool {
        self.from != self.to && self.at.elapsed() < self.duration
    }
}
pub struct Translated {
    child: AnyElement,
    offset: Point<Pixels>,
    rem: Option<Pixels>,
}
pub fn translated(child: impl IntoElement, x: f32, y: f32) -> Translated {
    Translated {
        child: child.into_any_element(),
        offset: point(px(x), px(y)),
        rem: None,
    }
}
/// 高字級下仍保留操作與內容空間，工具列及導覽縮放限制在 0.85–1.2 倍。
pub fn chrome(child: impl IntoElement, scale: f32) -> Translated {
    Translated {
        child: child.into_any_element(),
        offset: Point::default(),
        rem: Some(px(16. * scale.clamp(0.85, 1.2))),
    }
}
impl IntoElement for Translated {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for Translated {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (
            window.with_rem_size(self.rem, |window| self.child.request_layout(window, cx)),
            (),
        )
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_rem_size(self.rem, |window| {
            window.with_element_offset(self.offset, |window| {
                self.child.prepaint(window, cx);
            })
        });
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_rem_size(self.rem, |window| self.child.paint(window, cx));
    }
}

pub type ReflowCache = Rc<RefCell<BTreeMap<u64, ReflowPosition>>>;
pub struct ReflowPosition {
    origin: Point<Pixels>,
    x: MotionValue,
    y: MotionValue,
    revision: u64,
    frame: u64,
    width: f32,
}
pub struct Reflow {
    child: AnyElement,
    id: u64,
    cache: ReflowCache,
    revision: u64,
    frame: u64,
    width: f32,
    instant: bool,
}
pub fn reflow(
    child: impl IntoElement,
    id: u64,
    cache: ReflowCache,
    revision: u64,
    frame: u64,
    width: f32,
    instant: bool,
) -> Reflow {
    Reflow {
        child: child.into_any_element(),
        id,
        cache,
        revision,
        frame,
        width,
        instant,
    }
}
impl IntoElement for Reflow {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for Reflow {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let mut cache = self.cache.borrow_mut();
        let entry = cache.entry(self.id).or_insert_with(|| ReflowPosition {
            origin: bounds.origin,
            x: MotionValue::settled(0.),
            y: MotionValue::settled(0.),
            revision: self.revision,
            frame: self.frame,
            width: self.width,
        });
        if self.instant || (entry.width - self.width).abs() > 1. {
            entry.x = MotionValue::settled(0.);
            entry.y = MotionValue::settled(0.);
        } else if entry.revision != self.revision && entry.frame + 1 >= self.frame {
            entry.x =
                MotionValue::settled(f32::from(entry.origin.x - bounds.origin.x) + entry.x.value());
            entry.y =
                MotionValue::settled(f32::from(entry.origin.y - bounds.origin.y) + entry.y.value());
            entry.x.target(0., NORMAL, false);
            entry.y.target(0., NORMAL, false);
        }
        entry.origin = bounds.origin;
        entry.revision = self.revision;
        entry.frame = self.frame;
        entry.width = self.width;
        let offset = point(px(entry.x.value()), px(entry.y.value()));
        if entry.x.active() || entry.y.active() {
            window.request_animation_frame();
        }
        drop(cache);
        window.with_element_offset(offset, |window| {
            self.child.prepaint(window, cx);
        });
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;
    #[test]
    fn retarget_keeps_current_position() {
        let mut v = MotionValue::settled(10.);
        v.target(20., NORMAL, false);
        let at = v.value();
        v.target(0., NORMAL, false);
        assert!((v.value() - at).abs() < 0.1);
        v.target(7., NORMAL, true);
        assert_eq!(v.value(), 7.);
        assert!(!v.active());
    }
}
