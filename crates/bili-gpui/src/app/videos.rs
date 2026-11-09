use super::common::*;
use super::*;
use crate::{
    assets::AppIcon,
    ui::{self, Tone},
};
use futures::{StreamExt, stream};
use gpui::component::{
    Icon, IconName, Sizable as _,
    input::{Input, InputEvent, InputState},
    select::{Select, SelectEvent},
    tab::{Tab, TabBar},
};

const VIDEO_SORTS: [&str; 4] = ["最新發布", "最多播放", "最多彈幕", "最多評論"];

#[derive(Clone, PartialEq, Eq)]
struct VideoScope {
    selected: Option<u64>,
    targets: Vec<u64>,
}

pub(super) struct VideoState {
    pub creator: Selector,
    pub creator_choices: Vec<Choice>,
    pub search: Entity<InputState>,
    pub mid: Option<u64>,
    pub initialized: bool,
    displayed_scope: Option<VideoScope>,
    items: Vec<VideoItem>,
    visible: Vec<usize>,
    list: ListState,
    columns: usize,
    sort: usize,
    page: u32,
    total: u64,
    estimated: bool,
    more: bool,
    loading: bool,
    version: u64,
    cancel: tokio_util::sync::CancellationToken,
    error: Option<String>,
    warning: Option<String>,
}

impl VideoState {
    pub fn new(window: &mut Window, cx: &mut App) -> Self {
        let initial_choices = up_choices(Vec::new());
        Self {
            creator: selector(initial_choices.clone(), window, cx),
            creator_choices: initial_choices,
            search: input("", "即時過濾標題、簡介或 UP 主", window, cx),
            mid: None,
            initialized: false,
            displayed_scope: None,
            items: Vec::new(),
            visible: Vec::new(),
            list: ListState::new(0, ListAlignment::Top, px(360.)),
            columns: 2,
            sort: 0,
            page: 0,
            total: 0,
            estimated: false,
            more: false,
            loading: false,
            version: 0,
            cancel: tokio_util::sync::CancellationToken::new(),
            error: None,
            warning: None,
        }
    }
}

impl BiliApp {
    pub(super) fn relayout_videos(&mut self, cx: &mut Context<Self>) {
        self.filter_videos(false, cx);
    }

    pub(super) fn subscribe_videos(&mut self, window: &Window, cx: &mut Context<Self>) {
        cx.subscribe_in(
            &self.videos.search,
            window,
            |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.filter_videos(true, cx);
                }
            },
        )
        .detach();
        cx.subscribe_in(
            &self.videos.creator,
            window,
            |this, _, event: &SelectEvent<_>, _, cx| {
                let SelectEvent::Confirm(selected) = event;
                this.videos.mid = selected.as_ref().and_then(|id| id.parse().ok());
                this.load_videos(true, cx);
            },
        )
        .detach();
    }

    pub(super) fn load_videos(&mut self, reset: bool, cx: &mut Context<Self>) {
        let Some(services) = self.services.clone() else {
            return;
        };
        if !reset && (self.videos.loading || !self.videos.more) {
            return;
        }
        let selected = self.videos.mid;
        let scoped = self.scoped_ups();
        let truncated = selected.is_none() && scoped.len() > ui::VIDEO_MULTI_UP_LIMIT;
        let targets = if let Some(mid) = selected {
            vec![mid]
        } else {
            scoped
                .into_iter()
                .take(ui::VIDEO_MULTI_UP_LIMIT)
                .map(|up| up.mid)
                .collect()
        };
        let page = if reset { 1 } else { self.videos.page + 1 };
        self.videos.version += 1;
        self.videos.cancel.cancel();
        self.videos.cancel = services.stop.child_token();
        let cancel = self.videos.cancel.clone();
        let version = self.videos.version;
        self.videos.loading = true;
        self.videos.initialized = true;
        self.videos.error = None;
        if reset {
            self.videos.warning = None;
            let scope = VideoScope {
                selected,
                targets: targets.clone(),
            };
            if self.videos.displayed_scope.as_ref() != Some(&scope) {
                self.videos.items.clear();
                self.videos.visible.clear();
                self.videos.list.reset(1);
                self.videos.page = 0;
                self.videos.total = 0;
                self.videos.estimated = false;
                self.videos.more = false;
            }
            self.videos.displayed_scope = Some(scope);
        }
        self.run(
            async move {
                tokio::select! { biased;
                    _ = cancel.cancelled() => Err(anyhow!("已取消影片載入")),
                    result = async move {
                        let results = stream::iter(targets)
                            .map(|mid| {
                                let api = services.api.clone();
                                async move {
                                    api.get_videos(mid, page, if selected.is_some() { 40 } else { 10 })
                                        .await
                                }
                            })
                            .buffer_unordered(4)
                            .collect::<Vec<_>>()
                            .await;
                        let mut combined = VideoPage::default();
                        let mut failed = Vec::new();
                        let mut warnings = Vec::new();
                        let mut succeeded = 0;
                        for result in results {
                            match result {
                                Ok(result) => {
                                    succeeded += 1;
                                    combined.list.extend(result.list);
                                    combined.total += result.total;
                                    combined.has_more |= result.has_more;
                                    combined.total_estimated |= result.total_estimated;
                                    if let Some(warning) = result.warning {
                                        warnings.push(warning);
                                    }
                                }
                                Err(error) => failed.push(error.to_string()),
                            }
                        }
                        anyhow::ensure!(
                            succeeded > 0 || failed.is_empty(),
                            "影片讀取失敗：{}",
                            failed.into_iter().take(2).collect::<Vec<_>>().join("；")
                        );
                        if !failed.is_empty() {
                            warnings.push("部分 UP 主影片未能更新".into());
                        }
                        if truncated {
                            warnings.push(format!(
                                "目前顯示前 {} 位 UP 主近期投稿；選擇單一 UP 主可瀏覽完整投稿",
                                ui::VIDEO_MULTI_UP_LIMIT
                            ));
                        }
                        warnings.sort();
                        warnings.dedup();
                        combined.warning = (!warnings.is_empty()).then(|| warnings.join("；"));
                        if selected.is_none() {
                            combined.has_more = false;
                            combined.total = combined.list.len() as u64;
                            combined.total_estimated = true;
                        }
                        Ok(combined)
                    } => result,
                }
            },
            cx,
            move |this, result, cx| {
                if this.videos.version != version {
                    return;
                }
                this.videos.loading = false;
                match result {
                    Ok(page_result) => {
                        if reset {
                            this.videos.items = page_result.list;
                        } else {
                            this.videos.items.extend(page_result.list);
                        }
                        let mut seen = HashSet::new();
                        this.videos
                            .items
                            .retain(|item| seen.insert(item.bvid.clone()));
                        this.videos.items.truncate(ui::VIDEO_MAX_ITEMS);
                        this.videos.page = page;
                        this.videos.total = page_result.total;
                        this.videos.estimated = page_result.total_estimated;
                        this.videos.more =
                            page_result.has_more && this.videos.items.len() < ui::VIDEO_MAX_ITEMS;
                        this.videos.error = None;
                        this.videos.warning = page_result.warning;
                        this.filter_videos(reset, cx);
                    }
                    Err(error) => this.videos.error = Some(error.to_string()),
                }
                cx.notify();
            },
        );
        cx.notify();
    }

    fn filter_videos(&mut self, reset: bool, cx: &mut Context<Self>) {
        let keyword = value(&self.videos.search, cx).trim().to_lowercase();
        let state = &mut self.videos;
        state.visible = state
            .items
            .iter()
            .enumerate()
            .filter(|(_, video)| {
                keyword.is_empty()
                    || [&video.title, &video.description, &video.author]
                        .iter()
                        .any(|text| text.to_lowercase().contains(&keyword))
            })
            .map(|(index, _)| index)
            .collect();
        let field = |video: &VideoItem| match state.sort {
            1 => video.play,
            2 => video.video_review,
            3 => video.comment,
            _ => video.pubdate,
        };
        state.visible.sort_by(|a, b| {
            field(&state.items[*b])
                .cmp(&field(&state.items[*a]))
                .then_with(|| state.items[*b].bvid.cmp(&state.items[*a].bvid))
        });
        let top = state.list.logical_scroll_top();
        state
            .list
            .reset(state.visible.len().div_ceil(state.columns) + 1);
        if !reset {
            state.list.scroll_to(top);
        }
        cx.notify();
    }

    #[inline(never)]
    pub(super) fn render_videos(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let viewport_width = f32::from(window.viewport_size().width);
        let columns =
            ui::scaled_grid_columns(viewport_width, self.snapshot.settings.appearance.card_scale);
        if columns != self.videos.columns {
            self.videos.columns = columns;
            self.filter_videos(false, cx);
        }
        let state = &self.videos;
        let selected_sort = state.sort;
        let loaded_count = state.items.len();
        let matched_count = state.visible.len();

        let toolbar = div()
            .flex_shrink_0()
            .px_6()
            .py_2p5()
            .border_b_1()
            .border_color(cx.theme().border)
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2p5()
                    .child(
                        div().w(px(210.)).flex_shrink_0().child(
                            Select::new(&state.creator)
                                .small()
                                .accessibility_label("投稿 UP 主")
                                .placeholder("全部 UP 主"),
                        ),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&state.search)
                                .small()
                                .cleanable(true)
                                .aria_label("搜尋已載入影片的標題、簡介或 UP 主")
                                .prefix(
                                    Icon::new(IconName::Search)
                                        .small()
                                        .text_color(cx.theme().muted_foreground),
                                ),
                        ),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("僅搜尋已載入的 {loaded_count} 部")),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        TabBar::new("video-sort-tabs")
                            .segmented()
                            .small()
                            .selected_index(selected_sort)
                            .children(
                                VIDEO_SORTS
                                    .into_iter()
                                    .map(|label| Tab::new().label(label).aria_label(label)),
                            )
                            .on_click(cx.listener(|this, idx: &usize, _, cx| {
                                this.videos.sort = *idx;
                                this.filter_videos(true, cx);
                            })),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("顯示 {matched_count} / {loaded_count} 部")),
                    ),
            );

        let mut panel = div().flex().flex_col().size_full().child(toolbar);

        if let Some(error) = state.error.clone() {
            let retry = Button::new("video-retry")
                .xsmall()
                .outline()
                .label("重新載入")
                .on_click(cx.listener(|this, _, _, cx| this.load_videos(true, cx)))
                .into_any_element();
            panel = panel.child(
                div()
                    .flex_shrink_0()
                    .px_6()
                    .pt_2p5()
                    .child(ui::notice_banner(Tone::Danger, &error, vec![retry], cx)),
            );
        } else if let Some(warning) = state.warning.clone() {
            let dismiss = Button::new("video-dismiss-warning")
                .xsmall()
                .ghost()
                .icon(IconName::Close)
                .accessibility_label("關閉提示")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.videos.warning = None;
                    cx.notify();
                }))
                .into_any_element();
            panel = panel.child(
                div()
                    .flex_shrink_0()
                    .px_6()
                    .pt_2p5()
                    .child(ui::notice_banner(
                        Tone::Warning,
                        &warning,
                        vec![dismiss],
                        cx,
                    )),
            );
        }

        if state.loading && state.items.is_empty() {
            return panel
                .child(
                    div()
                        .px_6()
                        .pt_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("正在載入投稿")
                        .child(crate::ascii::pong("video-initial-wait")),
                )
                .child(ui::video_skeleton_grid(viewport_width, columns, cx))
                .into_any_element();
        }
        if state.visible.is_empty() {
            return panel
                .child(ui::empty_state(
                    AppIcon::Video,
                    if state.error.is_some() && state.items.is_empty() {
                        "投稿載入失敗"
                    } else if self.snapshot.ups.is_empty() {
                        "尚未追蹤任何 UP 主"
                    } else {
                        "沒有符合條件的投稿影片"
                    },
                    if state.error.is_some() && state.items.is_empty() {
                        "請檢查連線後重新載入。"
                    } else {
                        "可切換 UP 主、分組或清除搜尋關鍵字。"
                    },
                    None,
                    cx,
                ))
                .into_any_element();
        }

        let loading = state.loading;
        let more = state.more;
        let list_state = state.list.clone();
        let entity = cx.entity();
        let view_id = cx.entity_id();
        panel
            .child(crate::scroll::smooth_list(
                "videos-smooth-list",
                view_id,
                list_state,
                move |row, window, cx| {
                    entity.update(cx, |this, cx| this.video_row(row, window, cx))
                },
            ))
            .child(
                div()
                    .flex_shrink_0()
                    .px_6()
                    .py_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(if more {
                                "單一 UP 主模式支援分頁載入更多投稿"
                            } else {
                                "已顯示目前範圍內載入的投稿"
                            }),
                    )
                    .child(
                        Button::new("video-load-more")
                            .small()
                            .outline()
                            .accessibility_label("載入更多投稿")
                            .child(crate::ascii::button_label(
                                "video-more-wait",
                                if loading {
                                    "載入中…"
                                } else {
                                    "載入更多投稿"
                                },
                                loading,
                            ))
                            .disabled(!more || loading)
                            .on_click(cx.listener(|this, _, _, cx| this.load_videos(false, cx))),
                    ),
            )
            .into_any_element()
    }

    #[inline(never)]
    fn video_row(&mut self, row: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let state = &self.videos;
        let indices = state
            .visible
            .iter()
            .skip(row * state.columns)
            .take(state.columns)
            .copied()
            .collect::<Vec<_>>();
        if indices.is_empty() {
            return div().h(px(8.)).into_any_element();
        }
        let viewport_width = f32::from(window.viewport_size().width);
        let width = ui::grid_card_width(viewport_width, state.columns);
        let mut cards = div().px_6().py_2().flex().gap_3();
        for index in indices {
            let video = self.videos.items[index].clone();
            let cover = self.picture(&video.pic, width, width * 0.5625, cx);
            let url = format!("https://www.bilibili.com/video/{}", video.bvid);
            let click_url = url.clone();
            let key_url = url.clone();
            let title_url = url.clone();
            let space = format!("https://space.bilibili.com/{}", video.mid);
            let cover_label = format!("開啟影片：{}", video.title);
            let hover_border = ui::hover_card_border(cx);
            cards = cards.child(
                div()
                    .w(px(width))
                    .flex_shrink_0()
                    .rounded_lg()
                    .overflow_hidden()
                    .bg(cx.theme().muted)
                    .border_1()
                    .border_color(cx.theme().border)
                    .hover(move |s| s.border_color(hover_border))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .id(SharedString::from(format!("video-image-{}", video.bvid)))
                            .relative()
                            .cursor_pointer()
                            .tab_index(0)
                            .aria_label(cover_label)
                            .hover(|s| s.opacity(0.92))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_url(click_url.clone(), cx)
                            }))
                            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                    this.open_url(key_url.clone(), cx);
                                }
                            }))
                            .child(cover)
                            .when(!video.duration.is_empty(), |d| {
                                d.child(
                                    div()
                                        .absolute()
                                        .right_2()
                                        .bottom_2()
                                        .px_1p5()
                                        .py_0p5()
                                        .rounded_sm()
                                        .bg(rgba(0x000000bf))
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(gpui::white())
                                        .child(video.duration.clone()),
                                )
                            }),
                    )
                    .child(
                        div()
                            .p_3()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .justify_between()
                            .gap_2()
                            .child(
                                div()
                                    .id(("video-title", index))
                                    .min_w_0()
                                    .min_h(px(40.))
                                    .line_clamp(2)
                                    .text_ellipsis()
                                    .cursor_pointer()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .hover(|s| s.underline())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.open_url(title_url.clone(), cx)
                                    }))
                                    .child(video.title),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(format!(
                                                "{} 播放 · {} 彈幕 · {}",
                                                ui::format_count(video.play),
                                                ui::format_count(video.video_review),
                                                format_time(video.pubdate.saturating_mul(1000))
                                            )),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .gap_2()
                                            .child(
                                                Button::new(("video-author", index))
                                                    .text()
                                                    .small()
                                                    .label(video.author)
                                                    .on_click(cx.listener(
                                                        move |this, _, _, cx| {
                                                            this.open_url(space.clone(), cx)
                                                        },
                                                    )),
                                            )
                                            .child(
                                                Button::new(("video-watch", index))
                                                    .small()
                                                    .outline()
                                                    .icon(IconName::ExternalLink)
                                                    .label("觀看")
                                                    .on_click(cx.listener(
                                                        move |this, _, _, cx| {
                                                            this.open_url(url.clone(), cx)
                                                        },
                                                    )),
                                            ),
                                    ),
                            ),
                    ),
            );
        }
        ui::card_enter(cards, ("video-row-enter", row), row % 4)
    }
}
