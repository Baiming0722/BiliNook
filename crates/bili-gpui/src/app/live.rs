mod panels;
mod stats;
use super::common::{input, value};
use super::*;
use crate::{
    motion::{self, MotionValue},
    player::{PlayerEvent, PlayerHost},
};
use bili_core::live_watch::*;
use gpui::component::{
    input::{Input, InputEvent, InputState},
    tag::Tag,
};
pub(super) use panels::{LivePanel, PanelKind};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

pub(super) struct LiveState {
    pub snapshot: LiveWatchSnapshot,
    pub search: Entity<InputState>,
    pub add: Entity<InputState>,
    pub filter: RoomFilter,
    pub visible: Vec<DisplayRoom>,
    list: ListState,
    rows: Vec<Vec<u64>>,
    pub columns: usize,
    pub previews: Vec<u64>,
    pub multi: bool,
    pub watch_ids: Vec<u64>,
    pub layout: MultiViewLayout,
    pub limit: usize,
    pub focused: Option<u64>,
    pub danmaku: bool,
    pub host: Rc<RefCell<PlayerHost>>,
    pub shots: BTreeMap<u64, Arc<Image>>,
    pub loaded: HashSet<u64>,
    pub panels: Vec<Entity<LivePanel>>,
    pub panel_motion: MotionValue,
    pub closing: bool,
    pub previous_focus: Vec<Option<FocusHandle>>,
    pub busy: bool,
    pub error: Option<String>,
    pub undo: Option<(u64, Vec<(StoredRoom, TrackingSource)>)>,
    pub fullscreen: Option<u64>,
    pub pending_focus: Option<u64>,
    pub keyboard: bool,
    pub numbers: BTreeMap<(u64, &'static str), MotionValue>,
    pub presses: motion::PressCache,
    pub statistics: bool,
    pub highlighted: Option<u64>,
    #[cfg(debug_assertions)]
    pub next_media_trace: Instant,
    pub stats_scroll: ScrollHandle,
    pub multi_scroll: ScrollHandle,
    pub reflow: motion::ReflowCache,
    pub layout_revision: u64,
    pub render_frame: u64,
}
impl LiveState {
    pub fn new(directory: &std::path::Path, window: &mut Window, cx: &mut App) -> Self {
        Self {
            stats_scroll: ScrollHandle::new(),
            multi_scroll: ScrollHandle::new(),
            reflow: Default::default(),
            layout_revision: 0,
            render_frame: 0,
            snapshot: LiveWatchSnapshot::default(),
            search: input("", "搜尋房號、主播、標題或備註", window, cx),
            add: input("", "房號或直播網址", window, cx),
            filter: RoomFilter::default(),
            visible: vec![],
            list: ListState::new(0, ListAlignment::Top, px(320.)),
            rows: vec![],
            columns: 0,
            previews: vec![],
            multi: false,
            watch_ids: vec![],
            layout: MultiViewLayout::Auto,
            limit: 4,
            focused: None,
            danmaku: true,
            host: Rc::new(RefCell::new(PlayerHost::new(
                directory.join("cache/webview2"),
            ))),
            shots: BTreeMap::new(),
            loaded: HashSet::new(),
            panels: vec![],
            panel_motion: MotionValue::settled(1.),
            closing: false,
            previous_focus: vec![],
            busy: false,
            error: None,
            undo: None,
            fullscreen: None,
            pending_focus: None,
            keyboard: false,
            numbers: BTreeMap::new(),
            presses: Default::default(),
            statistics: false,
            highlighted: None,
            #[cfg(debug_assertions)]
            next_media_trace: Instant::now() + Duration::from_secs(5),
        }
    }
    pub fn relayout(&self) {
        self.list.reset(self.rows.len());
    }
    pub fn stop(&mut self) {
        self.host.borrow_mut().clear();
        self.previews.clear();
        self.watch_ids.clear();
        self.multi = false;
        self.loaded.clear();
        self.shots.clear();
        self.fullscreen = None;
    }
}
struct RoomDrag {
    room_id: u64,
    name: String,
}
impl Render for RoomDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .p_3()
            .rounded_lg()
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().primary)
            .child(self.name.clone())
    }
}

impl BiliApp {
    pub(super) fn stop_live_players(&mut self, window: &mut Window) {
        if self.live.fullscreen.take().is_some() {
            self.live.host.borrow().exit_fullscreen();
            window.toggle_fullscreen();
        }
        self.live.stop();
    }
    pub(super) fn subscribe_live(&mut self, window: &Window, cx: &mut Context<Self>) {
        cx.subscribe_in(
            &self.live.search,
            window,
            |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.live.filter.query = value(&this.live.search, cx);
                    this.filter_live(true, cx);
                }
            },
        )
        .detach();
        cx.subscribe_in(
            &self.live.add,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.add_live_room(window, cx);
                }
            },
        )
        .detach();
    }
    pub(super) fn accepted_live(&mut self, snapshot: LiveWatchSnapshot, cx: &mut Context<Self>) {
        if snapshot.data.native.revision < self.live.snapshot.data.native.revision {
            return;
        }
        self.live.snapshot = snapshot;
        if self
            .live
            .filter
            .group
            .as_ref()
            .is_some_and(|id| !self.live.snapshot.data.groups.iter().any(|g| &g.id == id))
        {
            self.live.filter.group = None;
        }
        let ids: HashSet<u64> = self
            .live
            .snapshot
            .rooms
            .iter()
            .map(|r| r.saved.room_id)
            .collect();
        self.live
            .numbers
            .retain(|(id, key), _| !matches!(*key, "online" | "attention") || ids.contains(id));
        self.live
            .reflow
            .borrow_mut()
            .retain(|id, _| ids.contains(id));
        self.filter_live(false, cx);
    }
    pub(super) fn filter_live(&mut self, reset: bool, cx: &mut Context<Self>) {
        let next = filter_rooms(&self.live.snapshot.rooms, &self.live.filter);
        if self
            .live
            .visible
            .iter()
            .map(|r| r.saved.room_id)
            .ne(next.iter().map(|r| r.saved.room_id))
        {
            self.live.layout_revision = self.live.layout_revision.wrapping_add(1);
        }
        self.live.visible = next;
        if reset {
            self.live.list.scroll_to(Default::default());
        }
        let ids: Vec<_> = self.live.visible.iter().map(|r| r.saved.room_id).collect();
        if !self.live.multi {
            self.live.previews.retain(|id| ids.contains(id));
            self.live.host.borrow_mut().retain(&self.live.previews);
        }
        cx.notify();
    }
    pub(super) fn live_change<T: Send + 'static>(
        &mut self,
        change: impl FnOnce(&mut LiveWatchData) -> Result<T> + Send + 'static,
        cx: &mut Context<Self>,
        done: impl FnOnce(&mut Self, T, &mut Context<Self>) + 'static,
    ) {
        let Some(services) = self.services.clone() else {
            return;
        };
        self.run(
            async move {
                let result = services.store.update_live(change).await?;
                services.poller.notify_update();
                Ok((result, services.live_watch.snapshot().await?))
            },
            cx,
            move |this, result, cx| match result {
                Ok((result, snapshot)) => {
                    this.live.error = None;
                    this.accepted_live(snapshot, cx);
                    done(this, result, cx);
                }
                Err(error) => {
                    this.live.error = Some(error.to_string());
                    cx.notify();
                }
            },
        );
    }
    fn add_live_room(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.live.busy {
            return;
        }
        let Some(services) = self.services.clone() else {
            return;
        };
        let text = value(&self.live.add, cx);
        if text.trim().is_empty() {
            return;
        }
        self.live.busy = true;
        self.live.error = None;
        self.run(
            async move {
                let result = services.live_watch.add_input(text).await?;
                let snapshot = services.live_watch.snapshot().await?;
                services.poller.notify_update();
                Ok((result, snapshot))
            },
            cx,
            |this, result, cx| {
                this.live.busy = false;
                match result {
                    Ok(((id, added), snapshot)) => {
                        this.accepted_live(snapshot, cx);
                        this.live.highlighted = Some(id);
                        let handle = this.window;
                        let _ = handle.update(cx, |_, window, cx| {
                            this.live
                                .add
                                .update(cx, |s, cx| s.set_value("", window, cx))
                        });
                        this.show_notice(
                            if added {
                                "已新增直播監看"
                            } else {
                                "此房間已在監看名單"
                            }
                            .into(),
                            cx,
                        );
                    }
                    Err(error) => this.live.error = Some(error.to_string()),
                }
                cx.notify();
            },
        );
    }
    fn request_live_player(
        &mut self,
        id: u64,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let start = self
            .live
            .host
            .borrow_mut()
            .request(id, self.live.danmaku, window)?;
        if start {
            let host = self.live.host.clone();
            // Wry pumps Windows messages while creating WebView2. Never hold a GPUI
            // App/entity borrow or the host RefCell across that message pump.
            cx.spawn(async move |weak, cx| {
                loop {
                    let job = { host.borrow_mut().next_job() };
                    let Some(job) = job else { break };
                    let result = job.run();
                    let failure = { host.borrow_mut().finish_job(result) };
                    if weak
                        .update(cx, |this, cx| {
                            if let Some((id, error)) = failure {
                                this.live.previews.retain(|item| *item != id);
                                this.live.watch_ids.retain(|item| *item != id);
                                this.live.loaded.remove(&id);
                                this.live.shots.remove(&id);
                                this.live.error = Some(format!("房間 {id}：{error}"));
                            }
                            cx.notify();
                        })
                        .is_err()
                    {
                        host.borrow_mut().clear();
                        break;
                    }
                }
            })
            .detach();
        }
        Ok(())
    }

    pub(super) fn toggle_live_preview(
        &mut self,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.live.snapshot.data.native.ui_sound_enabled && !self.live.keyboard {
            crate::audio::play(false, self.live.snapshot.data.settings.sound_volume);
        }
        if let Some(index) = self.live.previews.iter().position(|i| *i == id) {
            self.live.previews.remove(index);
            self.live.loaded.remove(&id);
            self.live.shots.remove(&id);
            self.live.host.borrow_mut().remove(id);
        } else {
            if self.live.previews.len() >= 9 {
                let old = self.live.previews.remove(0);
                self.live.loaded.remove(&old);
                self.live.shots.remove(&old);
                self.live.host.borrow_mut().remove(old);
            }
            match self.request_live_player(id, window, cx) {
                Ok(()) => self.live.previews.push(id),
                Err(error) => self.live.error = Some(format!("{error:#}")),
            }
        }
        cx.notify();
    }
    pub(super) fn start_multi(
        &mut self,
        ids: Vec<u64>,
        layout: MultiViewLayout,
        limit: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.live.snapshot.data.native.ui_sound_enabled && !self.live.keyboard {
            crate::audio::play(false, self.live.snapshot.data.settings.sound_volume);
        }
        self.stop_live_players(window);
        self.live.multi = true;
        self.live.layout = layout;
        self.live.limit = limit;
        let rooms: BTreeMap<_, _> = self
            .live
            .snapshot
            .rooms
            .iter()
            .map(|r| (r.saved.room_id, r.info.status))
            .collect();
        for id in ids
            .into_iter()
            .filter(|id| rooms.get(id) == Some(&RoomStatus::Live))
            .take(limit)
        {
            match self.request_live_player(id, window, cx) {
                Ok(()) => self.live.watch_ids.push(id),
                Err(error) => {
                    self.live.error = Some(format!("{error:#}"));
                    break;
                }
            }
        }
        self.live.focused = self.live.watch_ids.first().copied();
        cx.notify();
    }
    fn toggle_multi_room(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.live.watch_ids.iter().position(|i| *i == id) {
            self.live.watch_ids.remove(index);
            self.live.loaded.remove(&id);
            self.live.shots.remove(&id);
            self.live.host.borrow_mut().remove(id);
        } else if self.live.watch_ids.len() < self.live.limit {
            match self.request_live_player(id, window, cx) {
                Ok(()) => self.live.watch_ids.push(id),
                Err(error) => self.live.error = Some(format!("{error:#}")),
            }
        }
        if self
            .live
            .focused
            .is_none_or(|id| !self.live.watch_ids.contains(&id))
        {
            self.live.focused = self.live.watch_ids.first().copied();
        }
        cx.notify();
    }
    pub(super) fn poll_live_players(&mut self, cx: &mut Context<Self>) {
        #[cfg(debug_assertions)]
        if std::env::var_os("BILI_PLAYER_TRACE").is_some()
            && Instant::now() >= self.live.next_media_trace
        {
            self.live.host.borrow().trace_media();
            self.live.next_media_trace = Instant::now() + Duration::from_secs(5);
        }
        let events = self.live.host.borrow().events().collect::<Vec<_>>();
        for event in events {
            let owner = match &event {
                PlayerEvent::Loaded(id)
                | PlayerEvent::Error(id, _)
                | PlayerEvent::Snapshot(id, _)
                | PlayerEvent::Fullscreen(id, _) => Some(*id),
                _ => None,
            };
            if owner.is_some_and(|id| !self.live.host.borrow().contains(id)) {
                continue;
            }
            match event {
                PlayerEvent::Loaded(id) => {
                    self.live.loaded.insert(id);
                }
                PlayerEvent::Error(id, message) => {
                    self.live.host.borrow_mut().failed_room = Some(id);
                    self.live.error = Some(format!("房間 {id}：{message}"))
                }
                PlayerEvent::Snapshot(id, bytes) => {
                    self.live
                        .shots
                        .insert(id, Arc::new(Image::from_bytes(ImageFormat::Png, bytes)));
                }
                PlayerEvent::Open(url) => self.open_url(url, cx),
                PlayerEvent::Fullscreen(id, active) => {
                    let handle = self.window;
                    let _ = handle.update(cx, |_, window, _| {
                        if active != self.live.fullscreen.is_some() {
                            window.toggle_fullscreen();
                            self.live.fullscreen = active.then_some(id);
                        }
                    });
                }
                PlayerEvent::Shortcut(key) => {
                    let handle = self.window;
                    let entity = cx.entity().downgrade();
                    cx.defer(move |cx| {
                        let _ = handle.update(cx, |_, window, cx| {
                            let _ = entity.update(cx, |this, cx| match key.as_str() {
                                "ctrl-1" => this.switch_view(View::Feed, window, cx),
                                "ctrl-2" => this.switch_view(View::Videos, window, cx),
                                "ctrl-3" => this.switch_view(View::Live, window, cx),
                                "ctrl-4" => this.switch_view(View::Settings, window, cx),
                                "ctrl-k" => this.open_live_panel(PanelKind::QuickFind, window, cx),
                                "ctrl-q" => this.request_quit(cx),
                                "escape" if this.live.fullscreen.is_some() => {
                                    this.live.host.borrow().exit_fullscreen();
                                    this.live.fullscreen = None;
                                    window.toggle_fullscreen();
                                    cx.notify();
                                }
                                _ => {}
                            });
                        });
                    });
                }
            }
            cx.notify();
        }
    }
    pub(super) fn focus_live_room(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.switch_view(View::Live, window, cx);
        self.live.statistics = false;
        self.live.filter = RoomFilter::default();
        self.live
            .search
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.filter_live(true, cx);
        self.live.highlighted = Some(id);
        if let Some(index) = self.live.visible.iter().position(|r| r.saved.room_id == id) {
            let mut offset = self.live.list.logical_scroll_top();
            offset.item_ix = index / self.live.columns.max(1);
            offset.offset_in_item = px(0.);
            self.live.list.scroll_to(offset);
        }
        cx.notify();
    }
    pub(super) fn remove_live_rooms(&mut self, ids: Vec<u64>, cx: &mut Context<Self>) {
        for id in &ids {
            self.live.host.borrow_mut().remove(*id);
        }
        self.live_change(
            move |data| Ok(data.remove_rooms(&ids)),
            cx,
            |this, removed, cx| {
                this.show_notice(format!("已移除 {} 間直播監看", removed.len()), cx);
                this.live.undo = Some((this.notice_sequence, removed));
            },
        );
    }
    pub(super) fn restore_live_rooms(&mut self, cx: &mut Context<Self>) {
        if let Some((_, removed)) = self.live.undo.take() {
            self.live_change(
                move |d| d.restore_rooms(removed),
                cx,
                |this, _, cx| this.show_notice("已復原直播監看".into(), cx),
            );
        }
    }
    fn move_live_room(&mut self, id: u64, target: u64, cx: &mut Context<Self>) {
        self.live_change(
            move |data| {
                let source = data
                    .rooms
                    .iter()
                    .find(|r| r.room_id == id)
                    .cloned()
                    .ok_or_else(|| anyhow!("房間不存在"))?;
                let destination = data
                    .rooms
                    .iter()
                    .find(|r| r.room_id == target)
                    .ok_or_else(|| anyhow!("目標不存在"))?;
                anyhow::ensure!(
                    source.group_id == destination.group_id && source.pinned == destination.pinned,
                    "只能調整相同分組及置頂狀態的房間"
                );
                let mut rooms: Vec<_> = data
                    .rooms
                    .iter()
                    .filter(|r| r.group_id == source.group_id && r.pinned == source.pinned)
                    .collect();
                rooms.sort_by_key(|r| r.order);
                let mut ids: Vec<_> = rooms
                    .iter()
                    .map(|r| r.room_id)
                    .filter(|i| *i != id)
                    .collect();
                let index = ids.iter().position(|i| *i == target).unwrap_or(ids.len());
                ids.insert(index, id);
                data.reorder_rooms(ids, source.group_id, source.pinned)
            },
            cx,
            |_, _, _| {},
        );
    }
    pub(super) fn render_live(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.live.host.borrow_mut().fullscreen = self.live.fullscreen;
        self.live.host.borrow_mut().begin_frame();
        self.live.render_frame = self.live.render_frame.wrapping_add(1);
        let compact = self.live.snapshot.data.settings.room_view == "list";
        let available =
            (f32::from(window.viewport_size().width) - ui::SIDEBAR_WIDTH - 48.).max(200.);
        let columns = if compact {
            1
        } else {
            ((available + 12.)
                / (300. * self.snapshot.settings.appearance.card_scale).clamp(200., 600.))
            .floor()
            .clamp(1., 5.) as usize
        };
        let rows: Vec<Vec<_>> = self
            .live
            .visible
            .iter()
            .map(|r| r.saved.room_id)
            .collect::<Vec<_>>()
            .chunks(columns)
            .map(|v| v.to_vec())
            .collect();
        if rows != self.live.rows || columns != self.live.columns {
            self.live.rows = rows;
            self.live.columns = columns;
            self.live.list.reset(self.live.rows.len());
        }
        let live_count = self
            .live
            .snapshot
            .rooms
            .iter()
            .filter(|room| room.info.status == RoomStatus::Live)
            .count();
        let mut controls = div()
            .text_sm()
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar);
        controls = controls.child(
            div()
                .flex()
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .child(
                                    div()
                                        .text_lg()
                                        .font_weight(FontWeight::BOLD)
                                        .child("直播工作台"),
                                )
                                .when(
                                    self.snapshot.settings.appearance.ascii_animation_enabled,
                                    |row| {
                                        row.child(
                                            div()
                                                .text_color(ui::brand_accent(cx).opacity(0.8))
                                                .child(crate::ascii::decoration(
                                                    "live-radar",
                                                    crate::ascii::Style::Radar,
                                                )),
                                        )
                                    },
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!(
                                    "{} 間監看 · {} 間直播中",
                                    self.live.snapshot.rooms.len(),
                                    live_count
                                ))
                                .when(live_count > 0, |row| {
                                    row.child(crate::ascii::broadcast("live-broadcast"))
                                }),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            Button::new("live-stats")
                                .outline()
                                .small()
                                .label(if self.live.statistics {
                                    "返回直播間"
                                } else {
                                    "觀測統計"
                                })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.stop_live_players(window);
                                    this.live.statistics = !this.live.statistics;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("live-preferences")
                                .ghost()
                                .small()
                                .icon(AppIcon::Settings)
                                .label("設定")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_live_panel(PanelKind::Preferences, window, cx)
                                })),
                        ),
                ),
        );
        controls = controls.child(
            div()
                .flex()
                .gap_2()
                .child(Input::new(&self.live.add).flex_1())
                .child(motion::press_feedback(
                    Button::new("live-add")
                        .primary()
                        .accessibility_label("新增監看")
                        .child(crate::ascii::button_label(
                            "live-add-wait",
                            "新增監看",
                            self.live.busy,
                        ))
                        .disabled(self.live.busy)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.add_live_room(window, cx)),
                        ),
                    0,
                    self.live.presses.clone(),
                    cx.theme().primary_foreground,
                ))
                .child(
                    Button::new("live-batch-add")
                        .outline()
                        .label("批次新增")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_live_panel(PanelKind::BatchAdd, window, cx)
                        })),
                ),
        );
        let mut tools = div().flex().flex_wrap().gap_2();
        for (index, (label, kind)) in [
            ("快速尋找", PanelKind::QuickFind),
            ("觀看清單", PanelKind::Watchlists),
            ("觀測紀錄", PanelKind::History(None)),
            ("分組管理", PanelKind::Groups),
            ("通知紀錄", PanelKind::Notifications),
        ]
        .into_iter()
        .enumerate()
        {
            tools = tools.child(
                Button::new(("live-tool", index))
                    .ghost()
                    .small()
                    .label(label)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_live_panel(kind.clone(), window, cx)
                    })),
            );
        }
        controls = controls.child(tools);
        if let Some(error) = self
            .live
            .error
            .clone()
            .or_else(|| self.live.snapshot.data.native.migration_error.clone())
        {
            let mut actions = vec![
                Button::new("live-retry-error")
                    .outline()
                    .small()
                    .label("重試")
                    .on_click(cx.listener(|this, _, window, cx| {
                        let failed = this.live.host.borrow().failed_room;
                        this.live.error = None;
                        if let Some(id) = failed {
                            this.live.loaded.remove(&id);
                            this.live.host.borrow_mut().remove(id);
                            this.live.previews.retain(|item| *item != id);
                            if this.live.multi {
                                match this.request_live_player(id, window, cx) {
                                    Ok(()) => {
                                        if !this.live.watch_ids.contains(&id) {
                                            this.live.watch_ids.push(id);
                                        }
                                    }
                                    Err(error) => this.live.error = Some(format!("{error:#}")),
                                }
                            } else {
                                this.toggle_live_preview(id, window, cx);
                            }
                        } else {
                            this.refresh(cx);
                        }
                        cx.notify();
                    }))
                    .into_any_element(),
                Button::new("live-clear-error")
                    .ghost()
                    .xsmall()
                    .label("關閉")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.live.error = None;
                        cx.notify();
                    }))
                    .into_any_element(),
            ];
            if error.contains("WebView2") {
                actions.push(
                    Button::new("live-runtime-guide")
                        .outline()
                        .small()
                        .label("WebView2 安裝說明")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.open_url(
                                "https://developer.microsoft.com/en-us/microsoft-edge/webview2/"
                                    .into(),
                                cx,
                            )
                        }))
                        .into_any_element(),
                );
            }
            controls = controls.child(ui::notice_banner(Tone::Danger, &error, actions, cx));
        }
        if self.live.statistics {
            let stats = self.render_live_stats(window, cx);
            return div()
                .size_full()
                .flex()
                .flex_col()
                .child(motion::chrome(
                    controls,
                    self.snapshot.settings.appearance.font_scale,
                ))
                .child(stats)
                .into_any_element();
        }
        let mut filters = div().flex().flex_wrap().items_center().gap_2().child(
            Input::new(&self.live.search)
                .w(px((available * 0.4).clamp(180., 360.)))
                .prefix(Icon::new(IconName::Search)),
        );
        for (index, (label, status)) in [
            ("全部", None),
            ("直播中", Some(RoomStatus::Live)),
            ("未開播", Some(RoomStatus::Offline)),
            ("輪播", Some(RoomStatus::Round)),
            ("鎖定", Some(RoomStatus::Locked)),
            ("隱藏", Some(RoomStatus::Hidden)),
            ("錯誤", Some(RoomStatus::Error)),
            ("未知", Some(RoomStatus::Unknown)),
        ]
        .into_iter()
        .enumerate()
        {
            filters = filters.child(
                Button::new(("live-status", index))
                    .xsmall()
                    .outline()
                    .selected(self.live.filter.status == status)
                    .label(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.live.filter.status = status;
                        this.filter_live(true, cx);
                    })),
            )
        }
        let mut modes = div().flex().flex_wrap().items_center().gap_2().child(
            TabBar::new("live-view-segment")
                .segmented()
                .small()
                .selected_index(if compact { 1 } else { 0 })
                .child(Tab::new().label("卡片").aria_label("卡片"))
                .child(Tab::new().label("清單").aria_label("清單"))
                .on_click(cx.listener(|this, index: &usize, _, cx| {
                    let view = if *index == 1 { "list" } else { "grid" };
                    this.live_change(
                        move |d| {
                            d.settings.room_view = view.into();
                            Ok(())
                        },
                        cx,
                        |_, _, _| {},
                    );
                })),
        );
        modes = modes
            .child(
                Button::new("live-filters")
                    .small()
                    .ghost()
                    .label("篩選與排序")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_live_panel(PanelKind::Filters, window, cx)
                    })),
            )
            .child(
                Button::new("live-batch-manage")
                    .small()
                    .ghost()
                    .label("批次管理")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_live_panel(PanelKind::BatchManage, window, cx)
                    })),
            )
            .child(
                Button::new("live-multi")
                    .small()
                    .primary()
                    .label(if self.live.multi {
                        "退出同屏"
                    } else {
                        "多房同屏"
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        if this.live.multi {
                            this.live.stop();
                            cx.notify();
                        } else {
                            let ids = this
                                .live
                                .visible
                                .iter()
                                .filter(|r| r.info.status == RoomStatus::Live)
                                .map(|r| r.saved.room_id)
                                .collect();
                            this.start_multi(ids, MultiViewLayout::Auto, 4, window, cx);
                        }
                    })),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("顯示 {} 間", self.live.visible.len())),
            );
        if self.live.filter.only_ids.is_some() {
            modes = modes.child(
                Button::new("live-clear-scope")
                    .ghost()
                    .small()
                    .label("解除清單範圍")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.live.filter.only_ids = None;
                        this.filter_live(true, cx);
                    })),
            )
        }
        controls = controls.child(filters).child(modes);
        let content = if self.live.multi {
            self.render_multi(window, cx)
        } else if self.live.visible.is_empty() {
            ui::empty_state(
                AppIcon::Live,
                "沒有符合條件的直播間",
                "輸入房號新增監看，或重設篩選查看全部。",
                Some(
                    Button::new("live-reset")
                        .outline()
                        .label("重設篩選")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.live.filter = RoomFilter::default();
                            this.live
                                .search
                                .update(cx, |s, cx| s.set_value("", window, cx));
                            this.filter_live(true, cx);
                        }))
                        .into_any_element(),
                ),
                cx,
            )
        } else {
            let entity = cx.entity();
            crate::scroll::smooth_list(
                "live-rooms",
                cx.entity_id(),
                self.live.list.clone(),
                move |row, window, cx| {
                    entity.update(cx, |this, cx| this.live_room_row(row, window, cx))
                },
            )
            .into_any_element()
        };
        let host = self.live.host.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(motion::chrome(
                controls,
                self.snapshot.settings.appearance.font_scale,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .child(content),
            )
            .child(
                canvas(|_, _, _| {}, move |_, _, _, _| host.borrow().finish_frame())
                    .absolute()
                    .inset_0(),
            )
            .into_any_element()
    }
    fn live_room_row(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(ids) = self.live.rows.get(index).cloned() else {
            return div().into_any_element();
        };
        let width = (f32::from(window.viewport_size().width)
            - ui::SIDEBAR_WIDTH
            - 48.
            - 12. * self.live.columns.saturating_sub(1) as f32)
            / self.live.columns.max(1) as f32;
        let compact = self.live.snapshot.data.settings.room_view == "list";
        let mut row = div().px_4().py_2().flex().gap_3();
        for id in ids {
            if let Some(room) = self
                .live
                .visible
                .iter()
                .find(|r| r.saved.room_id == id)
                .cloned()
            {
                row = row.child(self.live_room_card(room, width.max(180.), compact, window, cx));
            }
        }
        row.into_any_element()
    }
    fn live_room_card(
        &mut self,
        room: DisplayRoom,
        width: f32,
        compact: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = room.saved.room_id;
        let preview = self.live.previews.contains(&id);
        let cover_width = if compact { 160. } else { width };
        let height = cover_width * 0.5625;
        let mut card = div()
            .id(("live-room", id))
            .w(px(width))
            .rounded_lg()
            .overflow_hidden()
            .bg(cx.theme().muted)
            .border_1()
            .border_color(if self.live.highlighted == Some(id) {
                cx.theme().primary
            } else {
                cx.theme().border
            })
            .flex()
            .flex_col();
        let visual =
            if preview {
                self.live_player_surface(id, cover_width, height, cx)
            } else {
                let picture = self.picture(&room.info.cover, cover_width, height, cx);
                div()
                    .id(("live-cover", id))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.toggle_live_preview(id, window, cx)
                    }))
                    .child(picture)
                    .into_any_element()
            };
        let online = self.live_number((id, "online"), room.info.online, window, cx);
        let attention = self.live_number((id, "attention"), room.info.attention, window, cx);
        let mut info = div()
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(highlight(&room.info.uname, &self.live.filter.query, cx)),
                    )
                    .child(if room.info.status == RoomStatus::Live {
                        ui::live_pill_badge(("live-status-dot", id), room.info.status.label(), cx)
                    } else {
                        Tag::secondary()
                            .xsmall()
                            .child(room.info.status.label())
                            .into_any_element()
                    }),
            )
            .child(
                div()
                    .min_h(px(30.))
                    .line_clamp(2)
                    .text_sm()
                    .child(highlight(
                        if room.info.title.is_empty() {
                            "尚未取得直播標題"
                        } else {
                            &room.info.title
                        },
                        &self.live.filter.query,
                        cx,
                    )),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "#{id} · {} · 人氣 {} · 關注 {}",
                        if room.info.area_name.is_empty() {
                            "未分類"
                        } else {
                            &room.info.area_name
                        },
                        online,
                        attention
                    )),
            );
        if !room.saved.note.is_empty() {
            info = info.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .line_clamp(2)
                    .child(highlight(&room.saved.note, &self.live.filter.query, cx)),
            );
        }
        if let Some(error) = &room.info.error {
            info = info.child(
                div()
                    .text_xs()
                    .text_color(ui::tone_colors(Tone::Danger, cx.theme().is_dark()).fg)
                    .line_clamp(2)
                    .child(error.clone()),
            );
        }
        let pinned = room.saved.pinned;
        let notify = room.saved.notify;
        info = info.child(
            div()
                .flex()
                .flex_wrap()
                .gap_1()
                .child(motion::press_feedback(
                    Button::new(("live-preview", id))
                        .xsmall()
                        .primary()
                        .label(if preview { "收合" } else { "預覽" })
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.toggle_live_preview(id, window, cx)
                        })),
                    id,
                    self.live.presses.clone(),
                    cx.theme().primary_foreground,
                ))
                .child(
                    Button::new(("live-details", id))
                        .xsmall()
                        .ghost()
                        .label("資訊")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_live_panel(PanelKind::Details(id), window, cx)
                        })),
                )
                .child(
                    Button::new(("live-pin", id))
                        .xsmall()
                        .ghost()
                        .selected(pinned)
                        .icon(AppIcon::Pin)
                        .accessibility_label("切換置頂")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.live_change(
                                move |d| {
                                    d.patch_room(
                                        id,
                                        RoomPatch {
                                            pinned: Some(!pinned),
                                            ..Default::default()
                                        },
                                    )
                                },
                                cx,
                                |_, _, _| {},
                            )
                        })),
                )
                .child(
                    Button::new(("live-notify", id))
                        .xsmall()
                        .ghost()
                        .selected(notify)
                        .icon(AppIcon::Bell)
                        .accessibility_label("切換此房提醒")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.live_change(
                                move |d| {
                                    d.patch_room(
                                        id,
                                        RoomPatch {
                                            notify: Some(!notify),
                                            ..Default::default()
                                        },
                                    )
                                },
                                cx,
                                |_, _, _| {},
                            )
                        })),
                )
                .child(
                    Button::new(("live-external", id))
                        .xsmall()
                        .ghost()
                        .icon(IconName::ExternalLink)
                        .accessibility_label("開啟官方直播間")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_url(format!("https://live.bilibili.com/{id}"), cx)
                        })),
                ),
        );
        if compact {
            card = card
                .flex_row()
                .child(div().w(px(cover_width)).flex_shrink_0().child(visual))
                .child(info.flex_1());
        } else {
            card = card.child(visual).child(info);
        }
        let draggable = self.live.filter.advanced.sort_by == "order"
            && self.live.filter.advanced.sort_dir == "asc"
            && !preview;
        let name = room.info.uname;
        let card = card
            .when(draggable, |card| {
                card.on_drag(RoomDrag { room_id: id, name }, |drag, _, _, cx| {
                    cx.new(|_| RoomDrag {
                        room_id: drag.room_id,
                        name: drag.name.clone(),
                    })
                })
                .drag_over::<RoomDrag>(|style, _, _, cx| style.border_color(cx.theme().primary))
                .on_drop(cx.listener(move |this, drag: &RoomDrag, _, cx| {
                    if drag.room_id != id {
                        this.move_live_room(drag.room_id, id, cx);
                    }
                }))
            })
            .into_any_element();
        motion::reflow(
            card,
            id,
            self.live.reflow.clone(),
            self.live.layout_revision,
            self.live.render_frame,
            width,
            cx.reduce_motion() || self.live.keyboard,
        )
        .into_any_element()
    }
    pub(super) fn live_player_surface(
        &mut self,
        id: u64,
        width: f32,
        height: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let host = self.live.host.clone();
        let mut base = div()
            .relative()
            .w(px(width))
            .h(px(height))
            .bg(rgb(0x090d16))
            .overflow_hidden();
        if !self.live.panels.is_empty() {
            if let Some(image) = self.live.shots.get(&id) {
                base = base.child(
                    img(image.clone())
                        .size_full()
                        .object_fit(ObjectFit::Contain),
                );
            } else {
                let cover = self
                    .live
                    .snapshot
                    .data
                    .native
                    .cached_rooms
                    .get(&id)
                    .map(|r| r.cover.clone())
                    .unwrap_or_default();
                base = base.child(self.picture(&cover, width, height, cx));
            }
        } else if !self.live.loaded.contains(&id) {
            base = base.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .text_xs()
                    .text_color(rgb(0x94a3b8))
                    .child("載入官方播放器")
                    .child(crate::ascii::connecting(("player-wait", id))),
            );
        }
        base.child(
            canvas(
                move |bounds, window, _| {
                    host.borrow_mut().layout(
                        id,
                        bounds,
                        window.content_mask().bounds,
                        window.scale_factor(),
                    )
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0(),
        )
        .into_any_element()
    }
    fn render_multi(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let width = (f32::from(window.viewport_size().width) - ui::SIDEBAR_WIDTH - 48.).max(200.);
        let mut options = div().flex().flex_wrap().gap_2();
        for (index, layout) in [
            MultiViewLayout::Auto,
            MultiViewLayout::Grid2,
            MultiViewLayout::Grid3,
            MultiViewLayout::Focus,
        ]
        .into_iter()
        .enumerate()
        {
            options = options.child(
                Button::new(("live-layout", index))
                    .small()
                    .outline()
                    .selected(layout == self.live.layout)
                    .label(layout.label())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.live.layout = layout.clone();
                        cx.notify();
                    })),
            );
        }
        for limit in [1, 2, 4, 9] {
            options = options.child(
                Button::new(("live-limit", limit))
                    .xsmall()
                    .outline()
                    .selected(self.live.limit == limit)
                    .label(format!("{limit} 間"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.live.limit = limit;
                        this.live.watch_ids.truncate(limit);
                        this.live.host.borrow_mut().retain(&this.live.watch_ids);
                        cx.notify();
                    })),
            );
        }
        options = options
            .child(
                Button::new("live-native-fullscreen")
                    .outline()
                    .small()
                    .label("全螢幕播放")
                    .disabled(self.live.watch_ids.is_empty())
                    .on_click(cx.listener(|this, _, window, cx| {
                        if let Some(id) = this.live.focused.or(this.live.watch_ids.first().copied())
                        {
                            this.live.fullscreen = Some(id);
                            window.toggle_fullscreen();
                            cx.notify();
                        }
                    })),
            )
            .child(
                Button::new("live-save-current")
                    .small()
                    .ghost()
                    .label("儲存這組同屏")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_live_panel(PanelKind::SaveCurrent, window, cx)
                    })),
            )
            .child(
                Button::new("live-danmaku")
                    .small()
                    .outline()
                    .selected(self.live.danmaku)
                    .label("彈幕")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.live.danmaku = !this.live.danmaku;
                        let ids = this.live.watch_ids.clone();
                        for id in ids {
                            if let Err(error) = this.request_live_player(id, window, cx) {
                                this.live.error = Some(error.to_string());
                            }
                        }
                        cx.notify();
                    })),
            );
        let mut picker = div().flex().flex_wrap().gap_2();
        for room in self.live.snapshot.rooms.iter().filter(|r| {
            r.info.status == RoomStatus::Live || self.live.watch_ids.contains(&r.saved.room_id)
        }) {
            let id = room.saved.room_id;
            picker = picker.child(
                Button::new(("multi-pick", id))
                    .xsmall()
                    .outline()
                    .selected(self.live.watch_ids.contains(&id))
                    .label(room.info.uname.clone())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.toggle_multi_room(id, window, cx)
                    })),
            );
        }
        let ids = self.live.watch_ids.clone();
        let columns = match self.live.layout {
            MultiViewLayout::Grid3 => 3,
            MultiViewLayout::Grid2 => 2,
            MultiViewLayout::Focus => 1,
            _ => {
                if ids.len() <= 1 {
                    1
                } else {
                    2
                }
            }
        };
        let card_width = (width - 12. * (columns - 1) as f32) / columns as f32;
        let mut videos = div().flex().flex_wrap().gap_3();
        if self.live.layout == MultiViewLayout::Focus {
            let focused = self.live.focused.or(ids.first().copied());
            let narrow = width < 620.;
            let main_width = if narrow || ids.len() <= 1 {
                width
            } else {
                (width - 12.) * 0.74
            };
            let side_width = if narrow {
                (width - 12.) / 2.
            } else {
                width - main_width - 12.
            };
            if let Some(id) = focused {
                videos =
                    videos.child(self.live_player_surface(id, main_width, main_width * 0.5625, cx));
            }
            let mut sides = div()
                .flex()
                .gap_2()
                .when(!narrow, |d| d.flex_col())
                .when(narrow, |d| d.flex_wrap().w_full());
            for id in ids.into_iter().filter(|id| Some(*id) != focused) {
                let name = self
                    .live
                    .snapshot
                    .rooms
                    .iter()
                    .find(|r| r.saved.room_id == id)
                    .map(|r| r.info.uname.clone())
                    .unwrap_or_else(|| id.to_string());
                sides = sides.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .w(px(side_width))
                        .child(self.live_player_surface(id, side_width, side_width * 0.5625, cx))
                        .child(
                            Button::new(("multi-focus", id))
                                .outline()
                                .xsmall()
                                .label(format!("聚焦 {name}"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.live.focused = Some(id);
                                    cx.notify();
                                })),
                        ),
                );
            }
            videos = videos.child(sides);
        } else {
            for id in ids {
                videos =
                    videos.child(self.live_player_surface(id, card_width, card_width * 0.5625, cx));
            }
        }
        crate::scroll::smooth_scroll_container(
            "multi-scroll",
            cx.entity_id(),
            &self.live.multi_scroll,
            div()
                .p_4()
                .flex()
                .flex_col()
                .gap_3()
                .child(options)
                .child(picker)
                .child(videos),
        )
        .into_any_element()
    }
}
fn highlight(text: &str, query: &str, cx: &App) -> AnyElement {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return text.to_owned().into_any_element();
    }
    let lower = text.to_lowercase();
    if let Some(start) = lower.find(&query).filter(|start| {
        text.is_char_boundary(*start) && text.is_char_boundary(*start + query.len())
    }) {
        div()
            .flex()
            .min_w_0()
            .child(text[..start].to_owned())
            .child(
                div()
                    .bg(cx.theme().selection)
                    .child(text[start..start + query.len()].to_owned()),
            )
            .child(text[start + query.len()..].to_owned())
            .into_any_element()
    } else {
        text.to_owned().into_any_element()
    }
}
