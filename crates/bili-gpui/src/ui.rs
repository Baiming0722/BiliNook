use crate::assets::AppIcon;
use gpui::component::{
    ActiveTheme as _, Icon, IconName, Selectable as _, Sizable as _, Theme, ThemeMode,
    animation::cubic_bezier, button::*, skeleton::Skeleton,
};
use gpui::{prelude::*, *};
use std::time::Duration;

pub const FONT_FAMILY: &str = "Microsoft JhengHei UI";

pub fn appearance_font(name: &str) -> SharedString {
    match name {
        "system" | "jhenghei" => FONT_FAMILY.into(),
        "serif" => "PMingLiU".into(),
        "mono" => "Cascadia Mono".into(),
        other => other.to_owned().into(),
    }
}
pub fn apply_appearance(settings: &bili_core::live_watch::AppearanceSettings, cx: &mut App) {
    Theme::update(cx, |theme| {
        theme.font_family = appearance_font(&settings.font_family);
        let default = match settings.accent_color.as_str() {
            "blue" => 0x1587f2,
            "purple" => 0x8b5cf6,
            "green" => 0x16a085,
            "orange" => 0xe67e22,
            _ => {
                if theme.is_dark() {
                    0xfb7299
                } else {
                    0xd83b69
                }
            }
        };
        let hex = settings
            .accent_custom
            .as_deref()
            .and_then(|s| u32::from_str_radix(s.trim_start_matches('#').get(..6)?, 16).ok())
            .unwrap_or(default);
        let brand: Hsla = rgb(hex).into();
        let foreground = if brand.l > 0.55 {
            gpui::black()
        } else {
            gpui::white()
        };
        set_brand_tokens(
            &mut theme.colors,
            brand,
            brand.opacity(0.9),
            brand.opacity(0.8),
            foreground,
        );
    });
}

// 版面與響應式常數集中管理，確保各分頁邊距與欄寬計算一致。
pub const SIDEBAR_WIDTH: f32 = 232.0;
pub const SETTINGS_SUBNAV_WIDTH: f32 = 204.0;
pub const PAGE_PADDING_X: f32 = 24.0;
pub const GRID_GAP: f32 = 12.0;
pub const TOAST_WIDTH: f32 = 384.0;

pub const AVATAR_XS: f32 = 22.0;
pub const AVATAR_SM: f32 = 26.0;
pub const AVATAR_MD: f32 = 38.0;
pub const AVATAR_LG: f32 = 42.0;

pub const BP_SETTINGS_SIDE_NAV: f32 = 1040.0;
pub const BP_DUAL_STREAM: f32 = 1180.0;
pub const BP_THREE_COLUMNS: f32 = 1480.0;

// 業務與顯示上限常數
pub const ERROR_MAX_CHARS: usize = 180;
pub const FEED_TEXT_PREVIEW_CHARS: usize = 700;
pub const FORWARD_TEXT_PREVIEW_CHARS: usize = 500;
pub const FEED_MAX_ITEMS: usize = 2000;
pub const VIDEO_MAX_ITEMS: usize = 3000;
pub const VIDEO_MULTI_UP_LIMIT: usize = 30;
pub const COMMENTS_MAX_ITEMS: usize = 200;
pub const MAX_GROUPS: usize = 50;
pub const IMAGE_CACHE_CAPACITY: usize = 256;
pub const IMAGE_MAX_CONCURRENT: usize = 64;
pub const IMAGE_RETRY_SECS: u64 = 30;
pub const MIN_LIVE_POLL_SEC: u64 = 30;
pub const MIN_DYNAMIC_POLL_SEC: u64 = 60;
pub const MAX_POLL_SEC: u64 = 86_400;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Info,
    Success,
    Warning,
    Danger,
}

#[derive(Clone, Copy)]
pub struct ToneColors {
    pub bg: Hsla,
    pub border: Hsla,
    pub fg: Hsla,
    pub accent: Hsla,
}

pub fn tone_colors(tone: Tone, is_dark: bool) -> ToneColors {
    match (tone, is_dark) {
        (Tone::Danger, true) => ToneColors {
            bg: rgb(0x2b151d).into(),
            border: rgb(0x69293c).into(),
            fg: rgb(0xffd4de).into(),
            accent: rgb(0xff6b8b).into(),
        },
        (Tone::Danger, false) => ToneColors {
            bg: rgb(0xfff1f4).into(),
            border: rgb(0xf4b4c3).into(),
            fg: rgb(0x9c1b37).into(),
            accent: rgb(0xd92d53).into(),
        },
        (Tone::Warning, true) => ToneColors {
            bg: rgb(0x2a2111).into(),
            border: rgb(0x664f1f).into(),
            fg: rgb(0xfde5b0).into(),
            accent: rgb(0xf5b041).into(),
        },
        (Tone::Warning, false) => ToneColors {
            bg: rgb(0xfffbeb).into(),
            border: rgb(0xf3d082).into(),
            fg: rgb(0x7c470b).into(),
            accent: rgb(0xd48806).into(),
        },
        (Tone::Success, true) => ToneColors {
            bg: rgb(0x12271d).into(),
            border: rgb(0x24583e).into(),
            fg: rgb(0xc2f5da).into(),
            accent: rgb(0x38c97e).into(),
        },
        (Tone::Success, false) => ToneColors {
            bg: rgb(0xf0fdf4).into(),
            border: rgb(0x9ce0b5).into(),
            fg: rgb(0x155e32).into(),
            accent: rgb(0x16a34a).into(),
        },
        (Tone::Info, true) => ToneColors {
            bg: rgb(0x142234).into(),
            border: rgb(0x294b75).into(),
            fg: rgb(0xc9e2ff).into(),
            accent: rgb(0x58a6ff).into(),
        },
        (Tone::Info, false) => ToneColors {
            bg: rgb(0xf0f7ff).into(),
            border: rgb(0xa8ccf5).into(),
            fg: rgb(0x1d40af).into(),
            accent: rgb(0x2563eb).into(),
        },
    }
}

pub fn tone_icon(tone: Tone) -> IconName {
    match tone {
        Tone::Danger => IconName::CircleAlert,
        Tone::Warning => IconName::TriangleAlert,
        Tone::Success => IconName::CircleCheck,
        Tone::Info => IconName::Info,
    }
}

pub fn brand_accent(cx: &App) -> Hsla {
    cx.theme().primary
}

pub fn brand_soft_bg(cx: &App) -> Hsla {
    if cx.theme().is_dark() {
        rgba(0xfb729926).into()
    } else {
        rgba(0xd83b6918).into()
    }
}

pub fn unread_card_border(cx: &App) -> Hsla {
    if cx.theme().is_dark() {
        rgb(0x88425b).into()
    } else {
        rgb(0xea8ca8).into()
    }
}

pub fn hover_card_border(cx: &App) -> Hsla {
    if cx.theme().is_dark() {
        rgb(0x434d66).into()
    } else {
        rgb(0xbec7da).into()
    }
}

/// 分頁／子分區切換時的 180ms 淡入與微幅上滑過渡動畫。
pub fn enter_transition(element: Div, id: impl Into<ElementId>) -> AnyElement {
    element
        .relative()
        .with_animation(
            id,
            Animation::new(Duration::from_millis(185))
                .with_easing(cubic_bezier(0.16, 1.0, 0.3, 1.0)),
            |el, delta| el.opacity(0.22 + 0.78 * delta).top(px((1.0 - delta) * 7.0)),
        )
        .into_any_element()
}

/// 列表／網格卡片初次出現時的階梯微幅浮現動畫。
pub fn card_enter(element: Div, id: impl Into<ElementId>, slot: usize) -> AnyElement {
    let millis = 150 + (slot.min(5) as u64) * 22;
    element
        .relative()
        .with_animation(
            id,
            Animation::new(Duration::from_millis(millis))
                .with_easing(cubic_bezier(0.2, 0.9, 0.3, 1.0)),
            |el, delta| el.opacity(0.28 + 0.72 * delta).top(px((1.0 - delta) * 5.0)),
        )
        .into_any_element()
}

/// 持續呼吸閃動的直播狀態圓點。
pub fn live_pulse_dot(
    id: impl Into<ElementId>,
    diameter: f32,
    color: Hsla,
    cx: &App,
) -> AnyElement {
    if cx.reduce_motion() {
        return div()
            .size(px(diameter))
            .flex_shrink_0()
            .rounded_full()
            .bg(color)
            .into_any_element();
    }
    div()
        .size(px(diameter))
        .flex_shrink_0()
        .rounded_full()
        .bg(color)
        .with_animation(
            id,
            Animation::new(Duration::from_millis(1600))
                .repeat()
                .with_easing(pulsating_between(0.35, 1.0)),
            |el, delta| el.opacity(delta),
        )
        .into_any_element()
}

/// 帶呼吸圓點的粉色直播膠囊角標。
pub fn live_pill_badge(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    cx: &App,
) -> AnyElement {
    let fg = cx.theme().primary_foreground;
    div()
        .px_2()
        .py_0p5()
        .rounded_full()
        .bg(brand_accent(cx))
        .flex()
        .items_center()
        .gap_1p5()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(fg)
        .child(live_pulse_dot(id, 6.0, fg, cx))
        .child(label.into())
        .into_any_element()
}

/// 設定頁頂部或分區儀表板使用的精實指標卡。
pub fn kpi_stat_card(
    icon: AppIcon,
    label: &str,
    value: impl Into<SharedString>,
    accent: bool,
    cx: &App,
) -> Div {
    let brand = brand_accent(cx);
    let hover_border = hover_card_border(cx);
    div()
        .flex_1()
        .min_w(px(136.))
        .px_3p5()
        .py_2p5()
        .rounded_lg()
        .bg(cx.theme().muted)
        .border_1()
        .border_color(if accent {
            unread_card_border(cx)
        } else {
            cx.theme().border
        })
        .hover(move |s| s.border_color(hover_border))
        .flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .size(px(34.))
                .flex_shrink_0()
                .rounded_md()
                .bg(if accent {
                    brand_soft_bg(cx)
                } else {
                    cx.theme().secondary
                })
                .flex()
                .items_center()
                .justify_center()
                .child(Icon::new(icon).with_size(px(17.)).text_color(if accent {
                    brand
                } else {
                    cx.theme().muted_foreground
                })),
        )
        .child(
            div()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_base()
                        .font_weight(FontWeight::BOLD)
                        .text_color(if accent { brand } else { cx.theme().foreground })
                        .child(value.into()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .truncate()
                        .child(label.to_owned()),
                ),
        )
}

pub fn placeholder_colors(cx: &App) -> (Hsla, Hsla, Hsla) {
    if cx.theme().is_dark() {
        (
            rgb(0x1e232e).into(),
            rgb(0x8d97aa).into(),
            rgb(0x2b3140).into(),
        )
    } else {
        (
            rgb(0xeef1f6).into(),
            rgb(0x5d6778).into(),
            rgb(0xdde2ec).into(),
        )
    }
}

/// 套用主題並同步覆寫 `button_primary*`。
/// `gpui-component` 的 `Theme::change` 會重置全部色票，且 `Theme::update` 不會由 `primary`
/// 重新推導 `button_primary*`；若不同步設定，淺色模式下 Primary 按鈕會維持預設深黑。
pub fn apply_brand_theme(theme_setting: &str, cx: &mut App) {
    let mode = match theme_setting {
        "light" => ThemeMode::Light,
        "system" => ThemeMode::from(cx.window_appearance()),
        _ => ThemeMode::Dark,
    };
    Theme::change(mode, None, cx);
    Theme::update(cx, |theme| {
        theme.font_family = FONT_FAMILY.into();
        if theme.is_dark() {
            let brand: Hsla = rgb(0xfb7299).into();
            let brand_hover: Hsla = rgb(0xff89ab).into();
            let brand_active: Hsla = rgb(0xe45a83).into();
            let brand_fg: Hsla = rgb(0x16070c).into();

            theme.colors.background = rgb(0x111318).into();
            theme.colors.foreground = rgb(0xedeff4).into();
            theme.colors.sidebar = rgb(0x161920).into();
            theme.colors.sidebar_foreground = rgb(0xe5e8f0).into();
            theme.colors.sidebar_border = rgb(0x232834).into();
            theme.colors.muted = rgb(0x191d26).into();
            theme.colors.muted_foreground = rgb(0x949eb2).into();
            theme.colors.secondary = rgb(0x212633).into();
            theme.colors.secondary_hover = rgb(0x2a3142).into();
            theme.colors.accent = rgb(0x242a38).into();
            theme.colors.accent_foreground = rgb(0xf3f5f9).into();
            theme.colors.border = rgb(0x262c3a).into();
            theme.colors.input = rgb(0x2e3547).into();
            theme.colors.popover = rgb(0x1b202b).into();
            theme.colors.popover_foreground = rgb(0xf0f2f7).into();

            set_brand_tokens(
                &mut theme.colors,
                brand,
                brand_hover,
                brand_active,
                brand_fg,
            );
        } else {
            // 淺色主色調深至 #d83b69，使白字對比度達 WCAG AA (>= 4.5:1)。
            let brand: Hsla = rgb(0xd83b69).into();
            let brand_hover: Hsla = rgb(0xe54c79).into();
            let brand_active: Hsla = rgb(0xbf2d58).into();
            let brand_fg: Hsla = gpui::white();

            theme.colors.background = rgb(0xf6f7fa).into();
            theme.colors.foreground = rgb(0x171a21).into();
            theme.colors.sidebar = rgb(0xeeeff4).into();
            theme.colors.sidebar_foreground = rgb(0x1e222b).into();
            theme.colors.sidebar_border = rgb(0xdfe3ec).into();
            theme.colors.muted = gpui::white();
            theme.colors.muted_foreground = rgb(0x596273).into();
            theme.colors.secondary = rgb(0xeceff5).into();
            theme.colors.secondary_hover = rgb(0xe1e6ef).into();
            theme.colors.accent = rgb(0xe8ecf4).into();
            theme.colors.accent_foreground = rgb(0x151820).into();
            theme.colors.border = rgb(0xdfe4ee).into();
            theme.colors.input = rgb(0xcfd6e4).into();
            theme.colors.popover = gpui::white();
            theme.colors.popover_foreground = rgb(0x171a21).into();

            set_brand_tokens(
                &mut theme.colors,
                brand,
                brand_hover,
                brand_active,
                brand_fg,
            );
        }
    });
}

fn set_brand_tokens(
    colors: &mut gpui::component::ThemeColor,
    brand: Hsla,
    brand_hover: Hsla,
    brand_active: Hsla,
    brand_fg: Hsla,
) {
    colors.primary = brand;
    colors.primary_hover = brand_hover;
    colors.primary_active = brand_active;
    colors.primary_foreground = brand_fg;

    colors.button_primary = brand;
    colors.button_primary_hover = brand_hover;
    colors.button_primary_active = brand_active;
    colors.button_primary_foreground = brand_fg;

    colors.ring = brand;
    colors.link = brand;
    colors.link_hover = brand_hover;
    colors.link_active = brand_active;
    colors.sidebar_primary = brand;
    colors.sidebar_primary_foreground = brand_fg;
}

pub fn grid_columns(viewport_width: f32) -> usize {
    if viewport_width >= BP_THREE_COLUMNS {
        3
    } else {
        2
    }
}

pub fn scaled_grid_columns(width: f32, scale: f32) -> usize {
    if (scale - 1.).abs() < 0.01 {
        return grid_columns(width);
    }
    let available = (width - SIDEBAR_WIDTH - PAGE_PADDING_X * 2.).max(160.);
    ((available + GRID_GAP) / ((420. * scale).clamp(180., 840.) + GRID_GAP))
        .floor()
        .clamp(1., 6.) as usize
}

pub fn grid_card_width(viewport_width: f32, columns: usize) -> f32 {
    let cols = columns.max(1) as f32;
    let content_width = viewport_width - SIDEBAR_WIDTH - PAGE_PADDING_X * 2.0 - 16.0;
    let gaps = GRID_GAP * (cols - 1.0);
    ((content_width - gaps) / cols).max(220.0)
}

pub fn feed_card_inner_width(viewport_width: f32, two_columns: bool) -> f32 {
    let workspace = viewport_width - SIDEBAR_WIDTH;
    let pane = if two_columns {
        (workspace - GRID_GAP) / 2.0
    } else {
        workspace
    };
    // 扣除訊息流水平內距 (24*2) 與卡片內距 (16*2)
    (pane - PAGE_PADDING_X * 2.0 - 36.0).max(180.0)
}

pub fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let collected: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{collected}…")
    } else {
        collected
    }
}

pub fn format_count(count: u64) -> String {
    if count >= 100_000_000 {
        format!("{:.1}億", count as f64 / 100_000_000.0)
    } else if count >= 10_000 {
        format!("{:.1}萬", count as f64 / 10_000.0)
    } else {
        count.to_string()
    }
}

/// 側欄與設定子導覽共用的靠左導覽按鈕（基於 `Button` 以完整保留鍵盤焦點與 Windows UIA 可及性）。
pub fn nav_item(
    id: impl Into<ElementId>,
    icon: AppIcon,
    label: impl Into<SharedString>,
    badge: Option<(String, bool)>,
    selected: bool,
    cx: &App,
) -> Button {
    let label = label.into();
    let brand = brand_accent(cx);
    let fg = if selected {
        if cx.theme().is_dark() {
            cx.theme().foreground
        } else {
            brand
        }
    } else {
        cx.theme().foreground
    };
    let icon_color = if selected {
        brand
    } else {
        cx.theme().muted_foreground
    };

    let mut inner = div()
        .flex_1()
        .min_w_0()
        .flex()
        .items_center()
        .gap_2p5()
        .text_color(fg)
        .when(selected, |el| el.font_weight(FontWeight::SEMIBOLD))
        .child(
            div()
                .w(px(3.))
                .h(px(18.))
                .flex_shrink_0()
                .rounded_full()
                .bg(if selected {
                    brand
                } else {
                    gpui::transparent_black()
                }),
        )
        .child(Icon::new(icon).with_size(px(17.)).text_color(icon_color))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_left()
                .child(label.clone()),
        );

    if let Some((text, highlight)) = badge {
        let (pill_bg, pill_fg) = if highlight {
            (brand, cx.theme().primary_foreground)
        } else {
            (cx.theme().secondary, cx.theme().muted_foreground)
        };
        inner = inner.child(
            div()
                .flex_shrink_0()
                .px_1p5()
                .py_0p5()
                .rounded_full()
                .bg(pill_bg)
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(pill_fg)
                .child(text),
        );
    }

    let mut button = Button::new(id)
        .ghost()
        .selected(selected)
        .toggled(selected)
        .accessibility_label(label)
        .w_full()
        .h(px(36.))
        .px_2();
    if selected {
        button = button.bg(brand_soft_bg(cx));
    }
    button.child(inner)
}

pub fn notice_banner(tone: Tone, message: &str, actions: Vec<AnyElement>, cx: &App) -> AnyElement {
    let colors = tone_colors(tone, cx.theme().is_dark());
    let mut bar = div()
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap_2p5()
        .px_3p5()
        .py_2()
        .rounded_md()
        .border_1()
        .border_color(colors.border)
        .bg(colors.bg)
        .text_sm()
        .text_color(colors.fg)
        .child(
            Icon::new(tone_icon(tone))
                .with_size(px(16.))
                .text_color(colors.accent),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(truncate_chars(message, ERROR_MAX_CHARS)),
        );
    if !actions.is_empty() {
        bar = bar.child(div().flex().items_center().gap_1p5().children(actions));
    }
    bar.relative()
        .with_animation(
            SharedString::from(format!("banner-{tone:?}-{}", message.len())),
            Animation::new(Duration::from_millis(160))
                .with_easing(cubic_bezier(0.16, 1.0, 0.3, 1.0)),
            |el, delta| el.opacity(0.3 + 0.7 * delta).top(px((1.0 - delta) * -4.0)),
        )
        .into_any_element()
}

pub fn empty_state(
    icon: AppIcon,
    title: &str,
    body: &str,
    action: Option<AnyElement>,
    cx: &App,
) -> AnyElement {
    let mut box_el = div()
        .size_full()
        .min_h(px(240.))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_3()
        .p_10()
        .child(
            div()
                .size(px(52.))
                .rounded_full()
                .bg(brand_soft_bg(cx))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    Icon::new(icon)
                        .with_size(px(24.))
                        .text_color(brand_accent(cx)),
                ),
        )
        .child(
            div()
                .text_lg()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(cx.theme().foreground)
                .child(title.to_owned()),
        )
        .child(
            div()
                .max_w(px(420.))
                .text_center()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(body.to_owned()),
        );
    if let Some(action) = action {
        box_el = box_el.child(div().pt_1().child(action));
    }
    box_el.into_any_element()
}

pub fn feed_skeleton_list(cx: &App) -> AnyElement {
    let card = |idx: usize| {
        div()
            .id(("feed-skeleton", idx))
            .p_4()
            .rounded_lg()
            .bg(cx.theme().muted)
            .border_1()
            .border_color(cx.theme().border)
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2p5()
                    .child(Skeleton::new().size(px(AVATAR_MD)).rounded_full())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1p5()
                            .child(Skeleton::new().w(px(120.)).h(px(14.)).rounded_sm())
                            .child(
                                Skeleton::new()
                                    .secondary()
                                    .w(px(84.))
                                    .h(px(11.))
                                    .rounded_sm(),
                            ),
                    ),
            )
            .child(Skeleton::new().w(px(280.)).h(px(16.)).rounded_sm())
            .child(Skeleton::new().secondary().w_full().h(px(14.)).rounded_sm())
            .child(
                Skeleton::new()
                    .secondary()
                    .w(px(360.))
                    .h(px(160.))
                    .rounded_md(),
            )
    };
    div()
        .px_6()
        .py_3()
        .flex()
        .flex_col()
        .gap_3()
        .child(card(0))
        .child(card(1))
        .child(card(2))
        .into_any_element()
}

pub fn video_skeleton_grid(viewport_width: f32, columns: usize, cx: &App) -> AnyElement {
    let width = grid_card_width(viewport_width, columns);
    let make_row = |row_idx: usize| {
        div().flex().gap_3().children((0..columns).map(|col_idx| {
            div()
                .id(("video-skeleton", row_idx * 10 + col_idx))
                .w(px(width))
                .rounded_lg()
                .overflow_hidden()
                .bg(cx.theme().muted)
                .border_1()
                .border_color(cx.theme().border)
                .flex()
                .flex_col()
                .child(
                    Skeleton::new()
                        .w(px(width))
                        .h(px(width * 0.5625))
                        .rounded_none(),
                )
                .child(
                    div()
                        .p_3()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(Skeleton::new().w_full().h(px(15.)).rounded_sm())
                        .child(
                            Skeleton::new()
                                .secondary()
                                .w(px(150.))
                                .h(px(12.))
                                .rounded_sm(),
                        ),
                )
        }))
    };
    div()
        .px_6()
        .py_2()
        .flex()
        .flex_col()
        .gap_3()
        .child(make_row(0))
        .child(make_row(1))
        .into_any_element()
}

pub fn section_card(title: &str, subtitle: Option<&str>, cx: &App) -> Div {
    let mut header = div().min_w_0().flex().flex_col().gap_1().child(
        div()
            .text_base()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(cx.theme().foreground)
            .child(title.to_owned()),
    );
    if let Some(sub) = subtitle {
        header = header.child(
            div()
                .min_w_0()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(sub.to_owned()),
        );
    }
    div()
        .min_w_0()
        .flex_shrink_0()
        .p_5()
        .rounded_lg()
        .bg(cx.theme().muted)
        .border_1()
        .border_color(cx.theme().border)
        .flex()
        .flex_col()
        .gap_4()
        .child(header)
}

pub fn form_field(
    label: &str,
    hint: Option<&str>,
    error: Option<&str>,
    control: impl IntoElement,
    cx: &App,
) -> Div {
    let danger_fg = tone_colors(Tone::Danger, cx.theme().is_dark()).fg;
    let mut field = div()
        .flex()
        .flex_col()
        .gap_1p5()
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(cx.theme().foreground)
                .child(label.to_owned()),
        )
        .child(control);
    if let Some(err) = error {
        field = field.child(div().text_xs().text_color(danger_fg).child(err.to_owned()));
    } else if let Some(hint) = hint {
        field = field.child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(hint.to_owned()),
        );
    }
    field
}

/// 建立不會在 flex_col 容器內被撐滿整行的緊湊按鈕列。
pub fn button_row() -> Div {
    div().flex().flex_wrap().items_center().gap_2()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn grid_columns_switches_at_wide_breakpoint() {
        assert_eq!(grid_columns(900.0), 2);
        assert_eq!(grid_columns(1280.0), 2);
        assert_eq!(grid_columns(1479.0), 2);
        assert_eq!(grid_columns(1480.0), 3);
        assert_eq!(grid_columns(1920.0), 3);
    }

    #[test]
    fn grid_card_width_respects_minimum_and_scales() {
        let narrow = grid_card_width(900.0, 2);
        let wide = grid_card_width(1600.0, 3);
        assert!(narrow >= 220.0);
        assert!(wide > 350.0);
    }

    #[test]
    fn truncate_chars_adds_ellipsis_only_when_exceeding_limit() {
        assert_eq!(truncate_chars("短字串", 10), "短字串");
        assert_eq!(truncate_chars("一二三四五六", 4), "一二三四…");
    }

    #[test]
    fn format_count_formats_wan_and_yi() {
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(12_500), "1.2萬");
        assert_eq!(format_count(150_000_000), "1.5億");
    }
}
