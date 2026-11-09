use super::*;
use crate::ui::{self, Tone};
use gpui::component::{
    input::InputState,
    select::{SearchableVec, SelectItem, SelectState},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Choice {
    pub value: String,
    pub label: SharedString,
}

impl SelectItem for Choice {
    type Value = String;
    fn title(&self) -> SharedString {
        self.label.clone()
    }
    fn value(&self) -> &String {
        &self.value
    }
}

pub(super) type Selector = Entity<SelectState<SearchableVec<Choice>>>;

pub(super) fn selector(items: Vec<Choice>, window: &mut Window, cx: &mut App) -> Selector {
    cx.new(|cx| SelectState::new(SearchableVec::new(items), None, window, cx).searchable(true))
}

pub(super) fn input(
    value: &str,
    placeholder: &str,
    window: &mut Window,
    cx: &mut App,
) -> Entity<InputState> {
    cx.new(|cx| {
        let mut state = InputState::new(window, cx).placeholder(placeholder.to_owned());
        state.set_value(value.to_owned(), window, cx);
        state
    })
}

pub(super) fn value(input: &Entity<InputState>, cx: &App) -> String {
    input.read(cx).value().to_string()
}

pub(super) fn up_choices(ups: Vec<UpUser>) -> Vec<Choice> {
    std::iter::once(Choice {
        value: String::new(),
        label: "全部 UP 主".into(),
    })
    .chain(ups.into_iter().map(|up| {
        let label = match up.custom_note.as_deref().map(str::trim) {
            Some(note) if !note.is_empty() => {
                format!("{}（{}）· {}", up.name, note, up.mid)
            }
            _ => format!("{} · {}", up.name, up.mid),
        };
        Choice {
            value: up.mid.to_string(),
            label: label.into(),
        }
    }))
    .collect()
}

pub(super) fn group_choices(groups: Vec<FollowGroup>) -> Vec<Choice> {
    std::iter::once(Choice {
        value: "all".into(),
        label: "全部".into(),
    })
    .chain(groups.into_iter().map(|group| Choice {
        value: group.id,
        label: group.name.into(),
    }))
    .chain(std::iter::once(Choice {
        value: "ungrouped".into(),
        label: "未分組".into(),
    }))
    .collect()
}

pub(super) fn format_time(timestamp: u64) -> String {
    chrono::DateTime::from_timestamp_millis(timestamp as i64)
        .filter(|_| timestamp > 0)
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%m/%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "時間未知".into())
}

pub(super) fn gui_image(image: &bili_core::images::CachedImage) -> Arc<Image> {
    let format = match image.kind {
        ImageKind::Png => ImageFormat::Png,
        ImageKind::Jpeg => ImageFormat::Jpeg,
        ImageKind::Webp => ImageFormat::Webp,
        ImageKind::Gif => ImageFormat::Gif,
    };
    Arc::new(Image::from_bytes(format, image.bytes.to_vec()))
}

#[derive(PartialEq, Eq)]
struct UpSelectorFingerprint<'a> {
    mid: u64,
    name: &'a str,
    face: &'a str,
    note: Option<&'a str>,
    tags: Option<&'a [String]>,
    pinned: Option<bool>,
    pin_order: Option<i64>,
}

/// 比對影響下拉選單、置頂列、頭像與備註標籤的完整指紋，避免僅比對 `(mid, name)` 漏同步。
pub(super) fn snapshot_selector_changed(old: &Snapshot, new: &Snapshot) -> bool {
    fn fingerprint(snap: &Snapshot) -> Vec<UpSelectorFingerprint<'_>> {
        snap.ups
            .iter()
            .map(|up| UpSelectorFingerprint {
                mid: up.mid,
                name: up.name.as_str(),
                face: up.face.as_str(),
                note: up.custom_note.as_deref(),
                tags: up.tags.as_deref(),
                pinned: up.is_pinned,
                pin_order: up.pin_order,
            })
            .collect()
    }
    fingerprint(old) != fingerprint(new) || old.settings.follow_groups != new.settings.follow_groups
}

/// 僅當追蹤名單 UID 集合或分組定義變動時才重新抓取遠端動態／影片，避免改備註或置頂時重置捲動位置。
pub(super) fn snapshot_scope_changed(old: &Snapshot, new: &Snapshot) -> bool {
    let mids = |snap: &Snapshot| -> Vec<u64> {
        let mut ids: Vec<u64> = snap.ups.iter().map(|up| up.mid).collect();
        ids.sort_unstable();
        ids
    };
    mids(old) != mids(new) || old.settings.follow_groups != new.settings.follow_groups
}

impl BiliApp {
    pub(super) fn active_theme_mode(&self) -> &str {
        if self.settings.initialized {
            self.settings.general.theme.as_str()
        } else {
            self.snapshot.settings.theme.as_str()
        }
    }

    pub(super) fn apply_theme(&self, cx: &mut App) {
        ui::apply_brand_theme(self.active_theme_mode(), cx);
        ui::apply_appearance(&self.snapshot.settings.appearance, cx);
    }

    pub(super) fn switch_view(&mut self, view: View, window: &mut Window, cx: &mut Context<Self>) {
        if view != View::Live {
            self.stop_live_players(window);
        }
        if self.view != view {
            self.view_epoch = self.view_epoch.wrapping_add(1);
            self.sidebar_scroll.set_offset(Point::default());
        }
        self.view = view;
        match view {
            View::Feed => {
                if !self.feeds[0].initialized {
                    self.load_feed(0, true, cx);
                }
                if self.dual && !self.feeds[1].initialized {
                    self.load_feed(1, true, cx);
                }
            }
            View::Videos => {
                if !self.videos.initialized {
                    self.load_videos(true, cx);
                }
            }
            View::Settings => self.prepare_settings(window, cx),
            View::Live => {}
        }
        cx.notify();
    }

    pub(super) fn select_group(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_group == id {
            return;
        }
        self.view_epoch = self.view_epoch.wrapping_add(1);
        self.selected_group = id.clone();
        self.feeds[0].group = id;
        self.feeds[0].mid = None;
        self.videos.mid = None;
        self.sync_selectors(window, cx);
        if self.view == View::Feed {
            self.load_feed(0, true, cx);
        }
        if self.view == View::Videos {
            self.load_videos(true, cx);
        }
        cx.notify();
    }

    pub(super) fn sync_selectors(&mut self, window: &mut Window, cx: &mut App) {
        let groups = self.groups();
        let all_group_choices = group_choices(groups.clone());

        for stream in &mut self.feeds {
            if !["all", "ungrouped"].contains(&stream.group.as_str())
                && !groups.iter().any(|group| group.id == stream.group)
            {
                stream.group = "all".into();
            }
            let options = up_choices(
                group_members(&self.snapshot.ups, &groups, &stream.group)
                    .into_iter()
                    .cloned()
                    .collect(),
            );
            let selected = stream.mid.map(|mid| mid.to_string()).unwrap_or_default();
            let index = options
                .iter()
                .position(|item| item.value == selected)
                .unwrap_or(0);
            if index == 0 {
                stream.mid = None;
            }
            let items_changed = stream.creator_choices != options;
            if items_changed {
                stream.creator_choices = options.clone();
            }
            stream.creator.update(cx, |state, cx| {
                if items_changed {
                    state.set_items(SearchableVec::new(options), window, cx);
                }
                state.set_selected_index(Some(IndexPath::default().row(index)), window, cx);
            });

            let group_index = all_group_choices
                .iter()
                .position(|item| item.value == stream.group)
                .unwrap_or(0);
            let group_items_changed = stream.group_choices != all_group_choices;
            if group_items_changed {
                stream.group_choices = all_group_choices.clone();
            }
            let next_groups = all_group_choices.clone();
            stream.groups.update(cx, |state, cx| {
                if group_items_changed {
                    state.set_items(SearchableVec::new(next_groups), window, cx);
                }
                state.set_selected_index(Some(IndexPath::default().row(group_index)), window, cx);
            });
        }

        let video_options = up_choices(self.scoped_ups());
        let selected_video_mid = self
            .videos
            .mid
            .map(|mid| mid.to_string())
            .unwrap_or_default();
        let video_index = video_options
            .iter()
            .position(|item| item.value == selected_video_mid)
            .unwrap_or(0);
        if video_index == 0 {
            self.videos.mid = None;
        }
        let video_items_changed = self.videos.creator_choices != video_options;
        if video_items_changed {
            self.videos.creator_choices = video_options.clone();
        }
        self.videos.creator.update(cx, |state, cx| {
            if video_items_changed {
                state.set_items(SearchableVec::new(video_options), window, cx);
            }
            state.set_selected_index(Some(IndexPath::default().row(video_index)), window, cx);
        });
    }

    pub(super) fn accepted_snapshot(&mut self, snapshot: Snapshot, cx: &mut Context<Self>) {
        let reset_general = self.settings.initialized
            && self.snapshot.settings != snapshot.settings
            && !self.settings.is_general_dirty(&self.snapshot, cx);
        let selector_changed = snapshot_selector_changed(&self.snapshot, &snapshot);
        let scope_changed = snapshot_scope_changed(&self.snapshot, &snapshot);
        let theme_changed = self.snapshot.settings.theme != snapshot.settings.theme
            || self.snapshot.settings.appearance != snapshot.settings.appearance;

        self.read_ids = snapshot.read_dynamics.iter().cloned().collect();
        self.snapshot = snapshot;
        // 外觀由全程式設定管理；保留使用者正在編輯的其他設定欄位。
        self.settings.general.appearance = self.snapshot.settings.appearance.clone();

        let top = self.settings.up_list.logical_scroll_top();
        self.settings.up_list.reset(self.snapshot.ups.len());
        self.settings.up_list.scroll_to(top);

        let groups = self.groups();
        if !["all", "ungrouped"].contains(&self.selected_group.as_str())
            && !groups.iter().any(|group| group.id == self.selected_group)
        {
            self.selected_group = "all".into();
            self.feeds[0].group = "all".into();
        }
        let handle = self.window;
        let _ = handle.update(cx, |_, window, cx| {
            if reset_general {
                self.reset_general_draft(window, cx);
            }
            if theme_changed || !self.settings.initialized {
                self.apply_theme(cx);
            }
            if selector_changed {
                self.sync_selectors(window, cx);
            }
        });
        if scope_changed && self.services.is_some() {
            for index in 0..2 {
                self.feeds[index].initialized = false;
            }
            self.videos.initialized = false;
            if self.view == View::Feed {
                self.load_feed(0, true, cx);
                if self.dual {
                    self.load_feed(1, true, cx);
                }
            }
            if self.view == View::Videos {
                self.load_videos(true, cx);
            }
        }
    }

    pub(super) fn show_toast(&mut self, toast: ToastNotice, cx: &mut Context<Self>) {
        self.notice_sequence += 1;
        self.toast_hovered = false;
        self.toast_focused = false;
        self.toast_remaining = std::time::Duration::from_secs(6);
        self.toast_resumed_at = Some(std::time::Instant::now());
        self.notice = Some(toast);
        cx.notify();
        self.schedule_toast_dismiss(std::time::Duration::from_secs(6), cx);
    }

    pub(super) fn schedule_toast_dismiss(
        &mut self,
        duration: std::time::Duration,
        cx: &mut Context<Self>,
    ) {
        let sequence = self.notice_sequence;
        self.toast_timer_generation = self.toast_timer_generation.wrapping_add(1);
        let generation = self.toast_timer_generation;
        let delay = cx.background_executor().timer(duration);
        cx.spawn(async move |weak, cx| {
            delay.await;
            let _ = weak.update(cx, |this, cx| {
                if this.notice_sequence == sequence
                    && this.toast_timer_generation == generation
                    && !this.toast_hovered
                    && !this.toast_focused
                {
                    this.notice = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn show_notice(&mut self, text: String, cx: &mut Context<Self>) {
        self.show_toast(
            ToastNotice {
                tone: Tone::Success,
                title: text,
                detail: None,
                folder: None,
            },
            cx,
        );
    }

    pub(super) fn toast_time_left(&self) -> std::time::Duration {
        self.toast_remaining.saturating_sub(
            self.toast_resumed_at
                .map(|at| at.elapsed())
                .unwrap_or_default(),
        )
    }
    pub(super) fn sync_toast_pause(&mut self, cx: &mut Context<Self>) {
        if self.toast_hovered || self.toast_focused {
            if self.toast_resumed_at.is_some() {
                self.toast_remaining = self.toast_time_left();
                self.toast_resumed_at = None;
                self.toast_timer_generation = self.toast_timer_generation.wrapping_add(1);
            }
        } else if self.toast_resumed_at.is_none() && self.notice.is_some() {
            self.toast_resumed_at = Some(std::time::Instant::now());
            self.schedule_toast_dismiss(self.toast_remaining, cx);
        }
    }

    pub(super) fn open_local_folder(&mut self, folder: PathBuf, cx: &mut Context<Self>) {
        self.run(
            async move {
                open::that_detached(&folder)
                    .with_context(|| format!("無法開啟資料夾：{}", folder.display()))
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

    pub(super) fn saved(&mut self, result: Result<()>, message: &str, cx: &mut Context<Self>) {
        self.settings.busy = false;
        match result {
            Ok(()) => {
                self.error = None;
                self.settings.form_error = None;
                self.show_notice(message.to_owned(), cx);
                self.reload_snapshot(cx);
            }
            Err(error) => {
                let text = error.to_string();
                if self.view == View::Settings {
                    self.settings.form_error = Some(text);
                } else {
                    self.error = Some(text);
                }
            }
        }
        cx.notify();
    }

    pub(super) fn mark_read(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        let Some(services) = self.services.clone() else {
            return;
        };
        self.run(
            async move { services.store.mark_read(ids).await },
            cx,
            |this, result, cx| match result {
                Ok(()) => this.reload_snapshot(cx),
                Err(error) => {
                    this.error = Some(error.to_string());
                    cx.notify();
                }
            },
        );
    }

    pub(super) fn preview_images(
        &mut self,
        urls: Vec<String>,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(services) = self.services.clone() else {
            return;
        };
        if urls.is_empty() {
            return;
        }
        let runtime = self.runtime.clone();
        let preview = cx.new(|cx| {
            super::preview::Preview::new(urls, index, runtime, services.images.clone(), window, cx)
        });
        let initial_focus = std::cell::Cell::new(true);
        let width = (f32::from(window.viewport_size().width) - 64.).clamp(420., 1040.);
        window.open_dialog(cx, move |dialog, window, cx| {
            if initial_focus.replace(false) {
                preview.focus_handle(cx).focus(window, cx);
            }
            dialog
                .title("圖片預覽")
                .width(px(width))
                .child(preview.clone())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn sample_snapshot() -> Snapshot {
        Snapshot {
            ups: vec![UpUser {
                mid: 1001,
                name: "測試UP".into(),
                face: "https://i0.hdslb.com/bfs/face/a.jpg".into(),
                custom_note: None,
                tags: None,
                is_pinned: Some(false),
                pin_order: None,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn selector_fingerprint_detects_note_pin_face_and_tags() {
        let base = sample_snapshot();
        assert!(!snapshot_selector_changed(&base, &base));

        let mut with_note = base.clone();
        with_note.ups[0].custom_note = Some("常看".into());
        assert!(snapshot_selector_changed(&base, &with_note));
        assert!(!snapshot_scope_changed(&base, &with_note));

        let mut with_pin = base.clone();
        with_pin.ups[0].is_pinned = Some(true);
        with_pin.ups[0].pin_order = Some(1);
        assert!(snapshot_selector_changed(&base, &with_pin));
        assert!(!snapshot_scope_changed(&base, &with_pin));

        let mut with_face = base.clone();
        with_face.ups[0].face = "https://i0.hdslb.com/bfs/face/b.jpg".into();
        assert!(snapshot_selector_changed(&base, &with_face));

        let mut with_tags = base.clone();
        with_tags.ups[0].tags = Some(vec!["技術".into()]);
        assert!(snapshot_selector_changed(&base, &with_tags));
    }

    #[test]
    fn scope_fingerprint_detects_added_up_or_group_change() {
        let base = sample_snapshot();
        let mut added = base.clone();
        added.ups.push(UpUser {
            mid: 2002,
            name: "新UP".into(),
            ..Default::default()
        });
        assert!(snapshot_scope_changed(&base, &added));

        let mut grouped = base.clone();
        grouped.settings.follow_groups = Some(vec![FollowGroup {
            id: "g1".into(),
            name: "特別組".into(),
            mids: vec![1001],
            ..Default::default()
        }]);
        assert!(snapshot_scope_changed(&base, &grouped));
    }
}
