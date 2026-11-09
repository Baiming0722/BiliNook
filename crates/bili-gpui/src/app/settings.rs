use super::common::*;
use super::*;
use crate::{
    assets::AppIcon,
    scroll::{smooth_list, smooth_scroll_container},
    ui::{self, Tone},
};
use bili_core::services::UpPatch;
use gpui::component::{
    Disableable as _, Icon, IconName, Selectable as _, Sizable as _,
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
    switch::Switch,
    tab::{Tab, TabBar},
    tag::Tag,
};
use serde_json::{Map, Value};

const SETTINGS_SECTIONS: [(AppIcon, &str); 4] = [
    (AppIcon::UserPlus, "UP 主管理"),
    (AppIcon::Users, "分組管理"),
    (AppIcon::Bell, "輪詢與通知"),
    (AppIcon::Database, "備份與還原"),
];

const THEME_MODES: [(&str, &str, IconName); 3] = [
    ("dark", "深色", IconName::Moon),
    ("light", "淺色", IconName::Sun),
    ("system", "跟隨系統", IconName::Palette),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ValidatedGeneralPatch {
    pub poll_interval_live_sec: u64,
    pub poll_interval_dynamic_sec: u64,
    pub enable_notifications: bool,
    pub notify_live: bool,
    pub notify_video: bool,
    pub notify_dynamic: bool,
    pub sound_enabled: bool,
    pub theme: String,
    pub custom_proxy_url: Option<String>,
}

impl ValidatedGeneralPatch {
    /// 以明確欄位常數組裝設定 patch，避免手寫單行 JSON 字串遺漏或拼錯鍵名。
    pub(super) fn into_json_patch(self) -> Value {
        let mut map = Map::with_capacity(9);
        map.insert(
            "pollIntervalLiveSec".into(),
            Value::from(self.poll_interval_live_sec),
        );
        map.insert(
            "pollIntervalDynamicSec".into(),
            Value::from(self.poll_interval_dynamic_sec),
        );
        map.insert(
            "enableNotifications".into(),
            Value::from(self.enable_notifications),
        );
        map.insert("notifyLive".into(), Value::from(self.notify_live));
        map.insert("notifyVideo".into(), Value::from(self.notify_video));
        map.insert("notifyDynamic".into(), Value::from(self.notify_dynamic));
        map.insert("soundEnabled".into(), Value::from(self.sound_enabled));
        map.insert("theme".into(), Value::from(self.theme));
        map.insert(
            "customProxyUrl".into(),
            match self.custom_proxy_url {
                Some(url) => Value::String(url),
                None => Value::Null,
            },
        );
        Value::Object(map)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GeneralValidation {
    pub live_error: Option<String>,
    pub dynamic_error: Option<String>,
    pub proxy_error: Option<String>,
    pub parsed: Option<ValidatedGeneralPatch>,
}

pub(super) fn validate_general_inputs(
    live_raw: &str,
    dynamic_raw: &str,
    proxy_raw: &str,
    general: &AppSettings,
) -> GeneralValidation {
    let mut live_error = None;
    let mut dynamic_error = None;
    let mut proxy_error = None;

    let live_sec = match live_raw.trim().parse::<u64>() {
        Ok(sec) if (ui::MIN_LIVE_POLL_SEC..=ui::MAX_POLL_SEC).contains(&sec) => Some(sec),
        Ok(_) => {
            live_error = Some(format!(
                "須介於 {}–{} 秒",
                ui::MIN_LIVE_POLL_SEC,
                ui::MAX_POLL_SEC
            ));
            None
        }
        Err(_) => {
            live_error = Some("請輸入整數秒數".into());
            None
        }
    };

    let dynamic_sec = match dynamic_raw.trim().parse::<u64>() {
        Ok(sec) if (ui::MIN_DYNAMIC_POLL_SEC..=ui::MAX_POLL_SEC).contains(&sec) => Some(sec),
        Ok(_) => {
            dynamic_error = Some(format!(
                "須介於 {}–{} 秒",
                ui::MIN_DYNAMIC_POLL_SEC,
                ui::MAX_POLL_SEC
            ));
            None
        }
        Err(_) => {
            dynamic_error = Some("請輸入整數秒數".into());
            None
        }
    };

    let trimmed_proxy = proxy_raw.trim();
    let custom_proxy_url = if trimmed_proxy.is_empty() {
        None
    } else {
        match url::Url::parse(trimmed_proxy) {
            Ok(parsed)
                if matches!(parsed.scheme(), "http" | "https" | "socks5" | "socks5h")
                    && parsed.host_str().is_some() =>
            {
                Some(trimmed_proxy.to_owned())
            }
            _ => {
                proxy_error = Some("格式無效（支援 http / https / socks5）".into());
                None
            }
        }
    };

    let parsed = if live_error.is_none() && dynamic_error.is_none() && proxy_error.is_none() {
        Some(ValidatedGeneralPatch {
            poll_interval_live_sec: live_sec.unwrap_or(general.poll_interval_live_sec),
            poll_interval_dynamic_sec: dynamic_sec.unwrap_or(general.poll_interval_dynamic_sec),
            enable_notifications: general.enable_notifications,
            notify_live: general.notify_live,
            notify_video: general.notify_video,
            notify_dynamic: general.notify_dynamic,
            sound_enabled: general.sound_enabled,
            theme: general.theme.clone(),
            custom_proxy_url,
        })
    } else {
        None
    };

    GeneralValidation {
        live_error,
        dynamic_error,
        proxy_error,
        parsed,
    }
}

pub(super) struct GroupDraft {
    source: FollowGroup,
    name: Entity<InputState>,
}

pub(super) struct SettingsState {
    section: usize,
    section_epoch: u64,
    pub initialized: bool,
    pub busy: bool,
    pub form_error: Option<String>,
    add: Entity<InputState>,
    up_filter: Entity<InputState>,
    group_member_filter: Entity<InputState>,
    live: Entity<InputState>,
    dynamic: Entity<InputState>,
    proxy: Entity<InputState>,
    note: Entity<InputState>,
    tags: Entity<InputState>,
    pub general: AppSettings,
    pub up_list: ListState,
    up_columns: usize,
    up_rows_count: usize,
    filtered_up_indices: Vec<usize>,
    editing: Option<u64>,
    deleting: Option<u64>,
    groups: Vec<GroupDraft>,
    group_index: usize,
    group_sidebar_scroll: ScrollHandle,
    group_list: ListState,
    group_columns: usize,
    group_rows_count: usize,
    filtered_group_up_indices: Vec<usize>,
    delete_group: bool,
    general_scroll: ScrollHandle,
    backup_scroll: ScrollHandle,
    pending_import: Option<Value>,
}

impl SettingsState {
    pub(super) fn general_sound_enabled(&self) -> bool {
        self.general.sound_enabled
    }

    pub(super) fn select_section(&mut self, section: usize) {
        let next = section.min(SETTINGS_SECTIONS.len() - 1);
        if self.section != next {
            self.section = next;
            self.section_epoch = self.section_epoch.wrapping_add(1);
        }
        self.form_error = None;
    }

    pub(super) fn is_general_dirty(&self, snapshot: &Snapshot, cx: &App) -> bool {
        if !self.initialized {
            return false;
        }
        let saved = &snapshot.settings;
        let live_raw = value(&self.live, cx);
        let dynamic_raw = value(&self.dynamic, cx);
        let proxy_raw = value(&self.proxy, cx).trim().to_owned();
        let saved_proxy = saved.custom_proxy_url.clone().unwrap_or_default();

        live_raw.trim() != saved.poll_interval_live_sec.to_string()
            || dynamic_raw.trim() != saved.poll_interval_dynamic_sec.to_string()
            || proxy_raw != saved_proxy
            || self.general.enable_notifications != saved.enable_notifications
            || self.general.notify_live != saved.notify_live
            || self.general.notify_video != saved.notify_video
            || self.general.notify_dynamic != saved.notify_dynamic
            || self.general.sound_enabled != saved.sound_enabled
            || self.general.theme != saved.theme
    }

    pub(super) fn is_groups_dirty(&self, snapshot: &Snapshot, cx: &App) -> bool {
        if !self.initialized {
            return false;
        }
        let saved = snapshot
            .settings
            .follow_groups
            .clone()
            .unwrap_or_else(default_groups);
        if self.groups.len() != saved.len() {
            return true;
        }
        self.groups.iter().zip(saved.iter()).any(|(draft, orig)| {
            draft.source.id != orig.id
                || value(&draft.name, cx).trim() != orig.name
                || draft.source.mids != orig.mids
        })
    }

    pub(super) fn is_dirty(&self, snapshot: &Snapshot, cx: &App) -> bool {
        self.is_general_dirty(snapshot, cx) || self.is_groups_dirty(snapshot, cx)
    }

    pub fn new(window: &mut Window, cx: &mut App) -> Self {
        Self {
            section: 0,
            section_epoch: 0,
            initialized: false,
            busy: false,
            form_error: None,
            add: input(
                "",
                "輸入 UID、空間網址、b23.tv 或 BV 號（Enter 新增）",
                window,
                cx,
            ),
            up_filter: input("", "篩選名稱、UID、備註或標籤", window, cx),
            group_member_filter: input("", "篩選成員名稱或 UID", window, cx),
            live: input("60", "至少 30 秒", window, cx),
            dynamic: input("300", "至少 60 秒", window, cx),
            proxy: input("", "選填 HTTP(S) / SOCKS5 代理網址", window, cx),
            note: input("", "備註（選填）", window, cx),
            tags: input("", "標籤，以逗號分隔（選填）", window, cx),
            general: AppSettings::default(),
            up_list: ListState::new(0, ListAlignment::Top, px(280.)),
            up_columns: 2,
            up_rows_count: 0,
            filtered_up_indices: Vec::new(),
            editing: None,
            deleting: None,
            groups: Vec::new(),
            group_index: 0,
            group_sidebar_scroll: ScrollHandle::new(),
            group_list: ListState::new(0, ListAlignment::Top, px(200.)),
            group_columns: 2,
            group_rows_count: 0,
            filtered_group_up_indices: Vec::new(),
            delete_group: false,
            general_scroll: ScrollHandle::new(),
            backup_scroll: ScrollHandle::new(),
            pending_import: None,
        }
    }
}

impl BiliApp {
    pub(super) fn subscribe_settings(&mut self, window: &Window, cx: &mut Context<Self>) {
        cx.subscribe_in(
            &self.settings.add,
            window,
            |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.add_up(cx);
                }
            },
        )
        .detach();

        cx.subscribe_in(
            &self.settings.up_filter,
            window,
            |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.sync_up_grid_rows(true, cx);
                    cx.notify();
                }
            },
        )
        .detach();

        cx.subscribe_in(
            &self.settings.group_member_filter,
            window,
            |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.sync_group_member_rows(true, cx);
                    cx.notify();
                }
            },
        )
        .detach();

        for general_input in [
            self.settings.live.clone(),
            self.settings.dynamic.clone(),
            self.settings.proxy.clone(),
        ] {
            cx.subscribe_in(
                &general_input,
                window,
                |this, _, event: &InputEvent, _, cx| match event {
                    InputEvent::PressEnter { .. } => this.save_general(cx),
                    InputEvent::Change => {
                        this.settings.form_error = None;
                        cx.notify();
                    }
                    _ => {}
                },
            )
            .detach();
        }

        for edit_input in [self.settings.note.clone(), self.settings.tags.clone()] {
            cx.subscribe_in(&edit_input, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. })
                    && let Some(mid) = this.settings.editing
                {
                    this.commit_up_edit(mid, cx);
                }
            })
            .detach();
        }
    }

    pub(super) fn prepare_settings(&mut self, window: &mut Window, cx: &mut App) {
        if self.settings.initialized {
            self.sync_up_grid_rows(false, cx);
            self.sync_group_member_rows(false, cx);
            return;
        }
        self.settings.initialized = true;
        self.reset_general_draft(window, cx);
        self.reset_groups_draft(window, cx);
        self.sync_up_grid_rows(true, cx);
        self.sync_group_member_rows(true, cx);
    }

    pub(super) fn reset_general_draft(&mut self, window: &mut Window, cx: &mut App) {
        self.settings.general = self.snapshot.settings.clone();
        self.settings.form_error = None;
        let fields = [
            (
                self.settings.live.clone(),
                self.snapshot.settings.poll_interval_live_sec.to_string(),
            ),
            (
                self.settings.dynamic.clone(),
                self.snapshot.settings.poll_interval_dynamic_sec.to_string(),
            ),
            (
                self.settings.proxy.clone(),
                self.snapshot
                    .settings
                    .custom_proxy_url
                    .clone()
                    .unwrap_or_default(),
            ),
        ];
        for (entity, text) in fields {
            entity.update(cx, |state, cx| state.set_value(text, window, cx));
        }
    }

    fn reset_groups_draft(&mut self, window: &mut Window, cx: &mut App) {
        self.settings.form_error = None;
        self.settings.groups = self
            .groups()
            .into_iter()
            .map(|source| {
                let name = input(&source.name, "分組名稱", window, cx);
                GroupDraft { source, name }
            })
            .collect();
        self.settings.group_index = 0;
        self.settings.delete_group = false;
        self.sync_group_member_rows(true, cx);
    }

    fn sync_up_grid_rows(&mut self, reset_scroll: bool, cx: &App) {
        let query = value(&self.settings.up_filter, cx).trim().to_lowercase();
        let next: Vec<usize> =
            self.snapshot
                .ups
                .iter()
                .enumerate()
                .filter(|(_, up)| {
                    if query.is_empty() {
                        return true;
                    }
                    up.name.to_lowercase().contains(&query)
                        || up.mid.to_string().contains(&query)
                        || up
                            .custom_note
                            .as_deref()
                            .is_some_and(|n| n.to_lowercase().contains(&query))
                        || up.tags.as_ref().is_some_and(|tags| {
                            tags.iter().any(|t| t.to_lowercase().contains(&query))
                        })
                })
                .map(|(idx, _)| idx)
                .collect();

        let cols = self.settings.up_columns.max(1);
        let rows = next.len().div_ceil(cols);
        if next != self.settings.filtered_up_indices
            || rows != self.settings.up_rows_count
            || self.settings.up_list.item_count() != rows
            || reset_scroll
        {
            let top = self.settings.up_list.logical_scroll_top();
            self.settings.filtered_up_indices = next;
            self.settings.up_rows_count = rows;
            self.settings.up_list.reset(rows);
            if !reset_scroll {
                self.settings.up_list.scroll_to(top);
            }
        }
    }

    fn sync_group_member_rows(&mut self, reset_scroll: bool, cx: &App) {
        let query = value(&self.settings.group_member_filter, cx)
            .trim()
            .to_lowercase();
        let next: Vec<usize> = self
            .snapshot
            .ups
            .iter()
            .enumerate()
            .filter(|(_, up)| {
                if query.is_empty() {
                    return true;
                }
                up.name.to_lowercase().contains(&query)
                    || up.mid.to_string().contains(&query)
                    || up
                        .custom_note
                        .as_deref()
                        .is_some_and(|n| n.to_lowercase().contains(&query))
            })
            .map(|(idx, _)| idx)
            .collect();

        let cols = self.settings.group_columns.max(1);
        let rows = next.len().div_ceil(cols);
        if next != self.settings.filtered_group_up_indices
            || rows != self.settings.group_rows_count
            || self.settings.group_list.item_count() != rows
            || reset_scroll
        {
            let top = self.settings.group_list.logical_scroll_top();
            self.settings.filtered_group_up_indices = next;
            self.settings.group_rows_count = rows;
            self.settings.group_list.reset(rows);
            if !reset_scroll {
                self.settings.group_list.scroll_to(top);
            }
        }
    }

    fn commit_up_edit(&mut self, mid: u64, cx: &mut Context<Self>) {
        let note = value(&self.settings.note, cx);
        let tags = value(&self.settings.tags, cx)
            .split([',', '，'])
            .map(str::trim)
            .filter(|tag| !tag.is_empty())
            .map(str::to_owned)
            .collect();
        self.update_up(
            mid,
            UpPatch {
                note: Some(note),
                tags: Some(tags),
                ..Default::default()
            },
            cx,
        );
    }

    fn update_up(&mut self, mid: u64, patch: UpPatch, cx: &mut Context<Self>) {
        let Some(services) = self.services.clone() else {
            return;
        };
        if self.settings.busy {
            return;
        }
        self.settings.busy = true;
        self.settings.form_error = None;
        self.run(
            async move { services.update_up(mid, patch).await },
            cx,
            |this, result, cx| {
                if result.is_ok() {
                    this.settings.editing = None;
                }
                this.saved(result, "UP 主設定已儲存", cx);
            },
        );
    }

    fn add_up(&mut self, cx: &mut Context<Self>) {
        let Some(services) = self.services.clone() else {
            return;
        };
        if self.settings.busy {
            return;
        }
        let raw_input = value(&self.settings.add, cx);
        if raw_input.trim().is_empty() {
            self.settings.form_error =
                Some("請先輸入 B 站 UID、個人空間網址、b23.tv 或 BV 號".into());
            cx.notify();
            return;
        }
        self.settings.busy = true;
        self.settings.form_error = None;
        self.run(
            async move { services.add_up(raw_input).await },
            cx,
            |this, result, cx| {
                this.settings.busy = false;
                match result {
                    Ok(up) => {
                        this.settings.form_error = None;
                        this.show_notice(format!("已加入特別關注：{}", up.name), cx);
                        this.reload_snapshot(cx);
                        let handle = this.window;
                        let _ = handle.update(cx, |_, window, cx| {
                            this.settings
                                .add
                                .update(cx, |state, cx| state.set_value("", window, cx))
                        });
                    }
                    Err(error) => this.settings.form_error = Some(error.to_string()),
                }
                cx.notify();
            },
        );
    }

    fn remove_up(&mut self, mid: u64, cx: &mut Context<Self>) {
        let Some(services) = self.services.clone() else {
            return;
        };
        if self.settings.busy {
            return;
        }
        self.settings.busy = true;
        self.settings.form_error = None;
        self.run(
            async move { services.remove_up(mid).await },
            cx,
            move |this, result, cx| {
                if result.is_ok() {
                    this.settings.deleting = None;
                    for group in &mut this.settings.groups {
                        group.source.mids.retain(|id| *id != mid);
                    }
                }
                this.saved(result, "已移除追蹤，並同步清理分組成員", cx);
            },
        );
    }

    fn save_general(&mut self, cx: &mut Context<Self>) {
        let Some(services) = self.services.clone() else {
            return;
        };
        if self.settings.busy {
            return;
        }
        let validation = validate_general_inputs(
            &value(&self.settings.live, cx),
            &value(&self.settings.dynamic, cx),
            &value(&self.settings.proxy, cx),
            &self.settings.general,
        );
        let Some(validated) = validation.parsed else {
            self.settings.form_error = validation
                .live_error
                .or(validation.dynamic_error)
                .or(validation.proxy_error);
            cx.notify();
            return;
        };
        let patch = validated.into_json_patch();
        self.settings.busy = true;
        self.settings.form_error = None;
        self.run(
            async move { services.patch_settings(patch).await },
            cx,
            |this, result, cx| this.saved(result, "輪詢與通知設定已儲存", cx),
        );
    }

    fn save_groups(&mut self, cx: &mut Context<Self>) {
        let Some(services) = self.services.clone() else {
            return;
        };
        if self.settings.busy {
            return;
        }
        let groups: Vec<FollowGroup> = self
            .settings
            .groups
            .iter()
            .map(|draft| {
                let mut group = draft.source.clone();
                group.name = value(&draft.name, cx);
                group
            })
            .collect();
        self.settings.busy = true;
        self.settings.form_error = None;
        self.run(
            async move { services.save_groups(groups).await },
            cx,
            |this, result, cx| this.saved(result, "分組設定已儲存並同步至各分頁", cx),
        );
    }

    #[inline(never)]
    pub(super) fn render_settings(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let viewport_width = f32::from(window.viewport_size().width);
        let side_nav_mode = viewport_width >= ui::BP_SETTINGS_SIDE_NAV;
        let detail_width = viewport_width
            - ui::SIDEBAR_WIDTH
            - if side_nav_mode {
                ui::SETTINGS_SUBNAV_WIDTH
            } else {
                0.0
            };

        let next_up_cols = if detail_width >= 760.0 { 2 } else { 1 };
        if self.settings.up_columns != next_up_cols {
            self.settings.up_columns = next_up_cols;
        }
        let next_group_cols = if detail_width >= 1040.0 {
            3
        } else if detail_width >= 620.0 {
            2
        } else {
            1
        };
        if self.settings.group_columns != next_group_cols {
            self.settings.group_columns = next_group_cols;
        }

        self.prepare_settings(window, cx);
        self.sync_up_grid_rows(false, cx);
        self.sync_group_member_rows(false, cx);

        let current_section = self.settings.section;
        let section_epoch = self.settings.section_epoch;
        let general_dirty = self.settings.is_general_dirty(&self.snapshot, cx);
        let groups_dirty = self.settings.is_groups_dirty(&self.snapshot, cx);
        let has_pending_import = self.settings.pending_import.is_some();
        let ups_count = self.snapshot.ups.len();
        let groups_count = self.settings.groups.len();

        let raw_section = match current_section {
            0 => self.render_settings_ups(detail_width, cx),
            1 => self.render_groups(detail_width, window, cx),
            2 => self.render_settings_general(detail_width, cx),
            _ => self.render_settings_backup(detail_width, cx),
        };
        let animated_content = ui::enter_transition(
            div().size_full().child(raw_section),
            ("settings-section-enter", section_epoch),
        );

        if side_nav_mode {
            let subnav = div()
                .w(px(ui::SETTINGS_SUBNAV_WIDTH))
                .flex_shrink_0()
                .h_full()
                .p_3p5()
                .border_r_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().background)
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .px_2p5()
                        .py_1p5()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(cx.theme().muted_foreground)
                        .child("設定分區"),
                )
                .children(SETTINGS_SECTIONS.into_iter().enumerate().map(
                    |(index, (icon, label))| {
                        let badge = match index {
                            0 => Some((ups_count.to_string(), false)),
                            1 if groups_dirty => Some(("未儲存".into(), true)),
                            1 => Some((groups_count.to_string(), false)),
                            2 if general_dirty => Some(("未儲存".into(), true)),
                            3 if has_pending_import => Some(("待確認".into(), true)),
                            _ => None,
                        };
                        ui::nav_item(
                            ("settings-section", index),
                            icon,
                            label,
                            badge,
                            current_section == index,
                            cx,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.settings.select_section(index);
                            cx.notify();
                        }))
                    },
                ));

            div()
                .size_full()
                .flex()
                .min_w_0()
                .min_h_0()
                .child(subnav)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .overflow_hidden()
                        .child(animated_content),
                )
                .into_any_element()
        } else {
            div()
                .size_full()
                .flex()
                .flex_col()
                .min_w_0()
                .min_h_0()
                .child(
                    div()
                        .flex_shrink_0()
                        .px_6()
                        .py_2p5()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(
                            TabBar::new("settings-compact-tabs")
                                .segmented()
                                .small()
                                .selected_index(current_section)
                                .children(SETTINGS_SECTIONS.into_iter().map(|(icon, label)| {
                                    Tab::new()
                                        .prefix(Icon::new(icon).small())
                                        .label(label)
                                        .aria_label(label)
                                }))
                                .on_click(cx.listener(|this, idx: &usize, _, cx| {
                                    this.settings.select_section(*idx);
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .overflow_hidden()
                        .child(animated_content),
                )
                .into_any_element()
        }
    }

    #[inline(never)]
    fn render_settings_ups(&mut self, detail_width: f32, cx: &mut Context<Self>) -> AnyElement {
        let busy = self.settings.busy;
        let total_ups = self.snapshot.ups.len();
        let pinned_ups = self
            .snapshot
            .ups
            .iter()
            .filter(|u| u.is_pinned == Some(true))
            .count();
        let live_ups = self
            .snapshot
            .ups
            .iter()
            .filter(|u| u.live_room.as_ref().is_some_and(|r| r.live_status == 1))
            .count();
        let notify_live_ups = self
            .snapshot
            .ups
            .iter()
            .filter(|u| u.notify_on_live != Some(false))
            .count();

        let filtered_count = self.settings.filtered_up_indices.len();
        let list_state = self.settings.up_list.clone();
        let entity = cx.entity();
        let view_id = cx.entity_id();

        let kpi_strip = div()
            .flex()
            .flex_wrap()
            .gap_2p5()
            .child(ui::kpi_stat_card(
                AppIcon::Users,
                "特別關注總數",
                format!("{total_ups} 位"),
                false,
                cx,
            ))
            .child(ui::kpi_stat_card(
                AppIcon::Pin,
                "已置頂快捷",
                format!("{pinned_ups} 位"),
                false,
                cx,
            ))
            .child(ui::kpi_stat_card(
                AppIcon::Live,
                "目前正在直播",
                format!("{live_ups} 位"),
                live_ups > 0,
                cx,
            ))
            .child(ui::kpi_stat_card(
                AppIcon::Bell,
                "開播提醒啟用",
                format!("{notify_live_ups} 位"),
                false,
                cx,
            ));

        let action_row = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_3()
            .child(
                div()
                    .flex_1()
                    .min_w(px(280.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.settings.add)
                                .small()
                                .cleanable(true)
                                .aria_label("輸入 UID、空間網址、b23.tv 或 BV 號"),
                        ),
                    )
                    .child(
                        Button::new("add-up")
                            .primary()
                            .small()
                            .when(!busy, |button| button.icon(IconName::Plus))
                            .accessibility_label("新增追蹤")
                            .child(crate::ascii::button_label(
                                "add-up-wait",
                                if busy { "處理中…" } else { "新增追蹤" },
                                busy,
                            ))
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| this.add_up(cx))),
                    ),
            )
            .child(
                div()
                    .w(px(260.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.settings.up_filter)
                                .small()
                                .cleanable(true)
                                .aria_label("篩選已追蹤的 UP 主")
                                .prefix(
                                    Icon::new(IconName::Search)
                                        .small()
                                        .text_color(cx.theme().muted_foreground),
                                ),
                        ),
                    )
                    .when(filtered_count != total_ups, |d| {
                        d.child(
                            Tag::secondary()
                                .xsmall()
                                .child(format!("{filtered_count}/{total_ups}")),
                        )
                    }),
            );

        let mut header = div()
            .flex_shrink_0()
            .px_6()
            .pt_4()
            .pb_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .flex()
            .flex_col()
            .gap_3()
            .child(kpi_strip)
            .child(action_row);

        if let Some(error) = self.settings.form_error.clone() {
            let dismiss = Button::new("dismiss-up-form-error")
                .xsmall()
                .ghost()
                .icon(IconName::Close)
                .accessibility_label("關閉錯誤提示")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.settings.form_error = None;
                    cx.notify();
                }))
                .into_any_element();
            header = header.child(ui::notice_banner(Tone::Danger, &error, vec![dismiss], cx));
        }

        let list_body = if total_ups == 0 {
            ui::empty_state(
                AppIcon::UserPlus,
                "尚未追蹤任何 UP 主",
                "在上方輸入框貼上 B 站 UID 或個人空間網址，即可開始監控最新動態、投稿與直播。",
                None,
                cx,
            )
        } else if filtered_count == 0 {
            ui::empty_state(
                AppIcon::Sparkles,
                "找不到符合條件的 UP 主",
                "請調整或清除右上方的篩選關鍵字。",
                None,
                cx,
            )
        } else {
            let columns = self.settings.up_columns.max(1);
            smooth_list(
                "settings-ups-smooth-list",
                view_id,
                list_state,
                move |grid_row, window, cx| {
                    entity.update(cx, |this, cx| {
                        this.settings_up_grid_row(grid_row, columns, detail_width, window, cx)
                    })
                },
            )
            .into_any_element()
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .child(header)
            .child(list_body)
            .into_any_element()
    }

    #[inline(never)]
    fn settings_up_grid_row(
        &mut self,
        grid_row: usize,
        columns: usize,
        detail_width: f32,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let start = grid_row * columns;
        let slice: Vec<usize> = self
            .settings
            .filtered_up_indices
            .iter()
            .skip(start)
            .take(columns)
            .copied()
            .collect();
        if slice.is_empty() {
            return div().h(px(6.)).into_any_element();
        }
        let card_width = if columns > 1 {
            Some(
                ((detail_width - ui::PAGE_PADDING_X * 2.0 - 16.0 - 12.0 * (columns - 1) as f32)
                    / columns as f32)
                    .max(300.0),
            )
        } else {
            None
        };

        let mut row = div().px_6().py_1p5().flex().items_stretch().gap_3();
        for actual_index in slice {
            let card = self.render_single_up_card(actual_index, grid_row, card_width, cx);
            row = row.child(card);
        }
        ui::card_enter(row, ("up-grid-row", grid_row), grid_row % 4)
    }

    fn render_single_up_card(
        &mut self,
        actual_index: usize,
        grid_row: usize,
        fixed_width: Option<f32>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(up) = self.snapshot.ups.get(actual_index).cloned() else {
            return div().into_any_element();
        };
        let mid = up.mid;
        let busy = self.settings.busy;
        let pinned = up.is_pinned == Some(true);
        let is_live = up
            .live_room
            .as_ref()
            .is_some_and(|room| room.live_status == 1);
        let avatar = self.avatar(&up.face, ui::AVATAR_LG, cx);
        let space = format!("https://space.bilibili.com/{mid}");
        let edit_note = up.custom_note.clone().unwrap_or_default();
        let edit_tags = up.tags.clone().unwrap_or_default().join(", ");

        let mut name_row = div().flex().items_center().gap_1p5().min_w_0().child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .truncate()
                .child(up.name.clone()),
        );
        if up.level > 0 {
            name_row = name_row.child(Tag::secondary().xsmall().child(format!("Lv.{}", up.level)));
        }
        if pinned {
            name_row = name_row.child(Tag::primary().outline().xsmall().child("置頂"));
        }
        if is_live {
            name_row = name_row.child(ui::live_pill_badge(("up-card-live", mid), "直播中", cx));
        }

        let mut top_actions = div().flex().items_center().gap_0p5().flex_shrink_0();
        top_actions = top_actions
            .child(
                Button::new(("up-pin", mid))
                    .ghost()
                    .xsmall()
                    .selected(pinned)
                    .icon(AppIcon::Pin)
                    .tooltip(if pinned {
                        "取消置頂"
                    } else {
                        "置頂至側欄"
                    })
                    .accessibility_label(if pinned { "取消置頂" } else { "置頂" })
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.update_up(
                            mid,
                            UpPatch {
                                pinned: Some(!pinned),
                                ..Default::default()
                            },
                            cx,
                        )
                    })),
            )
            .child(
                Button::new(("up-edit", mid))
                    .ghost()
                    .xsmall()
                    .selected(self.settings.editing == Some(mid))
                    .label("備註")
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if this.settings.editing == Some(mid) {
                            this.settings.editing = None;
                        } else {
                            this.settings.editing = Some(mid);
                            this.settings.note.update(cx, |state, cx| {
                                state.set_value(edit_note.clone(), window, cx)
                            });
                            this.settings.tags.update(cx, |state, cx| {
                                state.set_value(edit_tags.clone(), window, cx)
                            });
                        }
                        this.settings.up_list.splice(grid_row..grid_row + 1, 1);
                        cx.notify();
                    })),
            )
            .child(
                Button::new(("up-space", mid))
                    .ghost()
                    .xsmall()
                    .icon(IconName::ExternalLink)
                    .tooltip("開啟個人空間")
                    .accessibility_label("開啟個人空間")
                    .on_click(cx.listener(move |this, _, _, cx| this.open_url(space.clone(), cx))),
            );

        if self.settings.deleting == Some(mid) {
            top_actions = top_actions
                .child(
                    Button::new(("up-remove-confirm", mid))
                        .danger()
                        .xsmall()
                        .label("確認")
                        .disabled(busy)
                        .on_click(cx.listener(move |this, _, _, cx| this.remove_up(mid, cx))),
                )
                .child(
                    Button::new(("up-remove-cancel", mid))
                        .outline()
                        .xsmall()
                        .label("取消")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.settings.deleting = None;
                            cx.notify();
                        })),
                );
        } else {
            top_actions = top_actions.child(
                Button::new(("up-remove", mid))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Delete)
                    .tooltip("移除關注")
                    .accessibility_label("移除")
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings.deleting = Some(mid);
                        cx.notify();
                    })),
            );
        }

        let hover_border = ui::hover_card_border(cx);
        let mut card = div()
            .when_some(fixed_width, |d, w| d.w(px(w)).flex_shrink_0())
            .when(fixed_width.is_none(), |d| d.flex_1().min_w_0())
            .p_3p5()
            .rounded_lg()
            .bg(cx.theme().muted)
            .border_1()
            .border_color(if pinned {
                ui::unread_card_border(cx)
            } else {
                cx.theme().border
            })
            .hover(move |s| s.border_color(hover_border))
            .flex()
            .flex_col()
            .justify_between()
            .gap_2p5()
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap_2p5()
                    .child(avatar)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .child(name_row)
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .truncate()
                                    .child(if up.sign.trim().is_empty() {
                                        format!("UID {mid}")
                                    } else {
                                        format!("UID {mid} · {}", up.sign.trim())
                                    }),
                            ),
                    )
                    .child(top_actions),
            );

        if self.settings.editing == Some(mid) {
            let editor = div()
                .p_2p5()
                .rounded_md()
                .bg(cx.theme().background)
                .border_1()
                .border_color(cx.theme().border)
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    Input::new(&self.settings.note)
                        .small()
                        .cleanable(true)
                        .aria_label("UP 主自訂備註"),
                )
                .child(
                    Input::new(&self.settings.tags)
                        .small()
                        .cleanable(true)
                        .aria_label("UP 主分類標籤"),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_1p5()
                        .child(
                            Button::new(("up-cancel", mid))
                                .outline()
                                .xsmall()
                                .label("取消")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.settings.editing = None;
                                    this.settings.up_list.splice(grid_row..grid_row + 1, 1);
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new(("up-save", mid))
                                .primary()
                                .xsmall()
                                .label("儲存")
                                .disabled(busy)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.commit_up_edit(mid, cx);
                                })),
                        ),
                );
            card = card.child(ui::enter_transition(editor, ("up-editor-open", mid)));
        } else {
            let note_text = up
                .custom_note
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let tags_list = up
                .tags
                .as_ref()
                .filter(|tags| !tags.is_empty())
                .cloned()
                .unwrap_or_default();
            if note_text.is_some() || !tags_list.is_empty() {
                let mut meta = div().flex().flex_wrap().items_center().gap_1p5();
                if let Some(note) = note_text {
                    meta = meta.child(Tag::primary().outline().xsmall().child(note.to_owned()));
                }
                for tag in tags_list {
                    meta = meta.child(Tag::secondary().xsmall().child(format!("#{tag}")));
                }
                card = card.child(meta);
            }
        }

        let live_on = up.notify_on_live != Some(false);
        let video_on = up.notify_on_video != Some(false);
        let dynamic_on = up.notify_on_dynamic != Some(false);

        let notify_bar = div()
            .pt_2()
            .border_t_1()
            .border_color(cx.theme().border)
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_2()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child("通知")
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3p5()
                    .child(
                        Switch::new(SharedString::from(format!("up-sw-live-{mid}")))
                            .xsmall()
                            .checked(live_on)
                            .label("開播")
                            .accessibility_label(format!("{} 開播通知", up.name))
                            .disabled(busy)
                            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                this.update_up(
                                    mid,
                                    UpPatch {
                                        notify_live: Some(*checked),
                                        ..Default::default()
                                    },
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Switch::new(SharedString::from(format!("up-sw-video-{mid}")))
                            .xsmall()
                            .checked(video_on)
                            .label("影片")
                            .accessibility_label(format!("{} 投稿影片通知", up.name))
                            .disabled(busy)
                            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                this.update_up(
                                    mid,
                                    UpPatch {
                                        notify_video: Some(*checked),
                                        ..Default::default()
                                    },
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Switch::new(SharedString::from(format!("up-sw-dynamic-{mid}")))
                            .xsmall()
                            .checked(dynamic_on)
                            .label("動態")
                            .accessibility_label(format!("{} 圖文動態通知", up.name))
                            .disabled(busy)
                            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                this.update_up(
                                    mid,
                                    UpPatch {
                                        notify_dynamic: Some(*checked),
                                        ..Default::default()
                                    },
                                    cx,
                                );
                            })),
                    ),
            );

        card.child(notify_bar).into_any_element()
    }

    #[inline(never)]
    fn render_groups(
        &mut self,
        detail_width: f32,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let busy = self.settings.busy;
        let selected = self
            .settings
            .group_index
            .min(self.settings.groups.len().saturating_sub(1));
        self.settings.group_index = selected;
        let groups_dirty = self.settings.is_groups_dirty(&self.snapshot, cx);
        let total_ups = self.snapshot.ups.len().max(1);
        let split_master_detail = detail_width >= 680.0;
        let view_id = cx.entity_id();

        let groups_len = self.settings.groups.len();
        let make_add_group_btn = |cx: &mut Context<Self>| {
            Button::new("new-group")
                .xsmall()
                .primary()
                .icon(IconName::Plus)
                .label("新增")
                .disabled(busy || groups_len >= ui::MAX_GROUPS)
                .on_click(cx.listener(|this, _, window, cx| {
                    let index = this.settings.groups.len();
                    let name = format!("新分組 {}", index + 1);
                    let source = FollowGroup {
                        id: format!("group-{}-{index}", bili_core::now_ms()),
                        name: name.clone(),
                        ..Default::default()
                    };
                    this.settings.groups.push(GroupDraft {
                        source,
                        name: input(&name, "分組名稱（1–24 字）", window, cx),
                    });
                    this.settings.group_index = index;
                    this.settings.delete_group = false;
                    this.sync_group_member_rows(true, cx);
                    cx.notify();
                }))
        };

        let group_cards = self
            .settings
            .groups
            .iter()
            .enumerate()
            .map(|(index, group)| {
                let name = value(&group.name, cx);
                let count = group.source.mids.len();
                let ratio = (count as f32 / total_ups as f32).clamp(0.0, 1.0);
                let is_sel = index == selected;
                let hover_bg = cx.theme().accent;

                div()
                    .id(("master-group-card", index))
                    .p_2p5()
                    .rounded_md()
                    .cursor_pointer()
                    .border_1()
                    .border_color(if is_sel {
                        ui::brand_accent(cx)
                    } else {
                        cx.theme().border
                    })
                    .bg(if is_sel {
                        ui::brand_soft_bg(cx)
                    } else {
                        cx.theme().muted
                    })
                    .when(!is_sel, move |d| d.hover(move |s| s.bg(hover_bg)))
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings.group_index = index;
                        this.settings.delete_group = false;
                        cx.notify();
                    }))
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
                                    .truncate()
                                    .text_sm()
                                    .font_weight(if is_sel {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::MEDIUM
                                    })
                                    .child(if name.trim().is_empty() {
                                        "未命名分組".into()
                                    } else {
                                        name
                                    }),
                            )
                            .child(Tag::secondary().xsmall().child(format!("{count} 位"))),
                    )
                    .child(
                        div()
                            .h(px(3.))
                            .w_full()
                            .rounded_full()
                            .bg(cx.theme().secondary)
                            .overflow_hidden()
                            .child(
                                div()
                                    .h_full()
                                    .w(relative(ratio))
                                    .rounded_full()
                                    .bg(ui::brand_accent(cx)),
                            ),
                    )
            })
            .collect::<Vec<_>>();

        let mut right_top = div()
            .flex_shrink_0()
            .px_5()
            .py_3p5()
            .border_b_1()
            .border_color(cx.theme().border)
            .flex()
            .flex_col()
            .gap_2p5();

        if !split_master_detail {
            right_top = right_top.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_1p5()
                    .children(
                        self.settings
                            .groups
                            .iter()
                            .enumerate()
                            .map(|(index, group)| {
                                Button::new(("compact-group-pill", index))
                                    .xsmall()
                                    .outline()
                                    .selected(index == selected)
                                    .label(format!(
                                        "{} ({})",
                                        value(&group.name, cx),
                                        group.source.mids.len()
                                    ))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.settings.group_index = index;
                                        this.settings.delete_group = false;
                                        cx.notify();
                                    }))
                            }),
                    )
                    .child(make_add_group_btn(cx)),
            );
        }

        if let Some(group) = self.settings.groups.get(selected) {
            let filtered_mids: Vec<u64> = self
                .settings
                .filtered_group_up_indices
                .iter()
                .filter_map(|&idx| self.snapshot.ups.get(idx).map(|u| u.mid))
                .collect();
            let filtered_for_select_all = filtered_mids.clone();

            let mut delete_controls = div().flex().items_center().gap_1p5();
            if self.settings.delete_group {
                delete_controls = delete_controls
                    .child(
                        Button::new("remove-group-confirm")
                            .danger()
                            .xsmall()
                            .label("確認刪除")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                let index = this.settings.group_index;
                                if index < this.settings.groups.len() {
                                    this.settings.groups.remove(index);
                                }
                                this.settings.group_index = 0;
                                this.settings.delete_group = false;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("remove-group-cancel")
                            .outline()
                            .xsmall()
                            .label("取消")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.settings.delete_group = false;
                                cx.notify();
                            })),
                    );
            } else {
                delete_controls = delete_controls.child(
                    Button::new("remove-group")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Delete)
                        .label("刪除分組")
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.settings.delete_group = true;
                            cx.notify();
                        })),
                );
            }

            right_top = right_top.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_2p5()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div().w(px(200.)).child(
                                    Input::new(&group.name)
                                        .small()
                                        .aria_label("目前選取的分組名稱"),
                                ),
                            )
                            .child(
                                Tag::primary()
                                    .outline()
                                    .small()
                                    .child(format!("已選 {} 位", group.source.mids.len())),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_1p5()
                            .child(
                                div().w(px(190.)).child(
                                    Input::new(&self.settings.group_member_filter)
                                        .small()
                                        .cleanable(true)
                                        .aria_label("篩選分組成員")
                                        .prefix(
                                            Icon::new(IconName::Search)
                                                .small()
                                                .text_color(cx.theme().muted_foreground),
                                        ),
                                ),
                            )
                            .child(
                                Button::new("group-select-filtered")
                                    .ghost()
                                    .xsmall()
                                    .label("全選顯示")
                                    .disabled(filtered_mids.is_empty())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(g) =
                                            this.settings.groups.get_mut(this.settings.group_index)
                                        {
                                            for mid in &filtered_for_select_all {
                                                if !g.source.mids.contains(mid) {
                                                    g.source.mids.push(*mid);
                                                }
                                            }
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("group-clear-all")
                                    .ghost()
                                    .xsmall()
                                    .label("清空")
                                    .disabled(group.source.mids.is_empty())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(g) =
                                            this.settings.groups.get_mut(this.settings.group_index)
                                        {
                                            g.source.mids.clear();
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(delete_controls),
                    ),
            );
        }

        let member_pane_width = detail_width - if split_master_detail { 224.0 } else { 0.0 };
        let member_cols = if member_pane_width >= 760.0 {
            3
        } else if member_pane_width >= 440.0 {
            2
        } else {
            1
        };
        if self.settings.group_columns != member_cols {
            self.settings.group_columns = member_cols;
            self.sync_group_member_rows(false, cx);
        }

        let member_area = if self.settings.groups.is_empty() {
            ui::empty_state(
                AppIcon::Users,
                "目前沒有任何分組",
                "點擊左側「新增」建立分組後，即可勾選成員並儲存。",
                None,
                cx,
            )
        } else if self.snapshot.ups.is_empty() {
            ui::empty_state(
                AppIcon::UserPlus,
                "尚無可加入分組的 UP 主",
                "請先在「UP 主管理」新增特別關注。",
                None,
                cx,
            )
        } else {
            let state = self.settings.group_list.clone();
            let entity = cx.entity();
            let tile_w = ((member_pane_width - 40.0 - 10.0 * (member_cols - 1) as f32)
                / member_cols as f32)
                .max(180.0);
            smooth_list(
                "settings-group-members-smooth-list",
                view_id,
                state,
                move |grid_row, _, cx| {
                    entity.update(cx, |this, cx| {
                        let indices: Vec<usize> = this
                            .settings
                            .filtered_group_up_indices
                            .iter()
                            .skip(grid_row * member_cols)
                            .take(member_cols)
                            .copied()
                            .collect();
                        if indices.is_empty() {
                            return div().h(px(6.)).into_any_element();
                        }
                        let mut row = div().px_5().py_1().flex().gap_2p5();
                        for actual_idx in indices {
                            let Some(up) = this.snapshot.ups.get(actual_idx).cloned() else {
                                continue;
                            };
                            let mid = up.mid;
                            let on = this
                                .settings
                                .groups
                                .get(this.settings.group_index)
                                .is_some_and(|group| group.source.mids.contains(&mid));
                            let avatar = this.avatar(&up.face, 30., cx);
                            let hover_border = ui::hover_card_border(cx);

                            row = row.child(
                                div()
                                    .id(("group-member-tile", mid))
                                    .w(px(tile_w))
                                    .flex_shrink_0()
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .border_1()
                                    .border_color(if on {
                                        ui::unread_card_border(cx)
                                    } else {
                                        cx.theme().border
                                    })
                                    .bg(if on {
                                        ui::brand_soft_bg(cx)
                                    } else {
                                        cx.theme().muted
                                    })
                                    .hover(move |s| s.border_color(hover_border))
                                    .flex()
                                    .items_center()
                                    .gap_2p5()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(group) =
                                            this.settings.groups.get_mut(this.settings.group_index)
                                        {
                                            if group.source.mids.contains(&mid) {
                                                group.source.mids.retain(|id| *id != mid);
                                            } else {
                                                group.source.mids.push(mid);
                                            }
                                        }
                                        cx.notify();
                                    }))
                                    .child(
                                        Checkbox::new(("group-member-cb", mid))
                                            .checked(on)
                                            .accessibility_label(format!("加入分組：{}", up.name)),
                                    )
                                    .child(avatar)
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .flex()
                                            .flex_col()
                                            .child(
                                                div()
                                                    .truncate()
                                                    .text_sm()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .child(up.name),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(format!("UID {mid}")),
                                            ),
                                    ),
                            );
                        }
                        row.into_any_element()
                    })
                },
            )
            .into_any_element()
        };

        let mut workspace = div().flex_1().min_h_0().min_w_0().flex();
        if split_master_detail {
            let sidebar_handle = self.settings.group_sidebar_scroll.clone();
            let master_list = smooth_scroll_container(
                "group-master-scroll",
                view_id,
                &sidebar_handle,
                div().p_3().flex().flex_col().gap_2().children(group_cards),
            );
            let master_col = div()
                .w(px(224.))
                .flex_shrink_0()
                .h_full()
                .border_r_1()
                .border_color(cx.theme().border)
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex_shrink_0()
                        .px_3p5()
                        .py_3()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("自訂分組 ({})", self.settings.groups.len())),
                        )
                        .child(make_add_group_btn(cx)),
                )
                .child(master_list);
            workspace = workspace.child(master_col);
        }
        workspace = workspace.child(
            div()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .flex()
                .flex_col()
                .child(right_top)
                .child(member_area),
        );

        let mut footer = div()
            .flex_shrink_0()
            .px_6()
            .py_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .flex()
            .flex_col()
            .gap_2();

        if let Some(error) = self.settings.form_error.clone() {
            footer = footer.child(ui::notice_banner(Tone::Danger, &error, Vec::new(), cx));
        }

        let warning_fg = ui::tone_colors(Tone::Warning, cx.theme().is_dark()).fg;
        footer = footer.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .text_xs()
                        .text_color(if groups_dirty {
                            warning_fg
                        } else {
                            cx.theme().muted_foreground
                        })
                        .child(if groups_dirty {
                            "● 分組有未儲存的變更"
                        } else {
                            "分組設定已與本機存檔同步 · 同一位 UP 主可同時歸屬多個分組"
                        }),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Button::new("revert-groups")
                                .outline()
                                .small()
                                .label("還原變更")
                                .disabled(!groups_dirty || busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.reset_groups_draft(window, cx);
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("save-groups")
                                .primary()
                                .small()
                                .accessibility_label("儲存分組")
                                .child(crate::ascii::button_label_with_style(
                                    "save-groups-wait",
                                    "儲存分組",
                                    busy,
                                    crate::ascii::Style::Working,
                                ))
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| this.save_groups(cx))),
                        ),
                ),
        );

        div()
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .child(workspace)
            .child(footer)
            .into_any_element()
    }

    #[inline(never)]
    fn render_settings_general(&mut self, detail_width: f32, cx: &mut Context<Self>) -> AnyElement {
        let busy = self.settings.busy;
        let general = self.settings.general.clone();
        let general_dirty = self.settings.is_general_dirty(&self.snapshot, cx);
        let live_val = value(&self.settings.live, cx);
        let dynamic_val = value(&self.settings.dynamic, cx);
        let proxy_val = value(&self.settings.proxy, cx);
        let validation = validate_general_inputs(&live_val, &dynamic_val, &proxy_val, &general);
        let two_col_bento = detail_width >= 720.0;

        // 1. 介面外觀卡（帶視覺化微縮視窗色票預覽）
        let theme_swatches =
            div()
                .flex()
                .gap_2p5()
                .children(THEME_MODES.into_iter().map(|(mode, label, icon)| {
                    let selected = general.theme == mode;
                    let aria = match mode {
                        "dark" => "深色主題",
                        "light" => "淺色主題",
                        _ => "跟隨系統外觀",
                    };
                    let (canvas_bg, card_bg, bar_bg) = match mode {
                        "dark" => (rgb(0x111318), rgb(0x1e2430), rgb(0xfb7299)),
                        "light" => (rgb(0xf4f6fa), rgb(0xffffff), rgb(0xd83b69)),
                        _ => (rgb(0x232834), rgb(0xf4f6fa), rgb(0xfb7299)),
                    };
                    let hover_border = ui::hover_card_border(cx);

                    Button::new(SharedString::from(format!("theme-swatch-{mode}")))
                        .ghost()
                        .selected(selected)
                        .toggled(selected)
                        .accessibility_label(aria)
                        .flex_1()
                        .h(px(88.))
                        .p_0()
                        .border_1()
                        .border_color(if selected {
                            ui::brand_accent(cx)
                        } else {
                            cx.theme().border
                        })
                        .bg(if selected {
                            ui::brand_soft_bg(cx)
                        } else {
                            cx.theme().background
                        })
                        .hover(move |s| s.border_color(hover_border))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.settings.general.theme = mode.into();
                            this.apply_theme(cx);
                            cx.notify();
                        }))
                        .child(
                            div()
                                .size_full()
                                .p_2p5()
                                .flex()
                                .flex_col()
                                .justify_between()
                                .child(
                                    div()
                                        .w_full()
                                        .h(px(36.))
                                        .rounded_sm()
                                        .overflow_hidden()
                                        .border_1()
                                        .border_color(cx.theme().border)
                                        .bg(canvas_bg)
                                        .p_1p5()
                                        .flex()
                                        .items_center()
                                        .gap_1p5()
                                        .child(div().w(px(6.)).h_full().rounded_full().bg(bar_bg))
                                        .child(div().flex_1().h_full().rounded_xs().bg(card_bg)),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .gap_1()
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap_1p5()
                                                .text_xs()
                                                .font_weight(if selected {
                                                    FontWeight::SEMIBOLD
                                                } else {
                                                    FontWeight::MEDIUM
                                                })
                                                .child(Icon::new(icon).with_size(px(13.)))
                                                .child(label),
                                        )
                                        .when(selected, |d| {
                                            d.child(
                                                Icon::new(IconName::Check)
                                                    .with_size(px(13.))
                                                    .text_color(ui::brand_accent(cx)),
                                            )
                                        }),
                                ),
                        )
                }));

        let appearance_card = ui::section_card(
            "介面外觀",
            Some("點擊色票立即預覽全視窗配色，儲存後永久套用"),
            cx,
        )
        .flex_1()
        .min_w(px(300.))
        .child(theme_swatches);

        // 2. 背景輪詢間隔卡（附常用秒數快捷膠囊）
        let live_presets = div().flex().items_center().gap_1().children(
            [("30", "30s"), ("60", "60s"), ("120", "2m")]
                .into_iter()
                .map(|(sec, text)| {
                    let active = live_val.trim() == sec;
                    Button::new(SharedString::from(format!("preset-live-{sec}")))
                        .xsmall()
                        .ghost()
                        .selected(active)
                        .label(text)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.settings
                                .live
                                .update(cx, |s, cx| s.set_value(sec, window, cx));
                            cx.notify();
                        }))
                }),
        );
        let dynamic_presets = div().flex().items_center().gap_1().children(
            [("60", "1m"), ("300", "5m"), ("600", "10m")]
                .into_iter()
                .map(|(sec, text)| {
                    let active = dynamic_val.trim() == sec;
                    Button::new(SharedString::from(format!("preset-dyn-{sec}")))
                        .xsmall()
                        .ghost()
                        .selected(active)
                        .label(text)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.settings
                                .dynamic
                                .update(cx, |s, cx| s.set_value(sec, window, cx));
                            cx.notify();
                        }))
                }),
        );

        let polling_card = ui::section_card(
            "背景輪詢週期",
            Some("縮小至系統托盤時仍持續依此間隔背景檢查"),
            cx,
        )
        .flex_1()
        .min_w(px(300.))
        .child(
            div()
                .flex()
                .gap_3()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_1p5()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child("直播狀態"),
                                )
                                .child(live_presets),
                        )
                        .child(
                            Input::new(&self.settings.live)
                                .small()
                                .aria_label("直播輪詢間隔秒數")
                                .suffix(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("秒"),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(if validation.live_error.is_some() {
                                    ui::tone_colors(Tone::Danger, cx.theme().is_dark()).fg
                                } else {
                                    cx.theme().muted_foreground
                                })
                                .child(
                                    validation
                                        .live_error
                                        .clone()
                                        .unwrap_or_else(|| "最少 30 秒".into()),
                                ),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_1p5()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child("動態與投稿"),
                                )
                                .child(dynamic_presets),
                        )
                        .child(
                            Input::new(&self.settings.dynamic)
                                .small()
                                .aria_label("動態輪詢間隔秒數")
                                .suffix(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("秒"),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(if validation.dynamic_error.is_some() {
                                    ui::tone_colors(Tone::Danger, cx.theme().is_dark()).fg
                                } else {
                                    cx.theme().muted_foreground
                                })
                                .child(
                                    validation
                                        .dynamic_error
                                        .clone()
                                        .unwrap_or_else(|| "最少 60 秒".into()),
                                ),
                        ),
                ),
        );

        // 3. 系統通知與音效（頂部總開關 + 2×2 互動磁貼網格）
        type ToggleHandler = Box<dyn Fn(&mut BiliApp, bool, &mut Context<BiliApp>)>;
        let master_on = general.enable_notifications;
        let hover_border = ui::hover_card_border(cx);

        let make_tile = |tile_id: &'static str,
                         sw_id: &'static str,
                         icon: AppIcon,
                         title: &'static str,
                         desc: &'static str,
                         checked: bool,
                         handler: ToggleHandler,
                         cx: &mut Context<Self>| {
            let handler_rc = std::rc::Rc::new(handler);
            let h_tile = handler_rc.clone();
            let h_sw = handler_rc;
            let tile_click = cx.listener(move |this, _, _, cx| {
                h_tile(this, !checked, cx);
            });
            let sw_click = cx.listener(move |this, next: &bool, _, cx| {
                h_sw(this, *next, cx);
            });
            div()
                .id(tile_id)
                .flex_1()
                .min_w(px(180.))
                .p_3()
                .rounded_md()
                .border_1()
                .border_color(if checked && master_on {
                    ui::unread_card_border(cx)
                } else {
                    cx.theme().border
                })
                .bg(if checked && master_on {
                    ui::brand_soft_bg(cx)
                } else {
                    cx.theme().background
                })
                .when(master_on, move |d| {
                    d.cursor_pointer()
                        .hover(move |s| s.border_color(hover_border))
                        .on_click(tile_click)
                })
                .when(!master_on, |d| d.opacity(0.5))
                .flex()
                .items_center()
                .justify_between()
                .gap_2p5()
                .child(
                    div()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap_2p5()
                        .child(Icon::new(icon).with_size(px(16.)).text_color(
                            if checked && master_on {
                                ui::brand_accent(cx)
                            } else {
                                cx.theme().muted_foreground
                            },
                        ))
                        .child(
                            div()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::MEDIUM)
                                        .truncate()
                                        .child(title),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .truncate()
                                        .child(desc),
                                ),
                        ),
                )
                .child(
                    Switch::new(sw_id)
                        .small()
                        .checked(checked)
                        .accessibility_label(title)
                        .disabled(!master_on)
                        .on_click(sw_click),
                )
        };

        let tile_live = make_tile(
            "tile-notify-live",
            "sw-notify-live",
            AppIcon::Live,
            "開播通知",
            "UP 主開始直播時立即推播",
            general.notify_live,
            Box::new(|this, next, cx| {
                this.settings.general.notify_live = next;
                cx.notify();
            }),
            cx,
        );
        let tile_video = make_tile(
            "tile-notify-video",
            "sw-notify-video",
            AppIcon::Video,
            "投稿影片通知",
            "發布新影片投稿時提醒",
            general.notify_video,
            Box::new(|this, next, cx| {
                this.settings.general.notify_video = next;
                cx.notify();
            }),
            cx,
        );
        let tile_dynamic = make_tile(
            "tile-notify-dynamic",
            "sw-notify-dynamic",
            AppIcon::Feed,
            "圖文動態通知",
            "發布圖文、專欄或文字動態",
            general.notify_dynamic,
            Box::new(|this, next, cx| {
                this.settings.general.notify_dynamic = next;
                cx.notify();
            }),
            cx,
        );
        let tile_sound = make_tile(
            "tile-sound-enabled",
            "sw-sound-enabled",
            AppIcon::Bell,
            "通知提示音效",
            "搭配系統預設通知提示音",
            general.sound_enabled,
            Box::new(|this, next, cx| {
                this.settings.general.sound_enabled = next;
                cx.notify();
            }),
            cx,
        );

        let notifications_card = div()
            .flex_shrink_0()
            .flex_grow(3.0)
            .flex_basis(px(420.))
            .p_5()
            .rounded_lg()
            .bg(cx.theme().muted)
            .border_1()
            .border_color(cx.theme().border)
            .flex()
            .flex_col()
            .gap_3p5()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("系統通知與音效"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("透過 Windows 通知中心推播；可單擊磁貼快速切換"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2p5()
                            .child(
                                Button::new("test-notification")
                                    .outline()
                                    .xsmall()
                                    .icon(AppIcon::Bell)
                                    .label("測試通知")
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.test_notification(cx)),
                                    ),
                            )
                            .child(
                                Switch::new("sw-enable-notifications")
                                    .small()
                                    .checked(master_on)
                                    .label("總開關")
                                    .accessibility_label("啟用系統通知")
                                    .on_click(cx.listener(|this, next: &bool, _, cx| {
                                        this.settings.general.enable_notifications = *next;
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2p5()
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2p5()
                            .child(tile_live)
                            .child(tile_video),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2p5()
                            .child(tile_dynamic)
                            .child(tile_sound),
                    ),
            );

        // 4. 網路代理與連線狀態卡
        let has_proxy = !proxy_val.trim().is_empty();
        let proxy_card = div()
            .flex_shrink_0()
            .flex_grow(2.0)
            .flex_basis(px(280.))
            .p_5()
            .rounded_lg()
            .bg(cx.theme().muted)
            .border_1()
            .border_color(cx.theme().border)
            .flex()
            .flex_col()
            .justify_between()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("網路代理"),
                            )
                            .child(if has_proxy {
                                Tag::primary().outline().xsmall().child("自訂代理")
                            } else {
                                Tag::secondary().xsmall().child("系統預設／直連")
                            }),
                    )
                    .child(ui::form_field(
                        "代理伺服器網址（重啟後套用）",
                        Some("支援 http://、https://、socks5://"),
                        validation.proxy_error.as_deref(),
                        Input::new(&self.settings.proxy)
                            .small()
                            .cleanable(true)
                            .aria_label("自訂網路代理伺服器網址"),
                        cx,
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("留空即使用系統環境代理")
                    .when(has_proxy, |d| {
                        d.child(
                            Button::new("clear-proxy")
                                .ghost()
                                .xsmall()
                                .label("改回直連")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.settings
                                        .proxy
                                        .update(cx, |s, cx| s.set_value("", window, cx));
                                    cx.notify();
                                })),
                        )
                    }),
            );

        let bento_top = if two_col_bento {
            div()
                .flex()
                .items_stretch()
                .gap_4()
                .child(appearance_card)
                .child(polling_card)
        } else {
            div()
                .flex()
                .flex_col()
                .gap_4()
                .child(appearance_card)
                .child(polling_card)
        };

        let bento_bottom = if two_col_bento {
            div()
                .flex()
                .items_stretch()
                .gap_4()
                .child(notifications_card)
                .child(proxy_card)
        } else {
            div()
                .flex()
                .flex_col()
                .gap_4()
                .child(notifications_card)
                .child(proxy_card)
        };

        let mut footer = div()
            .flex_shrink_0()
            .px_6()
            .py_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .flex()
            .flex_col()
            .gap_2();

        if let Some(error) = self.settings.form_error.clone() {
            footer = footer.child(ui::notice_banner(Tone::Danger, &error, Vec::new(), cx));
        }

        let warning_fg = ui::tone_colors(Tone::Warning, cx.theme().is_dark()).fg;
        let can_save = !busy && validation.parsed.is_some();

        footer = footer.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .text_xs()
                        .text_color(if general_dirty {
                            warning_fg
                        } else {
                            cx.theme().muted_foreground
                        })
                        .child(if general_dirty {
                            "● 有未儲存的設定變更"
                        } else {
                            "所有輪詢與通知設定皆已儲存"
                        }),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Button::new("revert-settings")
                                .outline()
                                .small()
                                .label("還原")
                                .disabled(!general_dirty || busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.reset_general_draft(window, cx);
                                    this.apply_theme(cx);
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("save-settings")
                                .primary()
                                .small()
                                .accessibility_label("儲存設定")
                                .child(crate::ascii::button_label_with_style(
                                    "save-settings-wait",
                                    "儲存設定",
                                    busy,
                                    crate::ascii::Style::Working,
                                ))
                                .disabled(!can_save)
                                .on_click(cx.listener(|this, _, _, cx| this.save_general(cx))),
                        ),
                ),
        );

        let scroll_handle = self.settings.general_scroll.clone();
        let scroll_area = smooth_scroll_container(
            "settings-general-scroll",
            cx.entity_id(),
            &scroll_handle,
            div()
                .p_6()
                .flex()
                .flex_col()
                .gap_4()
                .child(bento_top)
                .child(bento_bottom),
        );

        div()
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .child(scroll_area)
            .child(footer)
            .into_any_element()
    }

    #[inline(never)]
    fn render_settings_backup(&mut self, detail_width: f32, cx: &mut Context<Self>) -> AnyElement {
        let busy = self.settings.busy;
        let two_col = detail_width >= 700.0;
        let ups_len = self.snapshot.ups.len();
        let groups_len = self.groups().len();
        let read_len = self.snapshot.read_dynamics.len();
        let poll_summary = format!(
            "{}s / {}s",
            self.snapshot.settings.poll_interval_live_sec,
            self.snapshot.settings.poll_interval_dynamic_sec
        );

        let kpi_row = div()
            .flex()
            .flex_wrap()
            .gap_3()
            .child(ui::kpi_stat_card(
                AppIcon::UserPlus,
                "關注名單",
                format!("{ups_len} 位"),
                false,
                cx,
            ))
            .child(ui::kpi_stat_card(
                AppIcon::Users,
                "自訂分組",
                format!("{groups_len} 組"),
                false,
                cx,
            ))
            .child(ui::kpi_stat_card(
                AppIcon::CheckRead,
                "已讀動態紀錄",
                format!("{read_len} 則"),
                false,
                cx,
            ))
            .child(ui::kpi_stat_card(
                AppIcon::Bell,
                "直播／動態輪詢",
                poll_summary,
                false,
                cx,
            ));

        let export_card = ui::section_card(
            "匯出完整備份",
            Some("產生與舊版完全相容的單一 JSON 備份檔，適合定期封存或跨電腦遷移。"),
            cx,
        )
        .flex_1()
        .justify_between()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1p5()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("✓ 關注 UP 主名單、自訂備註、標籤與置頂順序")
                .child("✓ 自訂關注分組與成員對應")
                .child("✓ 輪詢週期、通知開關、主題與網路代理設定")
                .child("✓ 已讀動態識別碼紀錄（最多 5,000 筆）"),
        )
        .child(
            ui::button_row().child(
                Button::new("export-backup")
                    .primary()
                    .icon(AppIcon::Database)
                    .label("匯出完整備份")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, _, cx| this.export_backup(cx))),
            ),
        );

        let mut import_card = ui::section_card(
            "從備份檔還原",
            Some("採用原子寫入與自動回滾保護；選擇檔案後會先預覽人數，確認後才覆蓋。"),
            cx,
        )
        .flex_1()
        .justify_between()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1p5()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("• 支援 32 MiB 以內之標準 JSON 備份檔")
                .child("• 若寫入中途發生異常，啟動時會自動由 rollback 日誌復原")
                .child("• 還原後將立即同步更新側欄、分組與背景輪詢器"),
        );

        if let Some(data) = &self.settings.pending_import {
            let count = data["ups"]
                .as_array()
                .map_or(self.snapshot.ups.len(), Vec::len);
            let warn = ui::tone_colors(Tone::Warning, cx.theme().is_dark());
            import_card = import_card.child(ui::enter_transition(
                div()
                    .p_3p5()
                    .rounded_md()
                    .bg(warn.bg)
                    .border_1()
                    .border_color(warn.border)
                    .flex()
                    .flex_col()
                    .gap_2p5()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(warn.fg)
                            .child(format!(
                                "已讀取備份檔（含 {count} 位 UP 主），確認後將完整取代目前資料。"
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap_2()
                            .child(
                                Button::new("cancel-import")
                                    .outline()
                                    .xsmall()
                                    .label("取消")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.settings.pending_import = None;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("confirm-import")
                                    .danger()
                                    .xsmall()
                                    .accessibility_label("確認覆蓋還原")
                                    .child(crate::ascii::button_label_with_style(
                                        "import-backup-wait",
                                        "確認覆蓋還原",
                                        busy,
                                        crate::ascii::Style::Working,
                                    ))
                                    .disabled(busy)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.confirm_import(cx)),
                                    ),
                            ),
                    ),
                "pending-import-confirm",
            ));
        } else {
            import_card = import_card.child(
                ui::button_row().child(
                    Button::new("import-backup")
                        .outline()
                        .icon(AppIcon::FolderOpen)
                        .label("選擇備份檔還原")
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| this.choose_import(cx))),
                ),
            );
        }

        let action_grid = if two_col {
            div()
                .flex()
                .items_stretch()
                .gap_4()
                .child(export_card)
                .child(import_card)
        } else {
            div()
                .flex()
                .flex_col()
                .gap_4()
                .child(export_card)
                .child(import_card)
        };

        let mut inner = div().p_6().flex().flex_col().gap_4();
        if let Some(error) = self.settings.form_error.clone() {
            inner = inner.child(ui::notice_banner(Tone::Danger, &error, Vec::new(), cx));
        }
        inner = inner.child(kpi_row).child(action_grid);

        if let Some(services) = &self.services {
            let dir = services.store.directory().to_path_buf();
            let raw_display = dir.display().to_string();
            let dir_display = raw_display
                .strip_prefix(r"\\?\")
                .unwrap_or(&raw_display)
                .to_owned();
            inner = inner.child(
                ui::section_card(
                    "本機資料與快取目錄",
                    Some("所有 JSON 設定檔、圖片磁碟快取（上限 256 MiB）與單例鎖定檔皆位於此資料夾。"),
                    cx,
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .p_3()
                        .rounded_md()
                        .bg(cx.theme().background)
                        .border_1()
                        .border_color(cx.theme().border)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(dir_display),
                        )
                        .child(
                            Button::new("open-data-dir")
                                .outline()
                                .small()
                                .icon(AppIcon::FolderOpen)
                                .label("開啟資料目錄")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.open_local_folder(dir.clone(), cx);
                                })),
                        ),
                ),
            );
        }

        let backup_handle = self.settings.backup_scroll.clone();
        smooth_scroll_container(
            "settings-backup-scroll",
            cx.entity_id(),
            &backup_handle,
            inner,
        )
        .size_full()
        .into_any_element()
    }

    fn export_backup(&mut self, cx: &mut Context<Self>) {
        if self.settings.busy {
            return;
        }
        let Some(services) = self.services.clone() else {
            return;
        };
        let directory = dirs::document_dir().unwrap_or_else(|| services.store.directory().into());
        let name = format!(
            "{APP_NAME}-backup-{}.json",
            chrono::Local::now().format("%Y-%m-%d")
        );
        let dialog = cx.prompt_for_new_path(&directory, Some(&name));
        cx.spawn(async move |weak, cx| match dialog.await {
            Ok(Ok(Some(path))) => {
                let saved_path = path.clone();
                let _ = weak.update(cx, |this, cx| {
                    this.settings.busy = true;
                    this.settings.form_error = None;
                    this.run(
                        async move {
                            let data = services.store.export_backup().await?;
                            tokio::task::spawn_blocking(move || {
                                bili_core::store::atomic_write(&path, &data)
                            })
                            .await??;
                            Ok(saved_path)
                        },
                        cx,
                        |this, result, cx| {
                            this.settings.busy = false;
                            match result {
                                Ok(written) => {
                                    let folder = written.parent().map(std::path::Path::to_path_buf);
                                    this.show_toast(
                                        ToastNotice {
                                            tone: Tone::Success,
                                            title: "完整備份已匯出".into(),
                                            detail: Some(written.display().to_string()),
                                            folder,
                                        },
                                        cx,
                                    );
                                }
                                Err(error) => this.settings.form_error = Some(error.to_string()),
                            }
                            cx.notify();
                        },
                    );
                });
            }
            Ok(Ok(None)) => {}
            _ => {
                let _ = weak.update(cx, |this, cx| {
                    this.settings.form_error = Some("無法開啟備份儲存對話框".into());
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn choose_import(&mut self, cx: &mut Context<Self>) {
        if self.settings.busy {
            return;
        }
        let dialog = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(format!("選擇 {APP_NAME} JSON 備份").into()),
        });
        cx.spawn(async move |weak, cx| match dialog.await {
            Ok(Ok(Some(paths))) => {
                if let Some(path) = paths.into_iter().next() {
                    let _ = weak.update(cx, |this, cx| {
                        this.settings.form_error = None;
                        this.run(
                            async move {
                                tokio::task::spawn_blocking(move || -> Result<Value> {
                                    anyhow::ensure!(
                                        std::fs::metadata(&path)?.len() <= 32 * 1024 * 1024,
                                        "備份超過 32 MiB 安全上限"
                                    );
                                    let data: Value =
                                        serde_json::from_slice(&std::fs::read(path)?)?;
                                    anyhow::ensure!(
                                        data.is_object()
                                            && (data.get("ups").is_some()
                                                || data.get("settings").is_some()),
                                        "不是有效的 {APP_NAME} 備份"
                                    );
                                    Ok(data)
                                })
                                .await?
                            },
                            cx,
                            |this, result, cx| {
                                match result {
                                    Ok(data) => this.settings.pending_import = Some(data),
                                    Err(error) => {
                                        this.settings.form_error = Some(error.to_string())
                                    }
                                }
                                cx.notify();
                            },
                        );
                    });
                }
            }
            Ok(Ok(None)) => {}
            _ => {
                let _ = weak.update(cx, |this, cx| {
                    this.settings.form_error = Some("無法開啟備份選擇對話框".into());
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn confirm_import(&mut self, cx: &mut Context<Self>) {
        let Some(services) = self.services.clone() else {
            return;
        };
        if self.settings.busy {
            return;
        }
        let Some(data) = self.settings.pending_import.clone() else {
            return;
        };
        self.settings.busy = true;
        self.settings.form_error = None;
        self.run(
            async move {
                let count = services.import_backup(data).await?;
                Ok((count, services.snapshot().await?))
            },
            cx,
            |this, result, cx| {
                this.settings.busy = false;
                match result {
                    Ok((count, snapshot)) => {
                        this.settings.pending_import = None;
                        this.settings.initialized = false;
                        this.accepted_snapshot(snapshot, cx);
                        let handle = this.window;
                        let _ =
                            handle.update(cx, |_, window, cx| this.prepare_settings(window, cx));
                        this.show_notice(format!("已還原 {count} 位 UP 主與備份設定"), cx);
                    }
                    Err(error) => this.settings.form_error = Some(error.to_string()),
                }
                cx.notify();
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn general_patch_matches_core_app_settings_schema() {
        let draft = AppSettings {
            enable_notifications: false,
            notify_live: true,
            notify_video: false,
            notify_dynamic: true,
            sound_enabled: false,
            theme: "light".into(),
            ..Default::default()
        };

        let validation = validate_general_inputs("45", "180", "http://127.0.0.1:7890", &draft);
        assert!(validation.live_error.is_none());
        assert!(validation.dynamic_error.is_none());
        assert!(validation.proxy_error.is_none());

        let patch = validation.parsed.expect("valid patch").into_json_patch();
        assert!(patch.get("followGroups").is_none());

        let mut merged = serde_json::to_value(AppSettings::default()).unwrap();
        merged
            .as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        let roundtrip: AppSettings =
            serde_json::from_value(merged).expect("must deserialize into AppSettings");
        assert_eq!(roundtrip.poll_interval_live_sec, 45);
        assert_eq!(roundtrip.poll_interval_dynamic_sec, 180);
        assert!(!roundtrip.enable_notifications);
        assert!(!roundtrip.notify_video);
        assert!(!roundtrip.sound_enabled);
        assert_eq!(roundtrip.theme, "light");
        assert_eq!(
            roundtrip.custom_proxy_url.as_deref(),
            Some("http://127.0.0.1:7890")
        );
    }

    #[test]
    fn general_validation_rejects_invalid_ranges_and_proxy() {
        let draft = AppSettings::default();

        let too_short = validate_general_inputs("15", "30", "not-a-url", &draft);
        assert!(too_short.parsed.is_none());
        assert!(too_short.live_error.is_some());
        assert!(too_short.dynamic_error.is_some());
        assert!(too_short.proxy_error.is_some());

        let non_numeric = validate_general_inputs("abc", "300", "", &draft);
        assert!(non_numeric.parsed.is_none());
        assert!(non_numeric.live_error.is_some());
        assert!(non_numeric.dynamic_error.is_none());
        assert!(non_numeric.proxy_error.is_none());
    }
}
