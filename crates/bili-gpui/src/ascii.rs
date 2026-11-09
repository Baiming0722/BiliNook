//! 小型 ASCII 狀態提示。共用既有桌面時鐘，不把未知進度偽裝成百分比。
use gpui::component::{Sizable as _, spinner::Spinner as NativeSpinner};
use gpui::{prelude::*, *};
use std::time::{Duration, Instant};

const SPINNER: [&str; 4] = ["|", "/", "-", "\\"];
const DOTS: [&str; 3] = [".  ", ".. ", "..."];
const WORKING: [&str; 4] = ["[=..]", "[.=.]", "[..=]", "[.=.]"];
const CONNECTING: [&str; 4] = ["[>..]", "[.>.]", "[..>]", "[.>.]"];
const BROADCAST: [&str; 4] = ["  .  ", " (.) ", "((.))", " (.) "];
const PONG: [&str; 12] = [
    "[=      ]",
    "[ =     ]",
    "[  =    ]",
    "[   =   ]",
    "[    =  ]",
    "[     = ]",
    "[      =]",
    "[     = ]",
    "[    =  ]",
    "[   =   ]",
    "[  =    ]",
    "[ =     ]",
];
const FLOW: [&str; 4] = [
    "~o~~~o~~~o~~",
    "~~o~~~o~~~o~",
    "~~~o~~~o~~~o",
    "o~~~o~~~o~~~",
];
const FILM: [&str; 8] = [
    "|>..|...|..|",
    "|.>.|...|..|",
    "|..>|...|..|",
    "|...|>..|..|",
    "|...|.>.|..|",
    "|...|..>|..|",
    "|...|...|>.|",
    "|...|...|.>|",
];
const TUNING: [&str; 4] = ["[=..][..=]", "[.=.][.=.]", "[..=][=..]", "[.=.][.=.]"];
const RADAR: [&str; 4] = ["(    +    )", "(   (+)   )", "(  ((+))  )", "(   (+)   )"];
const SPARKS: [&str; 4] = [
    " *      .   ",
    " .  *    .  ",
    "  .   *   . ",
    "   .    *  .",
];
const UNREAD: [&str; 4] = ["  *  ", " .*. ", ". * .", " .*. "];
const MASCOT: [&str; 8] = [
    "   \\   /   \n  .-----.  \n  | o o |  \n  '-----'  ",
    "   \\   /   \n  .-----.  \n  | o o |  \n  '-----'  ",
    "   \\   /   \n  .-----.  \n  | - - |  \n  '-----'  ",
    "   \\   /   \n  .-----.  \n  | o o |  \n  '-----'  ",
    "   \\   /   \n  .-----.  \n  | ^ ^ |  \n  '-----'  ",
    "   \\   /   \n  .-----.  \n  | ^ ^ |  \n  '-----'  ",
    "   \\   /   \n  .-----.  \n  | o o |  \n  '-----'  ",
    "   \\   /   \n  .-----.  \n  | o o |  \n  '-----'  ",
];
const CLOCK_STEPS: usize = 24;
const VISIBLE_GRACE: Duration = Duration::from_millis(360);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Spinner,
    Dots,
    Working,
    Connecting,
    Broadcast,
    Pong,
    Flow,
    Film,
    Tuning,
    Radar,
    Sparks,
    Unread,
    Mascot,
}
impl Style {
    fn key(self) -> &'static str {
        match self {
            Self::Spinner => "spinner",
            Self::Dots => "dots",
            Self::Working => "working",
            Self::Connecting => "connecting",
            Self::Broadcast => "broadcast",
            Self::Pong => "pong",
            Self::Flow => "flow",
            Self::Film => "film",
            Self::Tuning => "tuning",
            Self::Radar => "radar",
            Self::Sparks => "sparks",
            Self::Unread => "unread",
            Self::Mascot => "mascot",
        }
    }
    fn width(self) -> f32 {
        match self {
            Self::Spinner => 14.,
            Self::Dots => 28.,
            Self::Working | Self::Connecting | Self::Broadcast => 42.,
            Self::Pong => 76.,
            Self::Flow | Self::Film | Self::Sparks => 100.,
            Self::Tuning => 84.,
            Self::Radar => 92.,
            Self::Unread => 42.,
            Self::Mascot => 92.,
        }
    }
    fn frame(self, tick: usize, reduced: bool) -> &'static str {
        match (self, reduced) {
            (Self::Spinner, true) => "|",
            (Self::Dots, true) => "...",
            (Self::Working, true) => "[...]",
            (Self::Connecting, true) => "[...]",
            (Self::Broadcast, true) => "((.))",
            (Self::Pong, true) => PONG[3],
            (Self::Flow, true) => FLOW[0],
            (Self::Film, true) => FILM[0],
            (Self::Tuning, true) => TUNING[0],
            (Self::Radar, true) => RADAR[1],
            (Self::Sparks, true) => SPARKS[0],
            (Self::Unread, true) => UNREAD[0],
            (Self::Mascot, true) => MASCOT[0],
            (Self::Spinner, false) => SPINNER[tick % SPINNER.len()],
            (Self::Dots, false) => DOTS[(tick / 2) % DOTS.len()],
            (Self::Working, false) => WORKING[(tick / 2) % WORKING.len()],
            (Self::Connecting, false) => CONNECTING[(tick / 2) % CONNECTING.len()],
            (Self::Broadcast, false) => BROADCAST[(tick / 6) % BROADCAST.len()],
            (Self::Pong, false) => PONG[tick % PONG.len()],
            (Self::Flow, false) => FLOW[(tick / 2) % FLOW.len()],
            (Self::Film, false) => FILM[(tick / 3) % FILM.len()],
            (Self::Tuning, false) => TUNING[(tick / 2) % TUNING.len()],
            (Self::Radar, false) => RADAR[(tick / 3) % RADAR.len()],
            (Self::Sparks, false) => SPARKS[(tick / 2) % SPARKS.len()],
            (Self::Unread, false) => UNREAD[(tick / 3) % UNREAD.len()],
            (Self::Mascot, false) => MASCOT[(tick / 3) % MASCOT.len()],
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Broadcast => "有直播進行中",
            Self::Connecting | Self::Pong => "載入中",
            _ => "處理中",
        }
    }
    fn decorative(self) -> bool {
        matches!(
            self,
            Self::Mascot
                | Self::Flow
                | Self::Film
                | Self::Tuning
                | Self::Radar
                | Self::Sparks
                | Self::Unread
        )
    }
    fn height(self) -> f32 {
        if self == Self::Mascot { 64. } else { 16. }
    }
}

pub struct Clock {
    phase: usize,
    enabled: bool,
    reduced: bool,
    last_visible: Option<Instant>,
}
impl Default for Clock {
    fn default() -> Self {
        Self {
            phase: 0,
            enabled: true,
            reduced: false,
            last_visible: None,
        }
    }
}
struct SharedClock(Entity<Clock>);
impl Global for SharedClock {}

pub fn install(cx: &mut App) -> Entity<Clock> {
    let clock = cx.new(|_| Clock::default());
    cx.set_global(SharedClock(clock.clone()));
    clock
}
impl Clock {
    pub fn configure(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.enabled != enabled {
            self.enabled = enabled;
            cx.notify();
        }
    }
    pub fn tick(&mut self, foreground: bool, cx: &mut Context<Self>) {
        let reduced = cx.reduce_motion();
        let changed = reduced != self.reduced;
        self.reduced = reduced;
        if reduced {
            self.phase = 0;
        } else if should_advance(
            self.enabled,
            foreground,
            self.last_visible.map(|at| at.elapsed()),
        ) {
            self.phase = (self.phase + 1) % CLOCK_STEPS;
            #[cfg(debug_assertions)]
            if std::env::var_os("BILI_ASCII_TRACE").is_some() {
                eprintln!("[ASCII] frame={}", self.phase);
            }
            cx.notify();
            return;
        }
        if changed {
            cx.notify();
        }
    }
}
fn should_advance(enabled: bool, foreground: bool, age: Option<Duration>) -> bool {
    enabled && foreground && age.is_some_and(|age| age < VISIBLE_GRACE)
}

pub fn loading(id: impl Into<ElementId>) -> AnyElement {
    activity(id, Style::Spinner)
}
pub fn waiting(id: impl Into<ElementId>) -> AnyElement {
    activity(id, Style::Dots)
}
pub fn pong(id: impl Into<ElementId>) -> AnyElement {
    activity(id, Style::Pong)
}
pub fn working(id: impl Into<ElementId>) -> AnyElement {
    activity(id, Style::Working)
}
pub fn connecting(id: impl Into<ElementId>) -> AnyElement {
    activity(id, Style::Connecting)
}
pub fn broadcast(id: impl Into<ElementId>) -> AnyElement {
    activity(id, Style::Broadcast)
}
pub fn decoration(id: impl Into<ElementId>, style: Style) -> AnyElement {
    debug_assert!(style.decorative());
    activity(id, style)
}
fn activity(id: impl Into<ElementId>, style: Style) -> AnyElement {
    Loading {
        id: id.into(),
        style,
    }
    .into_any_element()
}

#[derive(IntoElement)]
struct Loading {
    id: ElementId,
    style: Style,
}
impl RenderOnce for Loading {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Some(shared) = cx.try_global::<SharedClock>() else {
            if self.style.decorative() {
                return div()
                    .w(px(self.style.width()))
                    .h(px(self.style.height()))
                    .into_any_element();
            }
            return NativeSpinner::new().xsmall().into_any_element();
        };
        let clock = shared.0.clone();
        let style = self.style;
        window
            .with_element_namespace(self.id, |window| {
                window.use_keyed_state(style.key(), cx, |_, cx| {
                    let subscription = cx.observe(&clock, |this: &mut InlineLoading, clock, cx| {
                        let clock = clock.read(cx);
                        let reduced = cx.reduce_motion();
                        if this.last_enabled != clock.enabled
                            || this.last_reduced != reduced
                            || (clock.enabled
                                && this.style.frame(this.held_phase, reduced)
                                    != this.style.frame(clock.phase, reduced))
                            // 廣播影格較慢，定期確認可見性以免共用時鐘提早休眠。
                            || (clock.enabled && (this.style == Style::Broadcast || this.style.decorative()) && clock.phase % 2 == 0)
                        {
                            cx.notify();
                        }
                    });
                    InlineLoading {
                        clock,
                        style,
                        _subscription: subscription,
                        held_phase: 0,
                        last_enabled: true,
                        last_reduced: false,
                    }
                })
            })
            .into_any_element()
    }
}
struct InlineLoading {
    clock: Entity<Clock>,
    style: Style,
    _subscription: Subscription,
    held_phase: usize,
    last_enabled: bool,
    last_reduced: bool,
}
impl Render for InlineLoading {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.last_enabled = self.clock.read(cx).enabled;
        self.last_reduced = cx.reduce_motion();
        if !self.last_enabled {
            if self.style.decorative() {
                return div()
                    .w(px(self.style.width()))
                    .h(px(self.style.height()))
                    .flex_shrink_0()
                    .into_any_element();
            }
            if self.style == Style::Broadcast {
                return div()
                    .id("ascii-broadcast-fallback")
                    .role(Role::Image)
                    .aria_label(self.style.label())
                    .w(px(self.style.width()))
                    .h(px(16.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .size(px(6.))
                            .rounded_full()
                            .bg(window.text_style().color),
                    )
                    .into_any_element();
            }
            return NativeSpinner::new().xsmall().into_any_element();
        }
        if self.last_reduced {
            self.held_phase = 0;
        } else if window.is_window_active() {
            self.held_phase = self.clock.read(cx).phase;
        }
        draw_ascii(
            self.style.frame(self.held_phase, self.last_reduced).into(),
            self.style.width(),
            self.style.height(),
            (!self.style.decorative()).then(|| self.style.label().into()),
            Some(self.clock.clone()),
        )
    }
}

fn draw_ascii(
    frame: SharedString,
    width: f32,
    height: f32,
    label: Option<SharedString>,
    clock: Option<Entity<Clock>>,
) -> AnyElement {
    div()
        .id("ascii-indicator")
        .when_some(label, |element, label| {
            element.role(Role::Image).aria_label(label)
        })
        .w(px(width))
        .h(px(height))
        .flex_shrink_0()
        .child(
            canvas(
                move |bounds, window, cx| {
                    if let Some(clock) = &clock {
                        let visible = bounds.intersect(&window.content_mask().bounds);
                        if window.is_window_active()
                            && visible.size.width > px(0.)
                            && visible.size.height > px(0.)
                        {
                            clock.update(cx, |clock, _| clock.last_visible = Some(Instant::now()));
                        }
                    }
                },
                move |bounds, _, window, cx| {
                    let reference_width = if frame.contains('\n') {
                        let reference: SharedString =
                            "M".repeat(frame.lines().next().map_or(0, str::len)).into();
                        let run = TextRun {
                            len: reference.len(),
                            font: font("Consolas"),
                            color: window.text_style().color,
                            ..Default::default()
                        };
                        Some(
                            window
                                .text_system()
                                .shape_line(reference, px(12.), &[run], None)
                                .width,
                        )
                    } else {
                        None
                    };
                    let mut paint_line = |line: SharedString, row: usize| {
                        let run = TextRun {
                            len: line.len(),
                            font: font("Consolas"),
                            color: window.text_style().color,
                            ..Default::default()
                        };
                        let shaped = window.text_system().shape_line(line, px(12.), &[run], None);
                        let origin = point(
                            bounds.origin.x
                                + (bounds.size.width - reference_width.unwrap_or(shaped.width))
                                    * 0.5,
                            bounds.origin.y + px(16. * row as f32),
                        );
                        if let Err(error) =
                            shaped.paint(origin, px(16.), TextAlign::Left, None, window, cx)
                        {
                            eprintln!("[ASCII 狀態提示] 字元繪製失敗：{error}");
                        }
                    };
                    if frame.contains('\n') {
                        for (row, line) in frame.split('\n').enumerate() {
                            paint_line(line.to_owned().into(), row);
                        }
                    } else {
                        paint_line(frame, 0);
                    }
                },
            )
            .w_full()
            .h_full(),
        )
        .into_any_element()
}

fn wipe_frame(progress: f32) -> String {
    let filled = (progress.clamp(0., 1.) * 12.).floor() as usize;
    format!("[{}|{}]", "=".repeat(filled), ".".repeat(12 - filled))
}

/// 切換時短暫顯示擦拭軌跡；不阻擋內容，也不代表資料進度。
pub fn panel_switch(id: impl Into<ElementId>, color: Hsla, background: Hsla) -> AnyElement {
    div()
        .absolute()
        .top_1()
        .right_4()
        .rounded_sm()
        .bg(background.opacity(0.93))
        .text_color(color)
        .with_animation(
            id,
            Animation::new(Duration::from_millis(240)),
            |element, progress| {
                let opacity = if progress > 0.75 {
                    (1. - progress) * 4.
                } else {
                    1.
                };
                element.opacity(opacity).child(draw_ascii(
                    wipe_frame(progress).into(),
                    116.,
                    16.,
                    None,
                    None,
                ))
            },
        )
        .into_any_element()
}

pub fn button_label(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    busy: bool,
) -> AnyElement {
    button_label_with_style(id, text, busy, Style::Spinner)
}
pub fn button_label_with_style(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    busy: bool,
    style: Style,
) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_center()
        .gap_1p5()
        .when(busy, |row| row.child(activity(id, style)))
        .child(text.into())
        .into_any_element()
}

/// 已處理包含成功、已存在及失敗；待處理與停止後留下的項目不計入。
pub fn progress(completed: usize, total: usize) -> AnyElement {
    let frame = progress_text(completed, total);
    let width = (frame.len() as f32 * 7.2 + 6.).ceil();
    let label = if total == 0 {
        "批次進度：總筆數未知".to_owned()
    } else {
        format!("批次進度：已處理 {} / {total} 筆", completed.min(total))
    };
    draw_ascii(frame.into(), width, 16., Some(label.into()), None)
}
fn progress_text(completed: usize, total: usize) -> String {
    if total == 0 {
        return "[....] --/--".into();
    }
    let completed = completed.min(total);
    let filled = (completed as u128 * 4 / total as u128) as usize;
    format!(
        "[{}{}] {completed:>2}/{total:>2}",
        "#".repeat(filled),
        ".".repeat(4 - filled)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;
    #[test]
    fn styles_keep_ascii_width_and_repeat_at_shared_clock_boundary() {
        assert_eq!(SPINNER, ["|", "/", "-", "\\"]);
        for (style, columns) in [
            (Style::Spinner, 1),
            (Style::Dots, 3),
            (Style::Working, 5),
            (Style::Connecting, 5),
            (Style::Broadcast, 5),
            (Style::Pong, 9),
            (Style::Flow, 12),
            (Style::Film, 12),
            (Style::Tuning, 10),
            (Style::Radar, 11),
            (Style::Sparks, 12),
            (Style::Unread, 5),
        ] {
            for tick in 0..CLOCK_STEPS {
                let frame = style.frame(tick, false);
                assert!(frame.is_ascii());
                assert_eq!(frame.len(), columns);
                assert_eq!(frame, style.frame(tick + CLOCK_STEPS, false));
                assert_eq!(style.frame(tick, true), style.frame(0, true));
                assert_eq!(style.frame(tick, true).len(), columns);
            }
        }
        for tick in 0..CLOCK_STEPS {
            let frame = Style::Mascot.frame(tick, false);
            assert!(frame.is_ascii());
            assert_eq!(frame.lines().count(), 4);
            assert!(frame.lines().all(|line| line.len() == 11));
            assert_eq!(frame, Style::Mascot.frame(tick + CLOCK_STEPS, false));
            assert_eq!(
                Style::Mascot.frame(tick, true),
                Style::Mascot.frame(0, true)
            );
        }
        assert_eq!(Style::Dots.frame(0, false), Style::Dots.frame(1, false));
        assert_ne!(Style::Dots.frame(1, false), Style::Dots.frame(2, false));
        assert_eq!(
            Style::Broadcast.frame(0, false),
            Style::Broadcast.frame(5, false)
        );
        assert_ne!(
            Style::Broadcast.frame(5, false),
            Style::Broadcast.frame(6, false)
        );
    }
    #[test]
    fn panel_wipe_uses_a_fixed_width_and_finishes_at_the_end() {
        assert_eq!(wipe_frame(0.), "[|............]");
        assert_eq!(wipe_frame(1.), "[============|]");
        assert_eq!(wipe_frame(0.5).len(), wipe_frame(1.).len());
    }
    #[test]
    fn invisible_or_background_loaders_do_not_drive_the_clock() {
        assert!(should_advance(true, true, Some(Duration::ZERO)));
        assert!(!should_advance(false, true, Some(Duration::ZERO)));
        assert!(!should_advance(true, false, Some(Duration::ZERO)));
        assert!(!should_advance(true, true, None));
        assert!(!should_advance(true, true, Some(VISIBLE_GRACE)));
    }
    #[test]
    fn progress_reports_real_counts_without_completing_pending_items() {
        assert_eq!(progress_text(0, 5), "[....]  0/ 5");
        assert_eq!(progress_text(3, 5), "[##..]  3/ 5");
        assert_eq!(progress_text(5, 5), "[####]  5/ 5");
        assert_eq!(progress_text(2, 0), "[....] --/--");
        assert_eq!(progress_text(9, 5), "[####]  5/ 5");
        assert_eq!(
            progress_text(usize::MAX, usize::MAX).get(..6),
            Some("[####]")
        );
        assert_eq!(progress_text(0, 50).len(), progress_text(50, 50).len());
    }
}
