mod common;
mod feed;
mod lifecycle;
mod live;
mod preview;
mod settings;
mod videos;

use crate::{
    assets::AppIcon,
    ui::{self, Tone},
};
use anyhow::{Context as _, Result, anyhow};
use bili_core::{
    APP_NAME,
    groups::{default_groups, group_members},
    images::{ImageKind, image_url},
    models::*,
    poller::{CoreEvent, NoticeKind},
    runtime::CoreRuntime,
    services::Services,
};
use gpui::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _,
    animation::cubic_bezier, button::*, shimmer::ShimmerText, tab::*, tooltip::Tooltip, *,
};
use gpui::{prelude::*, *};
use lru::LruCache;
use std::{
    collections::HashSet,
    future::Future,
    num::NonZeroUsize,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

gpui::actions!(
    bilinook,
    [
        FeedTab,
        VideosTab,
        LiveTab,
        SettingsTab,
        RefreshAll,
        LiveQuickFind,
        QuitApp
    ]
);

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum View {
    #[default]
    Feed,
    Videos,
    Live,
    Settings,
}

impl View {
    const ALL: [Self; 4] = [Self::Feed, Self::Videos, Self::Live, Self::Settings];

    pub fn label(self) -> &'static str {
        match self {
            Self::Feed => "動態",
            Self::Videos => "投稿影片",
            Self::Live => "直播工作台",
            Self::Settings => "設定與 UP 主管理",
        }
    }

    pub fn icon(self) -> AppIcon {
        match self {
            Self::Feed => AppIcon::Feed,
            Self::Videos => AppIcon::Video,
            Self::Live => AppIcon::Live,
            Self::Settings => AppIcon::Settings,
        }
    }

    pub fn shortcut_hint(self) -> &'static str {
        match self {
            Self::Feed => "動態 (Ctrl+1)",
            Self::Videos => "投稿影片 (Ctrl+2)",
            Self::Live => "直播狀態 (Ctrl+3)",
            Self::Settings => "設定與 UP 主管理 (Ctrl+4)",
        }
    }
}

enum Picture {
    Loading,
    Ready {
        image: Arc<Image>,
        loaded_at: Instant,
    },
    Failed(Instant),
}

#[derive(Clone)]
pub(super) struct ToastNotice {
    pub tone: Tone,
    pub title: String,
    pub detail: Option<String>,
    pub folder: Option<PathBuf>,
}

pub struct BiliApp {
    pub services: Option<Arc<Services>>,
    pub quitting: bool,
    pub force_quit: bool,
    pub io_stop: tokio_util::sync::CancellationToken,
    confirm_force_quit: bool,
    desktop: crate::desktop::Desktop,
    directory: PathBuf,
    shutdown_error: Option<String>,
    notifications: crate::notifications::Notifications,
    runtime: CoreRuntime,
    window: AnyWindowHandle,
    focus: FocusHandle,
    snapshot: Snapshot,
    read_ids: HashSet<String>,
    view: View,
    selected_group: String,
    loading: bool,
    refreshing: bool,
    error: Option<String>,
    notice: Option<ToastNotice>,
    notice_sequence: u64,
    toast_hovered: bool,
    toast_focused: bool,
    toast_focus: FocusHandle,
    toast_remaining: Duration,
    toast_resumed_at: Option<Instant>,
    toast_timer_generation: u64,
    view_epoch: u64,
    snapshot_version: u64,
    live_busy: bool,
    dynamic_busy: bool,
    pictures: LruCache<String, Picture>,
    pending_images: HashSet<String>,
    feeds: [feed::FeedState; 2],
    dual: bool,
    mobile_stream: usize,
    viewport_width: f32,
    videos: videos::VideoState,
    settings: settings::SettingsState,
    live: live::LiveState,
    ascii: Entity<crate::ascii::Clock>,
    sidebar_scroll: ScrollHandle,
}

pub(super) fn sidebar_pinned_ups(ups: &[UpUser]) -> Vec<UpUser> {
    let mut pinned: Vec<UpUser> = ups
        .iter()
        .filter(|up| up.is_pinned == Some(true))
        .cloned()
        .collect();
    pinned.sort_by(|a, b| {
        a.pin_order
            .unwrap_or(i64::MAX)
            .cmp(&b.pin_order.unwrap_or(i64::MAX))
            .then_with(|| a.mid.cmp(&b.mid))
    });
    pinned.truncate(8);
    pinned
}

impl BiliApp {
    pub fn new(
        runtime: CoreRuntime,
        desktop: crate::desktop::Desktop,
        directory: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let feeds = std::array::from_fn(|_| feed::FeedState::new(window, cx));
        let videos = videos::VideoState::new(window, cx);
        let settings = settings::SettingsState::new(window, cx);
        let live = live::LiveState::new(&directory, window, cx);
        let mut app = Self {
            services: None,
            quitting: false,
            force_quit: false,
            io_stop: tokio_util::sync::CancellationToken::new(),
            confirm_force_quit: false,
            desktop,
            directory,
            shutdown_error: None,
            notifications: crate::notifications::Notifications::new(),
            runtime,
            window: window.window_handle(),
            focus: cx.focus_handle(),
            snapshot: Snapshot::default(),
            read_ids: HashSet::new(),
            view: View::Feed,
            selected_group: "all".into(),
            loading: true,
            refreshing: false,
            error: None,
            notice: None,
            notice_sequence: 0,
            toast_hovered: false,
            toast_focused: false,
            toast_focus: cx.focus_handle(),
            toast_remaining: Duration::from_secs(6),
            toast_resumed_at: None,
            toast_timer_generation: 0,
            view_epoch: 0,
            snapshot_version: 0,
            live_busy: false,
            dynamic_busy: false,
            pictures: LruCache::new(NonZeroUsize::new(ui::IMAGE_CACHE_CAPACITY).unwrap()),
            pending_images: HashSet::new(),
            feeds,
            dual: false,
            mobile_stream: 0,
            viewport_width: 0.,
            videos,
            settings,
            live,
            ascii: crate::ascii::install(cx),
            sidebar_scroll: ScrollHandle::new(),
        };
        app.focus.focus(window, cx);
        app.subscribe_feeds(window, cx);
        app.subscribe_videos(window, cx);
        app.subscribe_settings(window, cx);
        app.subscribe_live(window, cx);
        app.install_desktop(window, cx);
        app.bootstrap(cx);
        app.error = app.desktop.warning.clone();
        app
    }

    pub fn bind_keys(cx: &mut App) {
        cx.bind_keys([
            KeyBinding::new("ctrl-q", QuitApp, Some(APP_NAME)),
            KeyBinding::new("ctrl-1", FeedTab, Some(APP_NAME)),
            KeyBinding::new("ctrl-2", VideosTab, Some(APP_NAME)),
            KeyBinding::new("ctrl-3", LiveTab, Some(APP_NAME)),
            KeyBinding::new("ctrl-4", SettingsTab, Some(APP_NAME)),
            KeyBinding::new("f5", RefreshAll, Some(APP_NAME)),
            KeyBinding::new("ctrl-k", LiveQuickFind, Some(APP_NAME)),
            KeyBinding::new("left", preview::PreviousImage, Some("ImagePreview")),
            KeyBinding::new("right", preview::NextImage, Some("ImagePreview")),
        ]);
    }

    pub(super) fn bootstrap(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        self.error = None;
        let directory = self.directory.clone();
        self.run(
            async move {
                let services = Services::open(directory).await?;
                services.live_watch.migrate_once(None, false).await?;
                services.live_watch.sync_links().await?;
                let snapshot = services.snapshot().await?;
                let live = services.live_watch.snapshot().await?;
                let events = services.poller.subscribe();
                Ok((services, snapshot, live, events))
            },
            cx,
            |this, result, cx| {
                this.loading = false;
                match result {
                    Ok((services, snapshot, live, mut events)) => {
                        let start = services.clone();
                        this.services = Some(services);
                        this.run(
                            async move {
                                start.start().await;
                                Ok(())
                            },
                            cx,
                            |this, result, cx| {
                                if let Err(error) = result {
                                    this.error = Some(error.to_string());
                                    cx.notify();
                                }
                            },
                        );
                        this.accepted_snapshot(snapshot, cx);
                        this.accepted_live(live, cx);
                        if !this.feeds[0].initialized {
                            this.load_feed(0, true, cx);
                        }
                        cx.spawn(async move |weak, cx| {
                            loop {
                                match events.recv().await {
                                    Ok(event) => {
                                        if weak
                                            .update(cx, |this, cx| this.on_event(event, cx))
                                            .is_err()
                                        {
                                            break;
                                        }
                                    }
                                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                        if weak
                                            .update(cx, |this, cx| this.reload_snapshot(cx))
                                            .is_err()
                                        {
                                            break;
                                        }
                                    }
                                    Err(_) => break,
                                }
                            }
                        })
                        .detach();
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            },
        );
    }

    fn run<T: Send + 'static>(
        &mut self,
        future: impl Future<Output = Result<T>> + Send + 'static,
        cx: &mut Context<Self>,
        done: impl FnOnce(&mut Self, Result<T>, &mut Context<Self>) + 'static,
    ) {
        if self.io_stop.is_cancelled() {
            return;
        }
        let token = self.io_stop.clone();
        match self.runtime.submit_cancellable(token.clone(), future) {
            Ok(result) => {
                cx.spawn(async move |weak, cx| {
                    let result = result
                        .recv()
                        .await
                        .unwrap_or_else(|_| Err(anyhow!("核心服務未回應")));
                    let _ = weak.update(cx, |this, cx| {
                        if !token.is_cancelled() {
                            done(this, result, cx);
                        }
                    });
                })
                .detach();
            }
            Err(error) => {
                self.settings.busy = false;
                self.error = Some(error.to_string());
                cx.notify();
            }
        }
    }

    fn reload_snapshot(&mut self, cx: &mut Context<Self>) {
        let Some(services) = self.services.clone() else {
            return;
        };
        self.snapshot_version += 1;
        let version = self.snapshot_version;
        self.run(
            async move {
                Ok((
                    services.snapshot().await?,
                    services.live_watch.snapshot().await?,
                ))
            },
            cx,
            move |this, result, cx| {
                if version != this.snapshot_version {
                    return;
                }
                match result {
                    Ok((snapshot, live)) => {
                        this.accepted_snapshot(snapshot, cx);
                        this.accepted_live(live, cx);
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            },
        );
    }

    fn on_event(&mut self, event: CoreEvent, cx: &mut Context<Self>) {
        if self.io_stop.is_cancelled() {
            return;
        }
        match event {
            CoreEvent::Updated => self.reload_snapshot(cx),
            CoreEvent::Notice(notice) => {
                self.native_notice(&notice, cx);
                if matches!(notice.kind, NoticeKind::Video | NoticeKind::Dynamic)
                    && let Some(services) = self.services.clone()
                {
                    self.run(
                        async move {
                            services.feed.invalidate().await;
                            Ok(())
                        },
                        cx,
                        |this, _, cx| {
                            if this.view == View::Feed {
                                this.load_feed(0, true, cx);
                                if this.dual {
                                    this.load_feed(1, true, cx);
                                }
                            }
                        },
                    );
                }
                self.show_toast(
                    ToastNotice {
                        tone: Tone::Info,
                        title: notice.title,
                        detail: Some(notice.body),
                        folder: None,
                    },
                    cx,
                );
            }
            CoreEvent::Error(message) => self.error = Some(message),
            CoreEvent::Polling { kind, active } => {
                if kind == "直播" {
                    self.live_busy = active;
                } else {
                    self.dynamic_busy = active;
                }
            }
        }
        cx.notify();
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(services) = self.services.clone() else {
            return;
        };
        if self.refreshing {
            return;
        }
        self.refreshing = true;
        let live_only = self.view == View::Live;
        self.error = None;
        let failed = self
            .pictures
            .iter()
            .filter(|(_, state)| matches!(state, Picture::Failed(_)))
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for key in failed {
            self.pictures.pop(&key);
        }
        self.run(
            async move {
                if live_only {
                    return services.poller.refresh_live().await;
                }
                services.feed.invalidate().await;
                services.poller.refresh().await
            },
            cx,
            |this, result, cx| {
                this.refreshing = false;
                match result {
                    Ok(()) => this.show_notice("更新檢查已完成".into(), cx),
                    Err(error) => this.error = Some(error.to_string()),
                }
                this.reload_snapshot(cx);
                if this.view == View::Feed {
                    this.load_feed(0, true, cx);
                    if this.dual {
                        this.load_feed(1, true, cx);
                    }
                }
                if this.view == View::Videos {
                    this.load_videos(true, cx);
                }
                cx.notify();
            },
        );
    }

    fn open_url(&mut self, url: String, cx: &mut Context<Self>) {
        self.run(
            async move {
                let parsed = url::Url::parse(&url).context("連結格式無效")?;
                anyhow::ensure!(
                    matches!(parsed.scheme(), "http" | "https"),
                    "此連結類型不支援"
                );
                open::that_detached(url).context("無法開啟系統預設瀏覽器")
            },
            cx,
            |this, result, cx| {
                if let Err(error) = result {
                    this.error = Some(error.to_string());
                    cx.notify();
                }
            },
        );
    }

    pub(super) fn set_dual_mode(&mut self, dual: bool, cx: &mut Context<Self>) {
        if self.dual == dual {
            return;
        }
        self.dual = dual;
        self.view_epoch = self.view_epoch.wrapping_add(1);
        for state in &self.feeds {
            let top = state.list.logical_scroll_top();
            state.list.reset(state.items.len() + 1);
            state.list.scroll_to(top);
        }
        if self.dual && !self.feeds[1].initialized {
            self.load_feed(1, true, cx);
        }
        cx.notify();
    }

    fn groups(&self) -> Vec<FollowGroup> {
        self.snapshot
            .settings
            .follow_groups
            .clone()
            .unwrap_or_else(default_groups)
    }

    fn scoped_ups(&self) -> Vec<UpUser> {
        group_members(&self.snapshot.ups, &self.groups(), &self.selected_group)
            .into_iter()
            .cloned()
            .collect()
    }

    fn avatar(&mut self, input: &str, size: f32, cx: &mut Context<Self>) -> AnyElement {
        self.picture_shaped(input, size, size, true, cx)
    }

    fn picture(
        &mut self,
        input: &str,
        width: f32,
        height: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.picture_shaped(input, width, height, false, cx)
    }

    fn inline_emote(
        &mut self,
        input: &str,
        size_px: f32,
        token_label: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Ok(url) = image_url(input) else {
            return div()
                .text_xs()
                .child(token_label.to_owned())
                .into_any_element();
        };
        if let Some(Picture::Ready { image, loaded_at }) = self.pictures.get(&url) {
            let tip = SharedString::from(token_label.to_owned());
            let elem = div()
                .id(SharedString::from(format!("emote-{url}-{token_label}")))
                .size(px(size_px))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                .child(
                    img(image.clone())
                        .size_full()
                        .object_fit(ObjectFit::Contain),
                );
            if loaded_at.elapsed() < Duration::from_millis(260) {
                return elem
                    .with_animation(
                        SharedString::from(format!("emote-fade-{url}")),
                        Animation::new(Duration::from_millis(200))
                            .with_easing(cubic_bezier(0.2, 0.0, 0.2, 1.0)),
                        |el, delta| el.opacity(0.15 + 0.85 * delta),
                    )
                    .into_any_element();
            }
            return elem.into_any_element();
        }
        self.request_image_download(&url, cx);
        div()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(token_label.to_owned())
            .into_any_element()
    }

    fn request_image_download(&mut self, url: &str, cx: &mut Context<Self>) {
        let retry = self.pictures.peek(url).is_none_or(|state| {
            matches!(state, Picture::Failed(time) if time.elapsed().as_secs() > ui::IMAGE_RETRY_SECS)
        });
        if retry
            && !self.pending_images.contains(url)
            && self.pending_images.len() < ui::IMAGE_MAX_CONCURRENT
            && let Some(services) = self.services.clone()
        {
            let owned = url.to_owned();
            let request_url = owned.clone();
            self.pictures.put(owned.clone(), Picture::Loading);
            self.pending_images.insert(owned.clone());
            self.run(
                async move { services.images.get(&request_url).await },
                cx,
                move |this, result, cx| {
                    this.pending_images.remove(&owned);
                    match result {
                        Ok(image) => {
                            this.pictures.put(
                                owned,
                                Picture::Ready {
                                    image: common::gui_image(&image),
                                    loaded_at: Instant::now(),
                                },
                            );
                        }
                        Err(error) => {
                            eprintln!("[圖片] {error}");
                            this.pictures.put(owned, Picture::Failed(Instant::now()));
                        }
                    }
                    cx.notify();
                },
            );
        }
    }

    fn picture_shaped(
        &mut self,
        input: &str,
        width: f32,
        height: f32,
        circle: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Ok(url) = image_url(input) else {
            return placeholder(width, height, circle, true, None, cx);
        };
        if let Some(Picture::Ready { image, loaded_at }) = self.pictures.get(&url) {
            let box_el = div()
                .w(px(width))
                .h(px(height))
                .flex_shrink_0()
                .when(circle, |d| d.rounded_full())
                .when(!circle, |d| d.rounded_lg())
                .overflow_hidden()
                .bg(cx.theme().secondary)
                .child(img(image.clone()).size_full().object_fit(ObjectFit::Cover));
            if loaded_at.elapsed() < Duration::from_millis(260) {
                return box_el
                    .with_animation(
                        SharedString::from(format!("pic-fade-{url}")),
                        Animation::new(Duration::from_millis(220))
                            .with_easing(cubic_bezier(0.2, 0.0, 0.2, 1.0)),
                        |el, delta| el.opacity(0.18 + 0.82 * delta),
                    )
                    .into_any_element();
            }
            return box_el.into_any_element();
        }
        let failed = self
            .pictures
            .peek(&url)
            .is_some_and(|state| matches!(state, Picture::Failed(_)));
        self.request_image_download(&url, cx);
        let waiting = (!failed && !circle && width >= 96. && height >= 48.)
            .then(|| SharedString::from(format!("picture-wait-{url}")));
        placeholder(width, height, circle, failed, waiting, cx)
    }

    fn group_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let groups = self.groups();
        let mut options = vec![("all".to_owned(), "全部".to_owned())];
        options.extend(groups.iter().map(|g| (g.id.clone(), g.name.clone())));
        options.push(("ungrouped".into(), "未分組".into()));
        div()
            .id("group-bar")
            .flex()
            .items_center()
            .overflow_x_scroll()
            .flex_shrink_0()
            .gap_1p5()
            .px_6()
            .py_2p5()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .children(options.into_iter().map(|(id, name)| {
                let count = group_members(&self.snapshot.ups, &groups, &id).len();
                let selected = self.selected_group == id;
                let label_text = format!("{name} {count}");
                let mut button = Button::new(SharedString::from(format!("group-{id}")))
                    .small()
                    .ghost()
                    .selected(selected)
                    .toggled(selected)
                    .accessibility_label(label_text.clone())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select_group(id.clone(), window, cx)
                    }));
                if selected {
                    button = button
                        .bg(ui::brand_soft_bg(cx))
                        .text_color(ui::brand_accent(cx));
                }
                button.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .child(
                            div()
                                .when(selected, |d| d.font_weight(FontWeight::SEMIBOLD))
                                .child(name),
                        )
                        .child(
                            div()
                                .px_1p5()
                                .rounded_full()
                                .bg(if selected {
                                    ui::brand_accent(cx)
                                } else {
                                    cx.theme().secondary
                                })
                                .text_xs()
                                .text_color(if selected {
                                    cx.theme().primary_foreground
                                } else {
                                    cx.theme().muted_foreground
                                })
                                .child(count.to_string()),
                        ),
                )
            }))
            .into_any_element()
    }

    #[inline(never)]
    fn render_sidebar(&mut self, active_live: usize, cx: &mut Context<Self>) -> AnyElement {
        let unread_feed_count = self.feeds[0]
            .items
            .iter()
            .filter(|item| !self.read_ids.contains(&item.id_str))
            .count();
        let settings_dirty = self.settings.is_dirty(&self.snapshot, cx);

        let sidebar = div()
            .text_sm()
            .min_h_0()
            .w(px(ui::SIDEBAR_WIDTH))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .p_3p5()
            .gap_1()
            .child(
                div()
                    .px_2()
                    .py_3()
                    .mb_1()
                    .flex()
                    .items_center()
                    .gap_2p5()
                    .child(
                        div()
                            .size(px(34.))
                            .rounded_lg()
                            .bg(ui::brand_accent(cx))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                Icon::new(AppIcon::BilibiliTv)
                                    .with_size(px(20.))
                                    .text_color(cx.theme().primary_foreground),
                            ),
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
                                    .text_color(cx.theme().sidebar_foreground)
                                    .truncate()
                                    .child(APP_NAME),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("動態 · 投稿 · 直播"),
                            )
                            .when(
                                self.snapshot.settings.appearance.ascii_animation_enabled,
                                |column| {
                                    column.child(
                                        div()
                                            .mt_1()
                                            .text_color(ui::brand_accent(cx).opacity(0.85))
                                            .child(crate::ascii::decoration(
                                                "sidebar-mascot",
                                                crate::ascii::Style::Mascot,
                                            )),
                                    )
                                },
                            ),
                    ),
            )
            .children(View::ALL.into_iter().enumerate().map(|(index, view)| {
                let badge = match view {
                    View::Feed if unread_feed_count > 0 => {
                        Some((unread_feed_count.to_string(), false))
                    }
                    View::Live if active_live > 0 => Some((format!("{active_live} 直播"), true)),
                    View::Settings if settings_dirty => Some(("未儲存".into(), true)),
                    _ => None,
                };
                ui::nav_item(
                    ("navigation", index),
                    view.icon(),
                    view.label(),
                    badge,
                    self.view == view,
                    cx,
                )
                .tooltip(view.shortcut_hint())
                .on_click(
                    cx.listener(move |this, _, window, cx| this.switch_view(view, window, cx)),
                )
            }));

        let mut collection = div().w_full().flex().flex_col().gap_1();
        if self.view == View::Live {
            collection = collection.child(self.render_live_sidebar(cx));
        }
        let pinned = if self.view == View::Live {
            Vec::new()
        } else {
            sidebar_pinned_ups(&self.snapshot.ups)
        };
        if !pinned.is_empty() {
            collection = collection.child(
                div()
                    .pt_4()
                    .pb_1()
                    .px_2p5()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(cx.theme().muted_foreground)
                    .child(Icon::new(AppIcon::Pin).with_size(px(12.)))
                    .child("置頂 UP 主"),
            );
        }
        for up in pinned {
            let mid = up.mid;
            let is_live = up
                .live_room
                .as_ref()
                .is_some_and(|room| room.live_status == 1);
            let avatar = self.avatar(&up.face, ui::AVATAR_XS, cx);
            let display_name = up.name.clone();
            let tooltip_text = match up.custom_note.as_deref().filter(|s| !s.is_empty()) {
                Some(note) => format!("{}（{}）", up.name, note),
                None => up.name.clone(),
            };
            let selected_up =
                self.view == View::Feed && !self.dual && self.feeds[0].mid == Some(mid);
            let mut item_row = div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap_2()
                .child(avatar)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_left()
                        .text_sm()
                        .child(display_name.clone()),
                );
            if is_live {
                item_row = item_row.child(ui::live_pulse_dot(
                    ("pinned-live-dot", mid),
                    7.0,
                    ui::brand_accent(cx),
                    cx,
                ));
            }
            collection = collection.child(
                Button::new(("pinned-up", mid))
                    .ghost()
                    .small()
                    .w_full()
                    .h(px(32.))
                    .selected(selected_up)
                    .accessibility_label(display_name)
                    .tooltip(tooltip_text)
                    .child(item_row)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.selected_group = "all".into();
                        this.feeds[0].group = "all".into();
                        this.feeds[0].mid = Some(mid);
                        this.sync_selectors(window, cx);
                        this.view = View::Feed;
                        this.live.stop();
                        this.load_feed(0, true, cx);
                        cx.notify();
                    })),
            );
        }

        let polling = self.live_busy || self.dynamic_busy;
        sidebar
            .child(crate::scroll::smooth_scroll_container(
                "sidebar-groups",
                cx.entity_id(),
                &self.sidebar_scroll,
                collection,
            ))
            .child(
                div()
                    .mt_2()
                    .flex_shrink_0()
                    .p_2p5()
                    .rounded_md()
                    .bg(cx.theme().muted)
                    .border_1()
                    .border_color(cx.theme().border)
                    .flex()
                    .gap_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .when(polling, |row| {
                                row.child(crate::ascii::loading("sidebar-sync"))
                            })
                            .child(if self.view == View::Live {
                                format!("{} 間監看", self.live.snapshot.rooms.len())
                            } else {
                                format!("{} 位關注", self.snapshot.ups.len())
                            })
                            .child("·")
                            .child(
                                div()
                                    .when(active_live > 0, |d| {
                                        d.text_color(ui::brand_accent(cx))
                                            .font_weight(FontWeight::SEMIBOLD)
                                    })
                                    .child(format!(
                                        "{active_live} {}開播",
                                        if self.view == View::Live {
                                            "間"
                                        } else {
                                            "位"
                                        }
                                    )),
                            )
                            .when(
                                active_live > 0 && self.view != View::Live && !polling,
                                |row| row.child(crate::ascii::broadcast("sidebar-live-broadcast")),
                            ),
                    ),
            )
            .child(
                Button::new("app-quit")
                    .flex_shrink_0()
                    .ghost()
                    .small()
                    .w_full()
                    .h(px(32.))
                    .icon(AppIcon::LogOut)
                    .accessibility_label("退出應用")
                    .tooltip("安全排空存檔並退出 (Ctrl+Q)")
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_left()
                            .text_color(cx.theme().muted_foreground)
                            .child("退出應用"),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.request_quit(cx))),
            )
            .into_any_element()
    }

    #[inline(never)]
    fn render_topbar(&self, active_live: usize, cx: &mut Context<Self>) -> AnyElement {
        let mut left = div()
            .flex()
            .items_center()
            .gap_2p5()
            .min_w_0()
            .child(
                Icon::new(self.view.icon())
                    .with_size(px(20.))
                    .text_color(ui::brand_accent(cx)),
            )
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::BOLD)
                    .child(self.view.label()),
            )
            .when(
                self.viewport_width >= 1100.
                    && self.snapshot.settings.appearance.ascii_animation_enabled,
                |left| {
                    let style = match self.view {
                        View::Feed => crate::ascii::Style::Flow,
                        View::Videos => crate::ascii::Style::Film,
                        View::Settings => crate::ascii::Style::Tuning,
                        View::Live => crate::ascii::Style::Radar,
                    };
                    left.child(
                        div()
                            .ml_2()
                            .text_color(ui::brand_accent(cx).opacity(0.8))
                            .child(crate::ascii::decoration("topbar-ascii", style)),
                    )
                },
            );

        if self.view == View::Live {
            let offline = self.scoped_ups().len().saturating_sub(active_live);
            left = left.child(
                div()
                    .ml_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("直播中 {active_live} 位 · 待機／離線 {offline} 位")),
            );
        }

        let mut right = div().flex().items_center().gap_2();
        if self.view == View::Feed {
            right = right.child(
                TabBar::new("feed-stream-layout-tabs")
                    .segmented()
                    .small()
                    .selected_index(usize::from(self.dual))
                    .child(
                        Tab::new()
                            .prefix(Icon::new(AppIcon::Rows).small())
                            .label("單欄")
                            .aria_label("切換單訊息流"),
                    )
                    .child(
                        Tab::new()
                            .prefix(Icon::new(AppIcon::Columns).small())
                            .label("雙欄")
                            .aria_label("切換雙訊息流"),
                    )
                    .on_click(cx.listener(|this, selected: &usize, _, cx| {
                        this.set_dual_mode(*selected == 1, cx);
                    })),
            );
        }
        right = right.child(
            Button::new("refresh-all")
                .outline()
                .small()
                .when(!self.refreshing, |button| button.icon(IconName::RefreshCw))
                .accessibility_label("立即刷新")
                .child(crate::ascii::button_label(
                    "refresh-wait",
                    if self.refreshing {
                        "更新中…"
                    } else {
                        "立即刷新"
                    },
                    self.refreshing,
                ))
                .tooltip("立即檢查所有追蹤狀態 (F5)")
                .disabled(self.refreshing || self.services.is_none())
                .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
        );

        div()
            .flex_shrink_0()
            .h(px(54.))
            .px_6()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(left)
            .child(right)
            .into_any_element()
    }

    #[inline(never)]
    fn render_toast(
        &mut self,
        toast: &ToastNotice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let focused = self.toast_focus.contains_focused(window, cx);
        if focused != self.toast_focused {
            self.toast_focused = focused;
            self.sync_toast_pause(cx);
        }
        let colors = ui::tone_colors(toast.tone, cx.theme().is_dark());
        let sequence = self.notice_sequence;
        let mut content = div().flex_1().min_w_0().flex().flex_col().gap_1().child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(cx.theme().popover_foreground)
                .child(ui::truncate_chars(&toast.title, ui::ERROR_MAX_CHARS)),
        );
        if let Some(detail) = &toast.detail {
            content = content.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(ui::truncate_chars(detail, ui::ERROR_MAX_CHARS)),
            );
        }
        if let Some(folder) = &toast.folder {
            let folder_path = folder.clone();
            content = content.child(
                div().pt_1().flex().child(
                    Button::new("toast-open-folder")
                        .xsmall()
                        .outline()
                        .icon(AppIcon::FolderOpen)
                        .label("開啟所在資料夾")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_local_folder(folder_path.clone(), cx);
                        })),
                ),
            );
        }

        if self
            .live
            .undo
            .as_ref()
            .is_some_and(|(sequence, _)| *sequence == self.notice_sequence)
        {
            content = content.child(
                Button::new("toast-restore-live")
                    .xsmall()
                    .outline()
                    .label("復原")
                    .on_click(cx.listener(|this, _, _, cx| this.restore_live_rooms(cx))),
            );
        }

        let remaining = self.toast_time_left();
        let ratio = (remaining.as_secs_f32() / 6.).clamp(0., 1.);
        let active = self.toast_resumed_at.is_some() && !remaining.is_zero();
        let color = colors.accent;
        let background = cx.theme().border;
        let countdown = canvas(
            move |_, window, _| {
                if active {
                    window.request_animation_frame();
                }
            },
            move |bounds, _, window, _| {
                window.paint_quad(fill(bounds, background));
                window.paint_quad(fill(
                    Bounds {
                        origin: bounds.origin,
                        size: size(bounds.size.width * ratio, bounds.size.height),
                    },
                    color,
                ));
            },
        )
        .h(px(2.))
        .w_full();

        div()
            .id("app-toast")
            .track_focus(&self.toast_focus)
            .absolute()
            .right_5()
            .bottom_10()
            .w(px(ui::TOAST_WIDTH))
            .rounded_lg()
            .overflow_hidden()
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .shadow_lg()
            .flex()
            .flex_col()
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.toast_hovered = *hovered;
                this.sync_toast_pause(cx);
                cx.notify();
            }))
            .child(
                div()
                    .flex()
                    .child(div().w(px(4.)).flex_shrink_0().bg(colors.accent))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .p_3p5()
                            .flex()
                            .items_start()
                            .gap_2p5()
                            .child(
                                div().pt_0p5().child(
                                    Icon::new(ui::tone_icon(toast.tone))
                                        .with_size(px(16.))
                                        .text_color(colors.accent),
                                ),
                            )
                            .child(content)
                            .child(
                                Button::new("dismiss-toast")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Close)
                                    .accessibility_label("關閉提示")
                                    .tooltip("關閉")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.notice = None;
                                        this.toast_hovered = false;
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .child(countdown)
            .with_animation(
                ("toast-slide-in", sequence),
                Animation::new(Duration::from_millis(220))
                    .with_easing(cubic_bezier(0.16, 1.0, 0.3, 1.0)),
                |el, delta| el.opacity(delta).right(px(20.0 - (1.0 - delta) * 24.0)),
            )
            .into_any_element()
    }
}

fn placeholder(
    width: f32,
    height: f32,
    circle: bool,
    failed: bool,
    waiting: Option<SharedString>,
    cx: &App,
) -> AnyElement {
    let (bg, fg, border) = ui::placeholder_colors(cx);
    let child: AnyElement = if width < 64. {
        Icon::new(if failed {
            IconName::CircleAlert
        } else {
            IconName::User
        })
        .with_size(px((width * 0.44).clamp(12., 18.)))
        .text_color(fg)
        .into_any_element()
    } else {
        div()
            .flex()
            .items_center()
            .gap_1p5()
            .text_xs()
            .text_color(fg)
            .child(if let Some(id) = waiting {
                crate::ascii::waiting(id)
            } else {
                Icon::new(if failed {
                    IconName::TriangleAlert
                } else {
                    IconName::Inbox
                })
                .with_size(px(14.))
                .text_color(fg)
                .into_any_element()
            })
            .child(if failed {
                "圖片無法載入"
            } else {
                "載入圖片"
            })
            .into_any_element()
    };

    div()
        .w(px(width))
        .h(px(height))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .when(circle, |d| d.rounded_full())
        .when(!circle, |d| d.rounded_lg())
        .bg(bg)
        .border_1()
        .border_color(border)
        .child(child)
        .into_any_element()
}

impl Render for BiliApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = f32::from(window.viewport_size().width);
        window.set_rem_size(px(16. * self.snapshot.settings.appearance.font_scale));
        if (width - self.viewport_width).abs() > 1. {
            self.viewport_width = width;
            for state in &self.feeds {
                let top = state.list.logical_scroll_top();
                state.list.reset(state.items.len() + 1);
                state.list.scroll_to(top);
            }
            self.relayout_videos(cx);
            self.live.relayout();
        }
        let active_live = self
            .snapshot
            .ups
            .iter()
            .filter(|up| {
                up.live_room
                    .as_ref()
                    .is_some_and(|room| room.live_status == 1)
            })
            .count();
        let active_live = if self.view == View::Live {
            self.live
                .snapshot
                .rooms
                .iter()
                .filter(|r| r.info.status == bili_core::live_watch::RoomStatus::Live)
                .count()
        } else {
            active_live
        };

        let sidebar = self.render_sidebar(active_live, cx);
        let topbar = self.render_topbar(active_live, cx);

        let raw_content = if self.loading {
            ui::feed_skeleton_list(cx)
        } else {
            match self.view {
                View::Feed => self.render_feed_workspace(window, cx),
                View::Videos => self.render_videos(window, cx),
                View::Live => self.render_live(window, cx),
                View::Settings => self.render_settings(window, cx),
            }
        };
        let content = if cx.reduce_motion() || self.live.keyboard {
            div().size_full().child(raw_content).into_any_element()
        } else {
            ui::enter_transition(
                div().size_full().child(raw_content),
                ("workspace-enter", self.view_epoch),
            )
        };
        let content = div().relative().size_full().child(content).when(
            self.view_epoch > 0
                && self.snapshot.settings.appearance.ascii_animation_enabled
                && !cx.reduce_motion()
                && !self.live.keyboard,
            |container| {
                container.child(crate::ascii::panel_switch(
                    ("ascii-workspace-switch", self.view_epoch),
                    ui::brand_accent(cx),
                    cx.theme().background,
                ))
            },
        );

        let mut body = div().flex().flex_col().flex_1().min_w_0().size_full();
        if self.view != View::Live {
            body = body.child(topbar);
        }

        if self.view != View::Settings
            && self.view != View::Live
            && !(self.view == View::Feed && self.dual)
        {
            body = body.child(self.group_bar(cx));
        }

        if let Some(error) = self.error.clone() {
            let actions = vec![
                Button::new("retry-error")
                    .xsmall()
                    .outline()
                    .label("重試")
                    .on_click(cx.listener(|this, _, _, cx| {
                        if this.services.is_none() {
                            this.bootstrap(cx);
                        } else {
                            this.refresh(cx);
                        }
                    }))
                    .into_any_element(),
                Button::new("dismiss-error")
                    .xsmall()
                    .ghost()
                    .icon(IconName::Close)
                    .accessibility_label("關閉錯誤提示")
                    .tooltip("關閉提示")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.error = None;
                        cx.notify();
                    }))
                    .into_any_element(),
            ];
            body = body.child(div().flex_shrink_0().px_6().pt_3().child(ui::notice_banner(
                Tone::Danger,
                &error,
                actions,
                cx,
            )));
        }

        let status_left = if self.loading {
            "正在讀取本機追蹤資料…"
        } else if self.live_busy && self.dynamic_busy {
            "正在背景同步直播與動態…"
        } else if self.live_busy {
            "正在背景檢查直播狀態…"
        } else if self.dynamic_busy {
            "正在背景檢查最新動態…"
        } else if self.services.is_some() {
            "背景追蹤已就緒"
        } else {
            "資料服務未就緒"
        };

        let status_busy = self.loading || self.live_busy || self.dynamic_busy;
        let status_text_el: AnyElement = if status_busy {
            ShimmerText::new(status_left).into_any_element()
        } else {
            div().child(status_left).into_any_element()
        };

        body = body
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .child(content),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .px_6()
                    .py_1p5()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background)
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(if status_busy {
                                crate::ascii::loading("status-sync")
                            } else {
                                div()
                                    .size(px(6.))
                                    .rounded_full()
                                    .bg(if self.services.is_some() {
                                        ui::tone_colors(Tone::Success, cx.theme().is_dark()).accent
                                    } else {
                                        ui::tone_colors(Tone::Warning, cx.theme().is_dark()).accent
                                    })
                                    .into_any_element()
                            })
                            .child(status_text_el)
                            .when(
                                self.viewport_width >= 1100.
                                    && self.snapshot.settings.appearance.ascii_animation_enabled,
                                |row| {
                                    row.child(
                                        div().text_color(ui::brand_accent(cx).opacity(0.75)).child(
                                            crate::ascii::decoration(
                                                "status-sparks",
                                                crate::ascii::Style::Sparks,
                                            ),
                                        ),
                                    )
                                },
                            ),
                    )
                    .child("Ctrl+1–4 分頁 · F5 刷新 · Ctrl+Q 安全退出"),
            );

        let mut root =
            div()
                .key_context(APP_NAME)
                .track_focus(&self.focus)
                .relative()
                .flex()
                .size_full()
                .bg(cx.theme().background)
                .text_color(cx.theme().foreground)
                .font_family(ui::appearance_font(
                    &self.snapshot.settings.appearance.font_family,
                ))
                .capture_any_mouse_down(cx.listener(|this, _, _, _| this.live.keyboard = false))
                .capture_key_down(cx.listener(|this, _, _, _| this.live.keyboard = true))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, _| this.live.keyboard = false),
                )
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    this.live.keyboard = true;
                    if this.view == View::Live
                        && this.live.panels.is_empty()
                        && !this.live.search.focus_handle(cx).is_focused(window)
                        && !this.live.add.focus_handle(cx).is_focused(window)
                    {
                        match event.keystroke.key.as_str() {
                            "r" => this.refresh(cx),
                            "s" => this.live.search.focus_handle(cx).focus(window, cx),
                            "a" => this.live.add.focus_handle(cx).focus(window, cx),
                            "?" => this.open_live_panel(live::PanelKind::Shortcuts, window, cx),
                            "escape" => {
                                if this.live.fullscreen.is_some() {
                                    this.live.host.borrow().exit_fullscreen();
                                    this.live.fullscreen = None;
                                    window.toggle_fullscreen();
                                } else {
                                    this.live.stop();
                                }
                                cx.notify();
                            }
                            _ => {}
                        }
                    }
                }))
                .on_action(cx.listener(|this, _: &FeedTab, window, cx| {
                    this.switch_view(View::Feed, window, cx)
                }))
                .on_action(cx.listener(|this, _: &VideosTab, window, cx| {
                    this.switch_view(View::Videos, window, cx)
                }))
                .on_action(cx.listener(|this, _: &LiveTab, window, cx| {
                    this.switch_view(View::Live, window, cx)
                }))
                .on_action(cx.listener(|this, _: &SettingsTab, window, cx| {
                    this.switch_view(View::Settings, window, cx)
                }))
                .on_action(cx.listener(|this, _: &RefreshAll, _, cx| this.refresh(cx)))
                .on_action(cx.listener(|this, _: &LiveQuickFind, window, cx| {
                    if this.live.panels.is_empty() {
                        this.open_live_panel(live::PanelKind::QuickFind, window, cx);
                    }
                }))
                .on_action(cx.listener(|this, _: &QuitApp, _, cx| this.request_quit(cx)))
                .child(crate::motion::chrome(
                    sidebar,
                    self.snapshot.settings.appearance.font_scale,
                ))
                .child(body);

        if let Some(id) = self.live.fullscreen {
            let size = window.viewport_size();
            root = root.child(div().absolute().inset_0().bg(gpui::black()).child(
                self.live_player_surface(id, f32::from(size.width), f32::from(size.height), cx),
            ));
        }
        if let Some(toast) = self.notice.clone() {
            root = root.child(self.render_toast(&toast, window, cx));
        }
        if let Some(panel) = self.render_live_drawer(window, cx) {
            root = root.child(panel);
        }
        if self.quitting || self.shutdown_error.is_some() {
            root = root.child(self.shutdown_overlay(cx));
        }
        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn sidebar_pinned_ups_sorts_by_pin_order_then_mid() {
        let ups = vec![
            UpUser {
                mid: 30,
                name: "C".into(),
                is_pinned: Some(true),
                pin_order: Some(5),
                ..Default::default()
            },
            UpUser {
                mid: 10,
                name: "Unpinned".into(),
                is_pinned: Some(false),
                pin_order: Some(0),
                ..Default::default()
            },
            UpUser {
                mid: 20,
                name: "A".into(),
                is_pinned: Some(true),
                pin_order: Some(1),
                ..Default::default()
            },
            UpUser {
                mid: 15,
                name: "B".into(),
                is_pinned: Some(true),
                pin_order: Some(1),
                ..Default::default()
            },
        ];
        let ordered = sidebar_pinned_ups(&ups);
        let mids: Vec<u64> = ordered.into_iter().map(|u| u.mid).collect();
        assert_eq!(mids, vec![15, 20, 30]);
    }
}
