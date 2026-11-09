use super::common::*;
use super::*;
use crate::{
    assets::AppIcon,
    ui::{self, Tone},
};
use bili_core::feed::FeedQuery;
use gpui::component::{
    Icon, IconName, Sizable as _,
    input::{Input, InputEvent, InputState},
    select::{Select, SelectEvent},
    tab::{Tab, TabBar},
    tag::Tag,
};
use std::collections::{BTreeMap, HashMap};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CommentSegment {
    Text(String),
    Emote {
        token: String,
        url: String,
        size: u32,
    },
}

/// 將評論文字依 `emotes` 映射切分為純文字與表情包片段，未命中映射的 `[...]` 保留原字面文字。
pub(super) fn tokenize_comment_message(
    message: &str,
    emotes: &BTreeMap<String, CommentEmote>,
) -> Vec<CommentSegment> {
    if emotes.is_empty() || !message.contains('[') {
        return if message.is_empty() {
            Vec::new()
        } else {
            vec![CommentSegment::Text(message.to_owned())]
        };
    }

    let mut segments = Vec::new();
    let mut buffer = String::new();
    let mut rest = message;

    while let Some(open) = rest.find('[') {
        buffer.push_str(&rest[..open]);
        let after_open = &rest[open..];
        let close_candidate = after_open[1..]
            .find([']', '[', '\n'])
            .map(|idx| (idx + 1, after_open.as_bytes()[idx + 1]));

        match close_candidate {
            Some((end_idx, b']')) if end_idx <= 64 => {
                let token = &after_open[..=end_idx];
                if let Some(emote) = emotes.get(token) {
                    if !buffer.is_empty() {
                        segments.push(CommentSegment::Text(std::mem::take(&mut buffer)));
                    }
                    segments.push(CommentSegment::Emote {
                        token: token.to_owned(),
                        url: emote.url.clone(),
                        size: emote.size,
                    });
                    rest = &after_open[end_idx + 1..];
                } else {
                    buffer.push_str(token);
                    rest = &after_open[end_idx + 1..];
                }
            }
            _ => {
                buffer.push('[');
                rest = &after_open[1..];
            }
        }
    }
    buffer.push_str(rest);
    if !buffer.is_empty() {
        segments.push(CommentSegment::Text(buffer));
    }
    segments
}

fn split_wrap_units(text: &str) -> Vec<String> {
    let mut units = Vec::new();
    let mut current = String::new();
    let mut count = 0usize;
    for ch in text.chars() {
        current.push(ch);
        count += 1;
        if ch.is_whitespace()
            || matches!(ch, '，' | '。' | '！' | '？' | '；' | '、' | ',')
            || count >= 14
        {
            units.push(std::mem::take(&mut current));
            count = 0;
        }
    }
    if !current.is_empty() {
        units.push(current);
    }
    units
}

#[derive(Default, Clone)]
pub(super) struct Comments {
    list: Vec<CommentItem>,
    page: u32,
    total: u64,
    more: bool,
    loading: bool,
    error: Option<String>,
}

pub(super) struct FeedState {
    pub group: String,
    pub mid: Option<u64>,
    pub creator: Selector,
    pub groups: Selector,
    pub creator_choices: Vec<Choice>,
    pub group_choices: Vec<Choice>,
    pub search: Entity<InputState>,
    pub initialized: bool,
    pub items: Vec<DynamicItem>,
    pub list: ListState,
    kind: Option<DynamicType>,
    keyword: String,
    displayed_scope: Option<FeedScope>,
    cursor: Option<String>,
    more: bool,
    loading: bool,
    version: u64,
    cancel: CancellationToken,
    error: Option<String>,
    warning: Option<String>,
    retry_reset: bool,
    expanded: Option<String>,
    comments: HashMap<String, Comments>,
}

#[derive(Clone, PartialEq, Eq)]
struct FeedScope {
    mids: Vec<u64>,
    kind: Option<DynamicType>,
    keyword: String,
}

const FEED_KINDS: [(Option<DynamicType>, &str); 7] = [
    (None, "全部"),
    (Some(DynamicType::Video), "影片"),
    (Some(DynamicType::Draw), "圖文"),
    (Some(DynamicType::Text), "文字"),
    (Some(DynamicType::Article), "專欄"),
    (Some(DynamicType::Forward), "轉發"),
    (Some(DynamicType::Live), "直播"),
];

impl FeedState {
    pub(super) fn new(window: &mut Window, cx: &mut App) -> Self {
        let initial_ups = up_choices(Vec::new());
        let initial_groups = group_choices(default_groups());
        Self {
            group: "all".into(),
            mid: None,
            creator: selector(initial_ups.clone(), window, cx),
            groups: selector(initial_groups.clone(), window, cx),
            creator_choices: initial_ups,
            group_choices: initial_groups,
            search: input("", "搜尋標題、內文或 UP 主（Enter 搜尋）", window, cx),
            initialized: false,
            items: Vec::new(),
            list: ListState::new(0, ListAlignment::Top, px(500.)),
            kind: None,
            keyword: String::new(),
            displayed_scope: None,
            cursor: None,
            more: false,
            loading: false,
            version: 0,
            cancel: CancellationToken::new(),
            error: None,
            warning: None,
            retry_reset: true,
            expanded: None,
            comments: HashMap::new(),
        }
    }
}

impl BiliApp {
    pub(super) fn subscribe_feeds(&mut self, window: &Window, cx: &mut Context<Self>) {
        for stream in 0..2 {
            cx.subscribe_in(
                &self.feeds[stream].search,
                window,
                move |this, _, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.load_feed(stream, true, cx);
                    }
                },
            )
            .detach();
            cx.subscribe_in(
                &self.feeds[stream].creator,
                window,
                move |this, _, event: &SelectEvent<_>, _, cx| {
                    let SelectEvent::Confirm(selected) = event;
                    this.feeds[stream].mid = selected.as_ref().and_then(|id| id.parse().ok());
                    this.load_feed(stream, true, cx);
                },
            )
            .detach();
            cx.subscribe_in(
                &self.feeds[stream].groups,
                window,
                move |this, _, event: &SelectEvent<_>, window, cx| {
                    let SelectEvent::Confirm(selected) = event;
                    let id = selected.clone().unwrap_or_else(|| "all".into());
                    if stream == 0 {
                        this.select_group(id, window, cx);
                    } else {
                        this.feeds[1].group = id;
                        this.feeds[1].mid = None;
                        this.sync_selectors(window, cx);
                        this.load_feed(1, true, cx);
                    }
                },
            )
            .detach();
        }
    }

    pub(super) fn load_feed(&mut self, stream: usize, reset: bool, cx: &mut Context<Self>) {
        let Some(services) = self.services.clone() else {
            return;
        };
        if !reset && (self.feeds[stream].loading || !self.feeds[stream].more) {
            return;
        }
        let groups = self.groups();
        let mut mids = group_members(&self.snapshot.ups, &groups, &self.feeds[stream].group)
            .iter()
            .map(|up| up.mid)
            .collect::<Vec<_>>();
        if let Some(mid) = self.feeds[stream].mid {
            mids.retain(|uid| *uid == mid);
        }
        let state = &mut self.feeds[stream];
        state.cancel.cancel();
        state.cancel = services.stop.child_token();
        state.version += 1;
        let version = state.version;
        if reset {
            state.keyword = value(&state.search, cx).trim().to_owned();
            state.cursor = None;
            state.expanded = None;
            state.comments.clear();
            let mut scope_mids = mids.clone();
            scope_mids.sort_unstable();
            scope_mids.dedup();
            let scope = FeedScope {
                mids: scope_mids,
                kind: state.kind,
                keyword: state.keyword.clone(),
            };
            if state.displayed_scope.as_ref() != Some(&scope) {
                state.items.clear();
                state.more = false;
                state.list.reset(1);
            }
            state.displayed_scope = Some(scope);
        }
        state.initialized = true;
        state.loading = true;
        state.error = None;
        if reset {
            state.warning = None;
        }
        state.retry_reset = reset;
        let query = FeedQuery {
            mids,
            kind: state.kind,
            keyword: state.keyword.clone(),
            cursor: if reset { None } else { state.cursor.clone() },
            limit: 20,
        };
        let cancel = state.cancel.clone();
        self.run(
            async move { services.feed.fetch(query, cancel).await },
            cx,
            move |this, result, cx| {
                let state = &mut this.feeds[stream];
                if state.version != version {
                    return;
                }
                state.loading = false;
                match result {
                    Ok(page) => {
                        let previous_length = state.items.len();
                        if reset {
                            state.items = page.items;
                        } else {
                            let mut seen: HashSet<_> =
                                state.items.iter().map(|item| item.id_str.clone()).collect();
                            state.items.extend(
                                page.items
                                    .into_iter()
                                    .filter(|item| seen.insert(item.id_str.clone())),
                            );
                        }
                        state.cursor = page.next_cursor;
                        state.more = page.has_more;
                        state.error = None;
                        state.warning = page.warning;
                        if state.items.len() >= ui::FEED_MAX_ITEMS {
                            state.items.truncate(ui::FEED_MAX_ITEMS);
                            state.more = false;
                            state.warning =
                                Some("本欄已載入 2000 則上限；請縮小分組或搜尋範圍".into());
                        }
                        if reset {
                            state.list.reset(state.items.len() + 1);
                        } else {
                            state.list.splice(
                                previous_length..previous_length + 1,
                                state.items.len().saturating_sub(previous_length) + 1,
                            );
                        }
                    }
                    Err(error) => state.error = Some(error.to_string()),
                }
                cx.notify();
            },
        );
        cx.notify();
    }

    #[inline(never)]
    pub(super) fn render_feed_workspace(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let narrow = f32::from(window.viewport_size().width) < ui::BP_DUAL_STREAM;
        let visible_stream = if self.dual && narrow {
            self.mobile_stream
        } else {
            0
        };
        let primary = self.render_stream(visible_stream, window, cx);
        let mut columns = div().flex().flex_1().min_h_0().min_w_0().child(primary);
        if self.dual && !narrow {
            columns = columns
                .child(div().w(px(1.)).h_full().bg(cx.theme().border))
                .child(self.render_stream(1, window, cx));
        }
        let mut workspace = div().flex().flex_col().size_full();
        if self.dual && narrow {
            workspace = workspace.child(
                div()
                    .flex_shrink_0()
                    .px_6()
                    .pt_2p5()
                    .pb_1()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        TabBar::new("mobile-stream-tabs")
                            .segmented()
                            .small()
                            .selected_index(self.mobile_stream)
                            .child(Tab::new().label("主訊息流").aria_label("主訊息流"))
                            .child(Tab::new().label("副訊息流").aria_label("副訊息流"))
                            .on_click(cx.listener(|this, selected: &usize, _, cx| {
                                this.mobile_stream = *selected;
                                cx.notify();
                            })),
                    ),
            );
        }
        workspace.child(columns).into_any_element()
    }

    #[inline(never)]
    fn render_stream(
        &mut self,
        stream: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = &self.feeds[stream];
        let selected_kind_index = FEED_KINDS
            .iter()
            .position(|(kind, _)| *kind == state.kind)
            .unwrap_or(0);
        let unread_in_stream = state
            .items
            .iter()
            .filter(|item| !self.read_ids.contains(&item.id_str))
            .count();

        let mut row1 = div().flex().items_center().justify_between().gap_2();

        let mut row1_left = div().flex().items_center().gap_2().min_w_0();
        if self.dual {
            row1_left = row1_left.child(Tag::primary().outline().xsmall().child(if stream == 0 {
                "主欄"
            } else {
                "副欄"
            }));
        }
        let mut type_bar = TabBar::new(("feed-types-bar", stream))
            .segmented()
            .selected_index(selected_kind_index)
            .children(
                FEED_KINDS
                    .into_iter()
                    .map(|(_, label)| Tab::new().label(label).aria_label(label)),
            )
            .on_click(cx.listener(move |this, idx: &usize, _, cx| {
                if let Some((kind, _)) = FEED_KINDS.get(*idx) {
                    this.feeds[stream].kind = *kind;
                    this.load_feed(stream, true, cx);
                }
            }));
        type_bar = if self.dual {
            type_bar.xsmall()
        } else {
            type_bar.small()
        };
        row1_left = row1_left.child(type_bar);

        let read_all_label = match (self.dual, unread_in_stream > 0) {
            (true, true) => format!("已讀 ({unread_in_stream})"),
            (true, false) => "已讀".into(),
            (false, true) => format!("本欄已讀 ({unread_in_stream})"),
            (false, false) => "本欄已讀".into(),
        };
        let mut read_all_btn = Button::new(("feed-read-all", stream))
            .ghost()
            .icon(AppIcon::CheckRead)
            .label(read_all_label)
            .disabled(unread_in_stream == 0)
            .on_click(cx.listener(move |this, _, _, cx| {
                let ids = this.feeds[stream]
                    .items
                    .iter()
                    .filter(|item| !this.read_ids.contains(&item.id_str))
                    .map(|item| item.id_str.clone())
                    .collect();
                this.mark_read(ids, cx);
            }));
        read_all_btn = if self.dual {
            read_all_btn.xsmall()
        } else {
            read_all_btn.small()
        };
        row1 = row1.child(row1_left).child(read_all_btn);

        let mut row2 = div().flex().items_center().gap_2();
        if self.dual {
            row2 = row2.child(
                div().w(px(124.)).flex_shrink_0().child(
                    Select::new(&state.groups)
                        .small()
                        .accessibility_label("訊息流分組")
                        .placeholder("分組"),
                ),
            );
        }
        let creator_width = if self.dual { 156. } else { 196. };
        row2 = row2
            .child(
                div().w(px(creator_width)).flex_shrink_0().child(
                    Select::new(&state.creator)
                        .small()
                        .accessibility_label("篩選 UP 主")
                        .placeholder("全部 UP 主"),
                ),
            )
            .child(
                div().min_w_0().flex_1().child(
                    Input::new(&state.search)
                        .small()
                        .cleanable(true)
                        .aria_label("搜尋動態標題、內文或 UP 主")
                        .prefix(
                            Icon::new(IconName::Search)
                                .small()
                                .text_color(cx.theme().muted_foreground),
                        ),
                ),
            )
            .child(
                Button::new(("feed-search", stream))
                    .small()
                    .outline()
                    .label("搜尋")
                    .on_click(cx.listener(move |this, _, _, cx| this.load_feed(stream, true, cx))),
            );

        let toolbar = div()
            .flex_shrink_0()
            .px_6()
            .py_2p5()
            .border_b_1()
            .border_color(cx.theme().border)
            .flex()
            .flex_col()
            .gap_2()
            .child(row1)
            .child(row2);

        let mut panel = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .child(toolbar);

        if let Some(error) = state.error.clone() {
            let reset = state.retry_reset;
            let retry = Button::new(("feed-retry", stream))
                .xsmall()
                .outline()
                .label("重試載入")
                .on_click(cx.listener(move |this, _, _, cx| this.load_feed(stream, reset, cx)))
                .into_any_element();
            panel = panel.child(
                div()
                    .flex_shrink_0()
                    .px_6()
                    .pt_2p5()
                    .child(ui::notice_banner(Tone::Danger, &error, vec![retry], cx)),
            );
        } else if let Some(warning) = state.warning.clone() {
            let retry = Button::new(("feed-warning-retry", stream))
                .xsmall()
                .outline()
                .label("重新載入")
                .on_click(cx.listener(move |this, _, _, cx| this.load_feed(stream, true, cx)))
                .into_any_element();
            let dismiss = Button::new(("feed-dismiss-warning", stream))
                .xsmall()
                .ghost()
                .icon(IconName::Close)
                .accessibility_label("關閉提示")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.feeds[stream].warning = None;
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
                        vec![retry, dismiss],
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
                        .child("正在載入動態")
                        .child(crate::ascii::pong(("feed-initial-wait", stream))),
                )
                .child(ui::feed_skeleton_list(cx))
                .into_any_element();
        }
        if state.items.is_empty() {
            let empty_view = if self.snapshot.ups.is_empty() {
                let go_settings = Button::new(("feed-empty-add-up", stream))
                    .primary()
                    .icon(AppIcon::UserPlus)
                    .label("前往新增 UP 主")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.settings.select_section(0);
                        this.switch_view(View::Settings, window, cx);
                    }))
                    .into_any_element();
                ui::empty_state(
                    AppIcon::UserPlus,
                    "尚未追蹤任何 UP 主",
                    "前往「設定與 UP 主管理」輸入 UID、空間網址、b23.tv 或 BV 號即可開始追蹤。",
                    Some(go_settings),
                    cx,
                )
            } else {
                ui::empty_state(
                    AppIcon::Feed,
                    "目前沒有顯示動態",
                    "可調整分組、類型或搜尋條件；若 B 站暫時沒有回傳資料，請稍後按「立即刷新」重試。",
                    None,
                    cx,
                )
            };
            return panel.child(empty_view).into_any_element();
        }

        let total = state.items.len();
        let more = state.more;
        let loading = state.loading;
        let list_state = state.list.clone();
        let entity = cx.entity();
        let view_id = cx.entity_id();
        panel
            .child(crate::scroll::smooth_list(
                ("feed-smooth-list", stream),
                view_id,
                list_state,
                move |index, window, cx| {
                    entity.update(cx, |this, cx| this.feed_card(stream, index, window, cx))
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
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("已載入 {total} 則")),
                    )
                    .child(
                        Button::new(("feed-more", stream))
                            .small()
                            .outline()
                            .accessibility_label("載入更多動態")
                            .child(crate::ascii::button_label(
                                ("feed-more-wait", stream),
                                if loading {
                                    "載入中…"
                                } else {
                                    "載入更多動態"
                                },
                                loading,
                            ))
                            .disabled(loading || !more)
                            .on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.load_feed(stream, false, cx)
                                }),
                            ),
                    ),
            )
            .into_any_element()
    }

    #[inline(never)]
    fn feed_card(
        &mut self,
        stream: usize,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(item) = self.feeds[stream].items.get(index).cloned() else {
            return div().h(px(8.)).into_any_element();
        };
        let read = self.read_ids.contains(&item.id_str);
        let up = self.snapshot.ups.iter().find(|up| up.mid == item.mid);
        let custom_note = up
            .and_then(|u| u.custom_note.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let face = if item.author_face.is_empty() {
            up.map(|up| up.face.as_str()).unwrap_or_default()
        } else {
            &item.author_face
        }
        .to_owned();
        let name = if item.author_name.is_empty() {
            up.map(|up| up.name.clone())
                .unwrap_or_else(|| format!("UP {}", item.mid))
        } else {
            item.author_name.clone()
        };

        let avatar = self.avatar(&face, ui::AVATAR_MD, cx);
        let space = format!("https://space.bilibili.com/{}", item.mid);
        let avatar_space = space.clone();
        let url = item
            .jump_url
            .clone()
            .unwrap_or_else(|| format!("https://t.bilibili.com/{}", item.id_str));
        let title_url = url.clone();
        let id = item.id_str.clone();
        let read_id = id.clone();
        let comment_id = id.clone();

        let mut author_line = div().flex().items_center().gap_2().min_w_0().child(
            Button::new(SharedString::from(format!("feed-author-{stream}-{id}")))
                .text()
                .small()
                .label(name.clone())
                .tooltip("開啟 UP 主個人空間")
                .font_weight(FontWeight::SEMIBOLD)
                .on_click(cx.listener(move |this, _, _, cx| this.open_url(space.clone(), cx))),
        );
        if let Some(note) = custom_note {
            author_line = author_line.child(Tag::secondary().xsmall().child(note));
        }
        if !read {
            author_line = author_line.child(Tag::primary().outline().xsmall().child("未讀"));
            if index < 3 && self.snapshot.settings.appearance.ascii_animation_enabled {
                author_line =
                    author_line.child(div().text_color(ui::brand_accent(cx).opacity(0.8)).child(
                        crate::ascii::decoration(
                            SharedString::from(format!("unread-spark-{stream}-{id}")),
                            crate::ascii::Style::Unread,
                        ),
                    ));
            }
        }

        let header =
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .id(SharedString::from(format!("feed-avatar-{stream}-{id}")))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_url(avatar_space.clone(), cx)
                        }))
                        .child(avatar),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .child(author_line)
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!(
                                    "{} · {}",
                                    format_time(item.pub_time),
                                    item.r#type.label()
                                )),
                        ),
                )
                .child(
                    Button::new(SharedString::from(format!("read-{stream}-{id}")))
                        .ghost()
                        .xsmall()
                        .icon(AppIcon::CheckRead)
                        .label(if read { "已讀" } else { "標記已讀" })
                        .disabled(read)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.mark_read(vec![read_id.clone()], cx)
                        })),
                );

        let mut body = div()
            .flex()
            .flex_col()
            .gap_3()
            .flex_1()
            .min_w_0()
            .p_4()
            .child(header);

        if let Some(title) = &item.title
            && !title.is_empty()
        {
            let title_text = title.clone();
            body = body.child(
                div()
                    .id(SharedString::from(format!("feed-title-{stream}-{id}")))
                    .cursor_pointer()
                    .text_base()
                    .font_weight(if read {
                        FontWeight::MEDIUM
                    } else {
                        FontWeight::SEMIBOLD
                    })
                    .text_color(if read {
                        cx.theme().foreground.opacity(0.88)
                    } else {
                        cx.theme().foreground
                    })
                    .hover(|s| s.underline())
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.open_url(title_url.clone(), cx)),
                    )
                    .child(title_text),
            );
        }

        if let Some(text) = &item.text
            && !text.is_empty()
        {
            body = body.child(
                div()
                    .text_sm()
                    .line_height(relative(1.55))
                    .text_color(cx.theme().foreground.opacity(0.92))
                    .child(ui::truncate_chars(text, ui::FEED_TEXT_PREVIEW_CHARS)),
            );
            if text.chars().count() > ui::FEED_TEXT_PREVIEW_CHARS {
                let full_text = text.clone();
                body = body.child(
                    div().flex().child(
                        Button::new(SharedString::from(format!("full-text-{stream}-{id}")))
                            .ghost()
                            .small()
                            .label("展開全文")
                            .on_click(move |_, window, cx| {
                                let content = full_text.clone();
                                window.open_dialog(cx, move |dialog, _, _| {
                                    dialog.title("動態完整內文").width(px(760.)).child(
                                        div()
                                            .id("full-dynamic-text")
                                            .max_h(px(440.))
                                            .overflow_y_scroll()
                                            .text_sm()
                                            .line_height(relative(1.65))
                                            .child(content.clone()),
                                    )
                                });
                            }),
                    ),
                );
            }
        }

        let two_columns =
            self.dual && f32::from(window.viewport_size().width) >= ui::BP_DUAL_STREAM;
        let available =
            ui::feed_card_inner_width(f32::from(window.viewport_size().width), two_columns);

        if let Some(images) = &item.images
            && !images.is_empty()
        {
            let urls = images.clone();
            let mut grid = div().flex().flex_wrap().gap_2();
            let tile_width = if images.len() == 1 {
                available.min(420. * self.snapshot.settings.appearance.card_scale)
            } else {
                ((available - 8.) / 2.).max(120.)
            };
            let tile_height = (tile_width * 0.64)
                .clamp(104., 220. * self.snapshot.settings.appearance.card_scale);
            for (position, image) in images.iter().take(4).enumerate() {
                let picture = self.picture(image, tile_width, tile_height, cx);
                let urls_click = urls.clone();
                let urls_key = urls.clone();
                let extra = if position == 3 && images.len() > 4 {
                    Some(images.len() - 4)
                } else {
                    None
                };
                let label = format!("預覽第 {} 張圖片（共 {} 張）", position + 1, images.len());
                let mut tile = div()
                    .id(SharedString::from(format!(
                        "album-tile-{stream}-{id}-{position}"
                    )))
                    .relative()
                    .rounded_lg()
                    .overflow_hidden()
                    .cursor_pointer()
                    .tab_index(0)
                    .aria_label(label.clone())
                    .border_1()
                    .border_color(cx.theme().border)
                    .hover(|s| s.opacity(0.92))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.preview_images(urls_click.clone(), position, window, cx)
                    }))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            this.preview_images(urls_key.clone(), position, window, cx);
                        }
                    }))
                    .child(picture);

                if let Some(rest) = extra {
                    tile = tile.child(
                        div()
                            .absolute()
                            .right_2()
                            .bottom_2()
                            .px_2()
                            .py_0p5()
                            .rounded_md()
                            .bg(rgba(0x000000b8))
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(gpui::white())
                            .child(format!("+{rest} 張")),
                    );
                }
                grid = grid.child(tile);
            }
            body = body.child(grid);
        } else if let Some(cover) = &item.cover
            && !cover.is_empty()
        {
            let width = available.min(if item.r#type == DynamicType::Video {
                420.
            } else {
                640.
            });
            let picture = self.picture(cover, width, width * 0.5625, cx);
            let covers_click = vec![cover.clone()];
            let covers_key = covers_click.clone();
            body = body.child(
                div()
                    .id(SharedString::from(format!("cover-tile-{stream}-{id}")))
                    .w(px(width))
                    .relative()
                    .rounded_lg()
                    .overflow_hidden()
                    .cursor_pointer()
                    .tab_index(0)
                    .aria_label("預覽動態封面原圖")
                    .border_1()
                    .border_color(cx.theme().border)
                    .hover(|s| s.opacity(0.93))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.preview_images(covers_click.clone(), 0, window, cx)
                    }))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            this.preview_images(covers_key.clone(), 0, window, cx);
                        }
                    }))
                    .child(picture)
                    .child(
                        div()
                            .absolute()
                            .right_2()
                            .bottom_2()
                            .px_2()
                            .py_0p5()
                            .rounded_md()
                            .bg(rgba(0x000000a8))
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_xs()
                            .text_color(gpui::white())
                            .child(
                                Icon::new(AppIcon::Image)
                                    .with_size(px(12.))
                                    .text_color(gpui::white()),
                            )
                            .child("預覽封面"),
                    ),
            );
        }

        if let Some(reservation) = &item.reservation {
            let title = if reservation.title.is_empty() {
                "預約資訊".to_owned()
            } else {
                format!("預約：{}", reservation.title)
            };
            let details = [reservation.time.as_deref(), reservation.detail.as_deref()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
            let original_url = format!("https://t.bilibili.com/{id}");
            body = body.child(
                div()
                    .p_3()
                    .rounded_lg()
                    .bg(cx.theme().muted)
                    .border_1()
                    .border_color(cx.theme().border)
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(title),
                            )
                            .when(!details.is_empty(), |column| {
                                column.child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(details),
                                )
                            }),
                    )
                    .child(
                        Button::new(SharedString::from(format!("reserve-{stream}-{id}")))
                            .outline()
                            .xsmall()
                            .label("在 B 站查看")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_url(original_url.clone(), cx)
                            })),
                    ),
            );
        }

        let has_content = item.title.as_deref().is_some_and(|s| !s.is_empty())
            || item.text.as_deref().is_some_and(|s| !s.is_empty())
            || item
                .images
                .as_ref()
                .is_some_and(|images| !images.is_empty())
            || item.cover.as_deref().is_some_and(|s| !s.is_empty())
            || item.orig.is_some()
            || item.reservation.is_some();
        if !has_content {
            body = body.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("這則動態沒有可顯示的內容，可在 B 站查看原文。"),
            );
        }

        if let Some(orig) = &item.orig {
            let original_url = format!("https://t.bilibili.com/{}", orig.id_str);
            let mut quote = div()
                .p_3()
                .rounded_md()
                .bg(cx.theme().background)
                .border_1()
                .border_color(cx.theme().border)
                .border_l_2()
                .flex()
                .flex_col()
                .gap_1p5()
                .text_sm()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(ui::brand_accent(cx))
                                .child(format!("轉發自 @{}", orig.author_name)),
                        )
                        .child(
                            Button::new(SharedString::from(format!("orig-{stream}-{id}")))
                                .ghost()
                                .xsmall()
                                .icon(IconName::ExternalLink)
                                .label("原動態")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.open_url(original_url.clone(), cx)
                                })),
                        ),
                );
            if let Some(orig_title) = orig.title.as_deref().filter(|s| !s.is_empty()) {
                quote = quote.child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(orig_title.to_owned()),
                );
            }
            if let Some(orig_text) = orig.text.as_deref().filter(|s| !s.is_empty()) {
                quote = quote.child(div().text_color(cx.theme().muted_foreground).child(
                    ui::truncate_chars(orig_text, ui::FORWARD_TEXT_PREVIEW_CHARS),
                ));
            }
            body = body.child(quote);
        }

        let stat = item.stat.clone().unwrap_or_default();
        let expanded = self.feeds[stream].expanded.as_deref() == Some(&id);
        body = body.child(
            div()
                .pt_1()
                .flex()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_4()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(Icon::new(AppIcon::Heart).with_size(px(13.)))
                                .child(ui::format_count(stat.like)),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(Icon::new(AppIcon::Share).with_size(px(13.)))
                                .child(ui::format_count(stat.forward)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .child(
                            Button::new(SharedString::from(format!("comments-{stream}-{id}")))
                                .ghost()
                                .small()
                                .selected(expanded)
                                .icon(AppIcon::MessageSquare)
                                .label(if expanded {
                                    "收起評論".into()
                                } else {
                                    format!("評論 {}", ui::format_count(stat.reply))
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if this.feeds[stream].expanded.as_deref() == Some(&comment_id) {
                                        this.feeds[stream].expanded = None;
                                    } else {
                                        this.feeds[stream].expanded = Some(comment_id.clone());
                                        if !this.feeds[stream].comments.contains_key(&comment_id) {
                                            this.load_comments(stream, index, 1, cx);
                                        }
                                    }
                                    this.feeds[stream].list.splice(index..index + 1, 1);
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new(SharedString::from(format!("feed-open-{stream}-{id}")))
                                .outline()
                                .small()
                                .icon(IconName::ExternalLink)
                                .label("在 B 站開啟")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.open_url(url.clone(), cx)
                                })),
                        ),
                ),
        );

        if expanded {
            let comments = self.feeds[stream]
                .comments
                .get(&id)
                .cloned()
                .unwrap_or_default();
            let mut area = div()
                .flex()
                .flex_col()
                .gap_3()
                .pt_3()
                .border_t_1()
                .border_color(cx.theme().border);
            if comments.loading && comments.list.is_empty() {
                area = area.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("正在載入評論")
                        .child(crate::ascii::waiting(SharedString::from(format!(
                            "comments-wait-{stream}-{id}"
                        )))),
                );
            }
            if let Some(error) = &comments.error {
                let retry =
                    Button::new(SharedString::from(format!("comments-retry-{stream}-{id}")))
                        .xsmall()
                        .outline()
                        .label("重試評論")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.load_comments(stream, index, 1, cx)
                        }))
                        .into_any_element();
                area = area.child(ui::notice_banner(Tone::Danger, error, vec![retry], cx));
            }
            for comment in &comments.list {
                let comment_avatar = self.avatar(&comment.avatar, ui::AVATAR_SM, cx);
                let message_el = self.render_comment_content(
                    &comment.rpid,
                    &comment.message,
                    &comment.emotes,
                    &comment.pictures,
                    false,
                    cx,
                );
                let mut reply = div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .text_sm()
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .items_center()
                            .child(comment_avatar)
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_xs()
                                    .child(comment.uname.clone()),
                            )
                            .when(comment.level > 0, |d| {
                                d.child(
                                    Tag::secondary()
                                        .xsmall()
                                        .child(format!("Lv.{}", comment.level)),
                                )
                            })
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!("· {} 讚", ui::format_count(comment.like))),
                            ),
                    )
                    .child(div().pl_8().child(message_el));
                for sub in comment.sub_replies.iter().flatten().take(3) {
                    let sub_body = self.render_comment_content(
                        &sub.rpid,
                        &sub.message,
                        &sub.emotes,
                        &sub.pictures,
                        true,
                        cx,
                    );
                    reply = reply.child(
                        div()
                            .ml_8()
                            .pl_2p5()
                            .py_0p5()
                            .border_l_2()
                            .border_color(cx.theme().border)
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!("{}：", sub.uname)),
                            )
                            .child(sub_body),
                    );
                }
                area = area.child(reply);
            }
            if comments.list.is_empty() && !comments.loading && comments.error.is_none() {
                area = area.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("目前沒有評論"),
                );
            }
            if comments.more {
                let next = comments.page + 1;
                area = area.child(
                    div().flex().child(
                        Button::new(SharedString::from(format!("comments-more-{stream}-{id}")))
                            .small()
                            .ghost()
                            .accessibility_label("載入更多評論")
                            .child(crate::ascii::button_label(
                                SharedString::from(format!("comments-more-wait-{stream}-{id}")),
                                format!(
                                    "載入更多評論（{} / {}）",
                                    comments.list.len(),
                                    comments.total
                                ),
                                comments.loading,
                            ))
                            .disabled(comments.loading)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.load_comments(stream, index, next, cx)
                            })),
                    ),
                );
            }
            body = body.child(ui::enter_transition(
                area,
                SharedString::from(format!("comments-expand-{stream}-{id}")),
            ));
        }

        let hover_border = if read {
            ui::hover_card_border(cx)
        } else {
            ui::brand_accent(cx)
        };
        let card = div()
            .flex()
            .rounded_lg()
            .overflow_hidden()
            .bg(cx.theme().muted)
            .border_1()
            .border_color(if read {
                cx.theme().border
            } else {
                ui::unread_card_border(cx)
            })
            .hover(move |s| s.border_color(hover_border))
            .child(div().w(px(3.)).flex_shrink_0().bg(if read {
                gpui::transparent_black()
            } else {
                ui::brand_accent(cx)
            }))
            .child(body);

        ui::card_enter(
            div().px_6().py_2().child(card),
            SharedString::from(format!("feed-card-enter-{stream}-{id}")),
            index % 4,
        )
    }

    fn render_comment_content(
        &mut self,
        rpid: &str,
        message: &str,
        emotes: &BTreeMap<String, CommentEmote>,
        pictures: &[String],
        compact: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut container = div().flex().flex_col().gap_1p5();
        if !message.is_empty() {
            if emotes.is_empty() {
                let text_div = if compact {
                    div().text_xs().text_color(cx.theme().muted_foreground)
                } else {
                    div().text_sm()
                };
                container = container.child(
                    text_div
                        .line_height(relative(1.5))
                        .child(message.to_owned()),
                );
            } else {
                let mut lines_col = div().flex().flex_col().gap_1();
                for line in message.split('\n') {
                    let segments = tokenize_comment_message(line, emotes);
                    let mut line_row = div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_x_0p5()
                        .gap_y_1()
                        .when(compact, |d| {
                            d.text_xs().text_color(cx.theme().muted_foreground)
                        })
                        .when(!compact, |d| d.text_sm());

                    for segment in segments {
                        match segment {
                            CommentSegment::Text(text) => {
                                for unit in split_wrap_units(&text) {
                                    line_row = line_row.child(unit);
                                }
                            }
                            CommentSegment::Emote { token, url, size } => {
                                let px_size = match (compact, size >= 2) {
                                    (true, true) => 32.0,
                                    (true, false) => 18.0,
                                    (false, true) => 42.0,
                                    (false, false) => 22.0,
                                };
                                line_row =
                                    line_row.child(self.inline_emote(&url, px_size, &token, cx));
                            }
                        }
                    }
                    lines_col = lines_col.child(line_row);
                }
                container = container.child(lines_col);
            }
        }

        if !pictures.is_empty() {
            let all_urls = pictures.to_vec();
            let thumb_size = if compact { 68.0 } else { 92.0 };
            let mut pic_row = div().flex().flex_wrap().gap_2();
            for (pic_idx, pic_url) in pictures.iter().take(4).enumerate() {
                let thumb = self.picture(pic_url, thumb_size, thumb_size, cx);
                let urls_click = all_urls.clone();
                pic_row = pic_row.child(
                    div()
                        .id(SharedString::from(format!("comment-pic-{rpid}-{pic_idx}")))
                        .rounded_md()
                        .overflow_hidden()
                        .cursor_pointer()
                        .border_1()
                        .border_color(cx.theme().border)
                        .hover(|s| s.opacity(0.9))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.preview_images(urls_click.clone(), pic_idx, window, cx);
                        }))
                        .child(thumb),
                );
            }
            container = container.child(pic_row);
        }

        container.into_any_element()
    }

    fn load_comments(&mut self, stream: usize, index: usize, page: u32, cx: &mut Context<Self>) {
        let Some(services) = self.services.clone() else {
            return;
        };
        let Some(item) = self.feeds[stream].items.get(index).cloned() else {
            return;
        };
        let id = item.id_str.clone();
        let version = self.feeds[stream].version;
        let entry = self.feeds[stream].comments.entry(id.clone()).or_default();
        if entry.loading {
            return;
        }
        entry.loading = true;
        entry.error = None;
        self.run(
            async move {
                let oid = item
                    .comment_id
                    .context("此備援動態沒有有效評論區，請前往 B 站查看")?;
                services
                    .api
                    .get_comments(item.comment_type.unwrap_or(11), &oid, page, 20)
                    .await
            },
            cx,
            move |this, result, cx| {
                if this.feeds[stream].version != version {
                    return;
                }
                let entry = this.feeds[stream].comments.entry(id).or_default();
                entry.loading = false;
                match result {
                    Ok(comments) => {
                        if page == 1 {
                            entry.list = comments.list;
                        } else {
                            let mut seen: HashSet<_> = entry
                                .list
                                .iter()
                                .map(|comment| comment.rpid.clone())
                                .collect();
                            entry.list.extend(
                                comments
                                    .list
                                    .into_iter()
                                    .filter(|comment| seen.insert(comment.rpid.clone())),
                            );
                        }
                        entry.list.truncate(ui::COMMENTS_MAX_ITEMS);
                        entry.page = page;
                        entry.total = comments.total;
                        entry.more = comments.has_more && entry.list.len() < ui::COMMENTS_MAX_ITEMS;
                        entry.error = comments.error;
                    }
                    Err(error) => entry.error = Some(error.to_string()),
                }
                this.feeds[stream].list.splice(index..index + 1, 1);
                cx.notify();
            },
        );
        self.feeds[stream].list.splice(index..index + 1, 1);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn tokenize_comment_message_splits_known_emotes_and_preserves_unknown_brackets() {
        let mut emotes = BTreeMap::new();
        emotes.insert(
            "[doge]".into(),
            CommentEmote {
                url: "https://i0.hdslb.com/bfs/emote/doge.png".into(),
                size: 1,
            },
        );
        emotes.insert(
            "[大表情]".into(),
            CommentEmote {
                url: "https://i0.hdslb.com/bfs/emote/big.gif".into(),
                size: 2,
            },
        );

        let tokens = tokenize_comment_message("前綴[未知標籤]中間[doge][大表情]結尾", &emotes);
        assert_eq!(
            tokens,
            vec![
                CommentSegment::Text("前綴[未知標籤]中間".into()),
                CommentSegment::Emote {
                    token: "[doge]".into(),
                    url: "https://i0.hdslb.com/bfs/emote/doge.png".into(),
                    size: 1,
                },
                CommentSegment::Emote {
                    token: "[大表情]".into(),
                    url: "https://i0.hdslb.com/bfs/emote/big.gif".into(),
                    size: 2,
                },
                CommentSegment::Text("結尾".into()),
            ]
        );
    }
}
