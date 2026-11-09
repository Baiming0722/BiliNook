use super::*;
use gpui::component::{
    checkbox::Checkbox,
    input::{Textarea, TextareaState},
    slider::{Slider, SliderState},
};
use serde_json::Value;

#[derive(Clone)]
pub(crate) enum PanelKind {
    BatchAdd,
    Details(u64),
    Groups,
    Watchlists,
    SaveCurrent,
    History(Option<u64>),
    Notifications,
    Preferences,
    QuickFind,
    Shortcuts,
    Filters,
    BatchManage,
}
impl PanelKind {
    fn title(&self) -> &'static str {
        match self {
            Self::BatchAdd => "批次新增直播間",
            Self::Details(_) => "直播間資訊",
            Self::Groups => "管理直播分組",
            Self::Watchlists | Self::SaveCurrent => "觀看清單",
            Self::History(_) => "觀測紀錄與匯出",
            Self::Notifications => "通知紀錄與勿擾",
            Self::Preferences => "外觀與直播設定",
            Self::QuickFind => "快速尋找",
            Self::Shortcuts => "快捷鍵說明",
            Self::Filters => "篩選與排序",
            Self::BatchManage => "批次管理",
        }
    }
}
#[derive(Clone)]
enum BatchResult {
    Pending,
    Added(u64),
    Existing(u64),
    Failed(String),
}
fn batch_completed(batch: &[(String, BatchResult)]) -> usize {
    batch
        .iter()
        .filter(|(_, result)| !matches!(result, BatchResult::Pending))
        .count()
}

fn apply_live_poll_interval(
    follow: &mut AppSettings,
    live: &mut LiveSettings,
    requested: Option<u64>,
) {
    if let Some(interval) = requested {
        follow.poll_interval_live_sec = interval;
    }
    if [30, 60, 180, 300].contains(&follow.poll_interval_live_sec) {
        live.refresh_interval = follow.poll_interval_live_sec;
    }
}

pub(crate) struct LivePanel {
    kind: PanelKind,
    parent: WeakEntity<BiliApp>,
    services: Arc<Services>,
    runtime: CoreRuntime,
    data: LiveWatchSnapshot,
    pub focus: FocusHandle,
    pub scroll: ScrollHandle,
    pub busy: bool,
    error: Option<String>,
    success: Option<String>,
    text: Entity<InputState>,
    search: Entity<InputState>,
    note: Entity<InputState>,
    description: Entity<InputState>,
    aux: Entity<InputState>,
    from: Entity<InputState>,
    to: Entity<InputState>,
    original: Option<StoredRoom>,
    group: Option<String>,
    pinned: bool,
    notify: bool,
    watch: Option<Watchlist>,
    selected: Vec<u64>,
    delete_confirm: bool,
    batch: Vec<(String, BatchResult)>,
    batch_text: Entity<TextareaState>,
    stop_batch: bool,
    filters: AdvancedFilters,
    theme: String,
    appearance: AppearanceSettings,
    settings: LiveSettings,
    poll_interval: u64,
    poll_interval_edited: bool,
    prefs: NotificationPreferences,
    ui_sound: bool,
    font_scale: Entity<SliderState>,
    card_scale: Entity<SliderState>,
    volume: Entity<SliderState>,
    pending_import: Option<Value>,
    history_state: String,
    quick_index: usize,
}
impl LivePanel {
    fn render_notifications(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let prefs = self.prefs.clone();
        let until = bili_core::now_ms() + 3_600_000;
        let mut content = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .child("勿擾與暫停只停止推播及提示音，仍保留開播紀錄。"),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("notice-snooze")
                            .outline()
                            .label("暫停一小時")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.change(
                                    move |d| {
                                        d.native.preferences.snooze_until = until;
                                        Ok(())
                                    },
                                    cx,
                                )
                            })),
                    )
                    .child(
                        Button::new("notice-resume")
                            .ghost()
                            .label("恢復提醒")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.change(
                                    |d| {
                                        d.native.preferences.snooze_until = 0;
                                        Ok(())
                                    },
                                    cx,
                                )
                            })),
                    ),
            )
            .child(
                Checkbox::new("quiet-enabled")
                    .label("每日勿擾時段")
                    .checked(prefs.quiet_enabled)
                    .on_click(cx.listener(|this, value: &bool, _, cx| {
                        this.prefs.quiet_enabled = *value;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(field("開始（HH:MM）", Input::new(&self.text)))
                    .child(field("結束（HH:MM）", Input::new(&self.note))),
            )
            .child(
                Button::new("quiet-save")
                    .primary()
                    .disabled(self.busy)
                    .label("儲存勿擾設定")
                    .on_click(cx.listener(|this, _, _, cx| {
                        let mut prefs = this.prefs.clone();
                        prefs.quiet_start = value(&this.text, cx);
                        prefs.quiet_end = value(&this.note, cx);
                        this.change(
                            move |d| {
                                d.native.preferences = prefs;
                                Ok(())
                            },
                            cx,
                        );
                    })),
            )
            .child(section("最近 100 筆開播紀錄"))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("notice-read")
                            .ghost()
                            .label("全部標為已讀")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.change(
                                    |d| {
                                        for n in &mut d.native.notices {
                                            n.read = true;
                                        }
                                        Ok(())
                                    },
                                    cx,
                                )
                            })),
                    )
                    .child(
                        Button::new("notice-clear")
                            .ghost()
                            .label("清空通知紀錄")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.change(
                                    |d| {
                                        d.native.notices.clear();
                                        Ok(())
                                    },
                                    cx,
                                )
                            })),
                    ),
            );
        if self.data.data.native.notices.is_empty() {
            content = content.child(div().text_sm().child("目前沒有開播通知紀錄。"));
        }
        for (index, notice) in self
            .data
            .data
            .native
            .notices
            .clone()
            .into_iter()
            .enumerate()
        {
            let id = notice.room_id;
            content = content.child(
                Button::new(("notice-room", index))
                    .ghost()
                    .h_auto()
                    .w_full()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_start()
                            .gap_1()
                            .text_sm()
                            .child(format!(
                                "{}{}",
                                if notice.read { "" } else { "● " },
                                notice.name
                            ))
                            .child(notice.title)
                            .child(div().text_xs().child(notice.at)),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.parent_action(window, cx, move |parent, window, cx| {
                            parent.close_live_panel(window, cx);
                            parent.focus_live_room(id, window, cx);
                        })
                    })),
            );
        }
        content.into_any_element()
    }
    fn render_preferences(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut content = div().flex().flex_col().gap_3().child(section("全程式外觀"));
        let mut themes = div().flex().flex_wrap().gap_2();
        for (index, (id, label)) in [("dark", "暗黑"), ("light", "明亮"), ("system", "跟隨系統")]
            .into_iter()
            .enumerate()
        {
            themes = themes.child(
                Button::new(("appearance-theme", index))
                    .outline()
                    .small()
                    .selected(self.theme == id)
                    .label(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.theme = id.into();
                        cx.notify();
                    })),
            );
        }
        content = content.child(themes);
        let mut colors = div().flex().flex_wrap().gap_2();
        for (index, (id, label)) in [
            ("blue", "藍"),
            ("pink", "粉"),
            ("purple", "紫"),
            ("green", "綠"),
            ("orange", "橙"),
        ]
        .into_iter()
        .enumerate()
        {
            colors = colors.child(
                Button::new(("appearance-color", index))
                    .outline()
                    .small()
                    .selected(self.appearance.accent_color == id)
                    .label(label)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.appearance.accent_color = id.into();
                        this.aux.update(cx, |s, cx| s.set_value("", window, cx));
                        cx.notify();
                    })),
            );
        }
        content = content.child(colors).child(field(
            "自訂重點色（留空使用預設，例如 #1587f2）",
            Input::new(&self.aux),
        ));
        let mut densities = div().flex().flex_wrap().gap_2();
        for (index, (id, label, card, font)) in [
            ("compact", "緊湊", 0.85, 0.9),
            ("standard", "標準", 1., 1.),
            ("comfortable", "舒適", 1.15, 1.1),
        ]
        .into_iter()
        .enumerate()
        {
            densities = densities.child(
                Button::new(("appearance-density", index))
                    .outline()
                    .small()
                    .selected(self.appearance.density == id)
                    .label(label)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.appearance.density = id.into();
                        this.font_scale
                            .update(cx, |s, cx| s.set_value(font, window, cx));
                        this.card_scale
                            .update(cx, |s, cx| s.set_value(card, window, cx));
                        cx.notify();
                    })),
            );
        }
        content = content
            .child(densities)
            .child(field(
                &format!(
                    "字體大小 {:.2} 倍",
                    self.font_scale.read(cx).value().start()
                ),
                Slider::new(&self.font_scale).accessibility_label("字體大小"),
            ))
            .child(field(
                &format!(
                    "卡片大小 {:.2} 倍",
                    self.card_scale.read(cx).value().start()
                ),
                Slider::new(&self.card_scale).accessibility_label("卡片大小"),
            ))
            .child(field(
                "字體（可輸入已安裝字體名稱）",
                Input::new(&self.description),
            ));
        let mut fonts = div().flex().flex_wrap().gap_2();
        for (index, (id, label)) in [
            ("system", "系統預設"),
            ("jhenghei", "微軟正黑體"),
            ("serif", "襯線"),
            ("mono", "等寬"),
        ]
        .into_iter()
        .enumerate()
        {
            fonts = fonts.child(
                Button::new(("appearance-font", index))
                    .ghost()
                    .small()
                    .label(label)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.description
                            .update(cx, |s, cx| s.set_value(id, window, cx));
                        cx.notify();
                    })),
            );
        }
        content = content.child(fonts);
        content = content.child(Checkbox::new("appearance-ascii-animation")
            .label("使用 ASCII 動畫")
            .checked(self.appearance.ascii_animation_enabled)
            .on_click(cx.listener(|this, enabled: &bool, _, cx| {
                this.appearance.ascii_animation_enabled = *enabled;
                cx.notify();
            })))
            .child(div().text_xs().text_color(cx.theme().muted_foreground)
                .child("側欄、標題與未讀卡片顯示裝飾；同步、等待與載入顯示狀態提示。減少動效時保持靜態。"));
        if let Some(old) = self.data.data.native.migrated_appearance.clone() {
            content = content.child(
                Button::new("apply-legacy-appearance")
                    .outline()
                    .label("帶入舊直播外觀供預覽與儲存")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.theme = if old.theme == "auto" {
                            "system".into()
                        } else {
                            old.theme.clone()
                        };
                        let ascii_enabled = this.appearance.ascii_animation_enabled;
                        this.appearance = old.appearance();
                        this.appearance.ascii_animation_enabled = ascii_enabled;
                        this.font_scale
                            .update(cx, |s, cx| s.set_value(old.font_scale, window, cx));
                        this.card_scale
                            .update(cx, |s, cx| s.set_value(old.card_scale, window, cx));
                        this.aux.update(cx, |s, cx| {
                            s.set_value(old.accent_custom.clone().unwrap_or_default(), window, cx)
                        });
                        this.description
                            .update(cx, |s, cx| s.set_value(old.font_family.clone(), window, cx));
                        cx.notify();
                    })),
            );
        }
        content = content.child(section("直播監看與提醒"));
        let mut intervals = div().flex().flex_wrap().gap_2();
        for seconds in [30, 60, 180, 300] {
            intervals = intervals.child(
                Button::new(("live-interval", seconds as usize))
                    .outline()
                    .small()
                    .selected(self.poll_interval == seconds)
                    .label(format!("{seconds} 秒"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.poll_interval = seconds;
                        this.poll_interval_edited = true;
                        cx.notify();
                    })),
            );
        }
        content = content
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "目前間隔 {} 秒；可在一般設定自訂",
                        self.poll_interval
                    )),
            )
            .child(intervals)
            .child(
                Checkbox::new("live-desktop-enabled")
                    .label("直播開播桌面通知（仍受程式總開關控制）")
                    .checked(self.settings.notify_on_live)
                    .on_click(cx.listener(|this, v: &bool, _, cx| {
                        this.settings.notify_on_live = *v;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new("live-sound-enabled")
                    .label("直播開播提示音")
                    .checked(self.settings.sound_enabled)
                    .on_click(cx.listener(|this, v: &bool, _, cx| {
                        this.settings.sound_enabled = *v;
                        cx.notify();
                    })),
            )
            .child(field(
                &format!(
                    "提示音音量 {:.0}%",
                    self.volume.read(cx).value().start() * 100.
                ),
                Slider::new(&self.volume).accessibility_label("提示音音量"),
            ))
            .child(
                Checkbox::new("ui-sound-enabled")
                    .label("介面點擊音效")
                    .checked(self.ui_sound)
                    .on_click(cx.listener(|this, v: &bool, _, cx| {
                        this.ui_sound = *v;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("appearance-save")
                    .primary()
                    .disabled(self.busy)
                    .label("儲存並套用全程式設定")
                    .on_click(cx.listener(|this, _, _, cx| this.save_preferences(cx))),
            )
            .child(section("資料移轉與備份"));
        if let Some(error) = &self.data.data.native.migration_error {
            content = content.child(div().text_sm().child(error.clone()));
        }
        content = content.child(
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .child(
                    Button::new("migration-retry")
                        .outline()
                        .disabled(self.busy)
                        .label("重新讀取舊直播資料")
                        .on_click(cx.listener(|this, _, _, cx| {
                            let services = this.services.clone();
                            this.run(
                                async move {
                                    services.live_watch.migrate_once(None, true).await?;
                                    services.poller.notify_update();
                                    services.live_watch.snapshot().await
                                },
                                cx,
                                |this, data, cx| {
                                    this.data = data;
                                    let _ = this.parent.update(cx, |p, cx| p.reload_snapshot(cx));
                                },
                            );
                        })),
                )
                .child(
                    Button::new("migration-skip")
                        .ghost()
                        .disabled(self.busy)
                        .label("略過舊資料移轉")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.change(
                                |d| {
                                    d.native.migration = "skipped".into();
                                    d.native.migration_error = None;
                                    Ok(())
                                },
                                cx,
                            )
                        })),
                )
                .child(
                    Button::new("live-export-json")
                        .outline()
                        .disabled(self.busy)
                        .label("匯出直播 JSON")
                        .on_click(cx.listener(|this, _, _, cx| {
                            match serde_json::to_vec_pretty(&this.data.data.export_legacy()) {
                                Ok(bytes) if bytes.len() <= MAX_BACKUP_BYTES => {
                                    this.save_file(bytes, "直播監看備份.json", cx)
                                }
                                Ok(_) => this.error = Some("直播備份超過 32 MiB 上限".into()),
                                Err(error) => this.error = Some(error.to_string()),
                            }
                        })),
                )
                .child(
                    Button::new("live-import-json")
                        .outline()
                        .disabled(self.busy)
                        .label("匯入直播 JSON")
                        .on_click(cx.listener(|this, _, _, cx| this.choose_import(cx))),
                ),
        );
        if let Some(payload) = self.pending_import.clone() {
            let merge = payload.clone();
            content = content.child(
                div()
                    .p_3()
                    .rounded_md()
                    .bg(cx.theme().muted)
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child("已選擇備份。覆蓋會替換直播房間、分組、觀看清單與歷史；請先匯出備份。")
                    .child(
                        Button::new("live-import-merge")
                            .primary()
                            .disabled(self.busy)
                            .label("合併匯入")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let value = merge.clone();
                                this.change(
                                    move |d| {
                                        d.import(value, false)?;
                                        Ok(())
                                    },
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new("live-import-replace")
                            .outline()
                            .disabled(self.busy)
                            .label(if self.delete_confirm {
                                "確認覆蓋直播資料"
                            } else {
                                "覆蓋匯入…"
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.delete_confirm {
                                    this.delete_confirm = true;
                                    cx.notify();
                                    return;
                                }
                                let value = payload.clone();
                                this.change(
                                    move |d| {
                                        d.import(value, true)?;
                                        Ok(())
                                    },
                                    cx,
                                );
                            })),
                    ),
            );
        }
        content.into_any_element()
    }
    fn save_preferences(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let mut appearance = self.appearance.clone();
        appearance.font_scale = self.font_scale.read(cx).value().start();
        appearance.card_scale = self.card_scale.read(cx).value().start();
        appearance.font_family = value(&self.description, cx).trim().into();
        let color = value(&self.aux, cx);
        appearance.accent_custom = (!color.trim().is_empty()).then(|| color.trim().into());
        if let Err(error) = appearance.validate() {
            self.error = Some(error.to_string());
            cx.notify();
            return;
        }
        let mut settings = self.settings.clone();
        settings.sound_volume = self.volume.read(cx).value().start();
        settings.font_scale = appearance.font_scale;
        settings.card_scale = appearance.card_scale;
        settings.font_family = appearance.font_family.clone();
        settings.accent_color = appearance.accent_color.clone();
        settings.accent_custom = appearance.accent_custom.clone();
        settings.density = appearance.density.clone();
        let theme = self.theme.clone();
        settings.theme = if theme == "system" {
            "auto".into()
        } else {
            theme.clone()
        };
        let sound = self.ui_sound;
        let poll_interval = self.poll_interval_edited.then_some(self.poll_interval);
        let services = self.services.clone();
        self.run(
            async move {
                let interval = services
                    .store
                    .update_both(move |follow, live| {
                        follow.settings.appearance = appearance;
                        follow.settings.theme = theme;
                        apply_live_poll_interval(
                            &mut follow.settings,
                            &mut settings,
                            poll_interval,
                        );
                        live.settings = settings;
                        live.native.ui_sound_enabled = sound;
                        Ok(follow.settings.poll_interval_live_sec)
                    })
                    .await?;
                services.poller.reconfigure();
                services.poller.notify_update();
                Ok((services.live_watch.snapshot().await?, interval))
            },
            cx,
            |this, (data, interval), cx| {
                this.data = data;
                this.poll_interval = interval;
                this.poll_interval_edited = false;
                this.success = Some("全程式外觀與直播設定已套用".into());
                let _ = this.parent.update(cx, |p, cx| p.reload_snapshot(cx));
            },
        );
    }
    fn read_filters(&self, cx: &App) -> Result<AdvancedFilters> {
        let mut filters = self.filters.clone();
        let area = value(&self.search, cx);
        let parent = value(&self.description, cx);
        filters.area = if area.trim().is_empty() {
            "all".into()
        } else {
            area.trim().into()
        };
        filters.parent_area = if parent.trim().is_empty() {
            "all".into()
        } else {
            parent.trim().into()
        };
        let min = value(&self.aux, cx);
        let attention = value(&self.note, cx);
        filters.online_min = if min.trim().is_empty() {
            0
        } else {
            min.trim()
                .parse()
                .map_err(|_| anyhow!("最低人氣須為非負整數"))?
        };
        filters.attention_min = if attention.trim().is_empty() {
            0
        } else {
            attention
                .trim()
                .parse()
                .map_err(|_| anyhow!("最低關注數須為非負整數"))?
        };
        Ok(filters)
    }
    fn render_filters(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut content = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(field(
                "分區名稱（留空或 all 表示全部）",
                Input::new(&self.search),
            ))
            .child(field("大分區名稱", Input::new(&self.description)))
            .child(field("最低人氣", Input::new(&self.aux)))
            .child(field("最低關注數", Input::new(&self.note)));
        let mut sorts = div().flex().flex_wrap().gap_2();
        for (index, (id, label)) in [
            ("order", "自訂順序"),
            ("liveFirst", "直播優先"),
            ("online", "人氣"),
            ("attention", "關注數"),
            ("addedAt", "新增時間"),
        ]
        .into_iter()
        .enumerate()
        {
            sorts = sorts.child(
                Button::new(("filter-sort", index))
                    .small()
                    .outline()
                    .selected(self.filters.sort_by == id)
                    .label(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.filters.sort_by = id.into();
                        cx.notify();
                    })),
            );
        }
        content = content
            .child(sorts)
            .child(
                Button::new("filter-direction")
                    .outline()
                    .label(if self.filters.sort_dir == "desc" {
                        "降冪 ↓"
                    } else {
                        "升冪 ↑"
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.filters.sort_dir = if this.filters.sort_dir == "desc" {
                            "asc"
                        } else {
                            "desc"
                        }
                        .into();
                        cx.notify();
                    })),
            )
            .child(
                Button::new("filter-apply")
                    .primary()
                    .label("套用篩選與排序")
                    .on_click(
                        cx.listener(|this, _, window, cx| match this.read_filters(cx) {
                            Ok(filters) => this.parent_action(window, cx, move |p, window, cx| {
                                p.live.filter.advanced = filters;
                                p.filter_live(true, cx);
                                p.close_live_panel(window, cx);
                            }),
                            Err(error) => {
                                this.error = Some(error.to_string());
                                cx.notify();
                            }
                        }),
                    ),
            )
            .child(section("篩選預設集"))
            .child(field("預設集名稱", Input::new(&self.text)))
            .child(
                Button::new("filter-save-preset")
                    .outline()
                    .disabled(self.busy)
                    .label("儲存為預設集")
                    .on_click(cx.listener(|this, _, _, cx| {
                        let name = value(&this.text, cx);
                        if name.trim().is_empty() {
                            this.error = Some("請輸入預設集名稱".into());
                            cx.notify();
                            return;
                        }
                        match this.read_filters(cx) {
                            Ok(filters) => this.change(
                                move |d| {
                                    d.settings.filter_presets.push(FilterPreset {
                                        id: new_id("filter"),
                                        name: name.trim().into(),
                                        sort_by: filters.sort_by.clone(),
                                        sort_order: filters.sort_dir.clone(),
                                        filters,
                                        created_at: iso(bili_core::now_ms()),
                                    });
                                    Ok(())
                                },
                                cx,
                            ),
                            Err(error) => {
                                this.error = Some(error.to_string());
                                cx.notify();
                            }
                        }
                    })),
            );
        for (index, preset) in self
            .data
            .data
            .settings
            .filter_presets
            .clone()
            .into_iter()
            .enumerate()
        {
            let remove = preset.id.clone();
            content = content.child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new(("filter-load-preset", index))
                            .ghost()
                            .label(preset.name)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let filters = preset.filters.clone();
                                this.parent_action(window, cx, move |p, window, cx| {
                                    p.live.filter.advanced = filters;
                                    p.filter_live(true, cx);
                                    p.close_live_panel(window, cx);
                                });
                            })),
                    )
                    .child(
                        Button::new(("filter-delete-preset", index))
                            .ghost()
                            .small()
                            .label("移除")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let id = remove.clone();
                                this.change(
                                    move |d| {
                                        d.settings.filter_presets.retain(|p| p.id != id);
                                        Ok(())
                                    },
                                    cx,
                                );
                            })),
                    ),
            );
        }
        content.into_any_element()
    }
    fn quick_results(&self, cx: &App) -> Vec<(String, QuickTarget)> {
        let query = value(&self.text, cx).trim().to_lowercase();
        let words: Vec<_> = query.split_whitespace().collect();
        let mut result = vec![];
        for room in &self.data.rooms {
            let hay = format!(
                "{} {} {} {} {}",
                room.saved.room_id,
                room.saved.short_id.unwrap_or(0),
                room.info.uname,
                room.info.title,
                room.saved.note
            )
            .to_lowercase();
            if words.iter().all(|w| hay.contains(w)) {
                let score = if room.saved.room_id.to_string() == query
                    || room.info.uname.to_lowercase() == query
                {
                    0
                } else if room.info.uname.to_lowercase().starts_with(&query) {
                    1
                } else {
                    2
                };
                result.push((
                    score,
                    format!(
                        "{} · #{} · {}",
                        room.info.uname,
                        room.saved.room_id,
                        room.info.status.label()
                    ),
                    QuickTarget::Room(room.saved.room_id),
                ));
            }
        }
        for list in &self.data.data.watchlists {
            if words.iter().all(|w| {
                format!("{} {} 觀看清單", list.name, list.description)
                    .to_lowercase()
                    .contains(w)
            }) {
                result.push((
                    3,
                    format!("觀看清單：{}", list.name),
                    QuickTarget::Watch(list.clone()),
                ));
            }
        }
        for (label, target) in [
            ("重新整理直播狀態", QuickTarget::Refresh),
            ("開啟觀測統計", QuickTarget::Statistics),
            ("多房同屏", QuickTarget::Multi),
            ("管理觀看清單", QuickTarget::Panel(PanelKind::Watchlists)),
            ("新增多個直播間", QuickTarget::Panel(PanelKind::BatchAdd)),
            ("查詢觀測紀錄", QuickTarget::Panel(PanelKind::History(None))),
        ] {
            if words.iter().all(|w| label.to_lowercase().contains(w)) {
                result.push((4, label.into(), target));
            }
        }
        result.sort_by_key(|r| r.0);
        result
            .into_iter()
            .take(20)
            .map(|(_, label, target)| (label, target))
            .collect()
    }
    fn activate_quick(&mut self, target: QuickTarget, window: &mut Window, cx: &mut Context<Self>) {
        self.parent_action(window, cx, move |parent, window, cx| {
            parent.live.keyboard = true;
            parent.close_live_panel(window, cx);
            match target {
                QuickTarget::Room(id) => parent.focus_live_room(id, window, cx),
                QuickTarget::Watch(list) => {
                    parent.switch_view(View::Live, window, cx);
                    parent.live.filter.only_ids = Some(list.room_ids.into_iter().collect());
                    parent.filter_live(true, cx);
                }
                QuickTarget::Refresh => parent.refresh(cx),
                QuickTarget::Statistics => {
                    parent.switch_view(View::Live, window, cx);
                    parent.stop_live_players(window);
                    parent.live.statistics = true;
                }
                QuickTarget::Multi => {
                    let ids = parent
                        .live
                        .snapshot
                        .rooms
                        .iter()
                        .filter(|r| r.info.status == RoomStatus::Live)
                        .map(|r| r.saved.room_id)
                        .collect();
                    parent.switch_view(View::Live, window, cx);
                    parent.start_multi(ids, MultiViewLayout::Auto, 4, window, cx);
                }
                QuickTarget::Panel(kind) => parent.open_live_panel(kind, window, cx),
            }
            cx.notify();
        });
    }
    fn render_quick_find(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let results = self.quick_results(cx);
        let mut content = div()
            .flex()
            .flex_col()
            .gap_2()
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let results = this.quick_results(cx);
                match event.keystroke.key.as_str() {
                    "down" => {
                        this.quick_index =
                            (this.quick_index + 1).min(results.len().saturating_sub(1));
                        cx.stop_propagation();
                    }
                    "up" => {
                        this.quick_index = this.quick_index.saturating_sub(1);
                        cx.stop_propagation();
                    }
                    "enter" => {
                        if let Some((_, target)) = results.get(this.quick_index) {
                            this.activate_quick(target.clone(), window, cx);
                        }
                        cx.stop_propagation();
                    }
                    _ => {}
                }
                cx.notify();
            }))
            .child(Input::new(&self.text).prefix(Icon::new(IconName::Search)));
        if results.is_empty() {
            content = content.child(div().p_3().child("沒有符合的房間、清單或操作。"));
        }
        for (index, (label, target)) in results.into_iter().enumerate() {
            content = content.child(
                Button::new(("quick-result", index))
                    .ghost()
                    .w_full()
                    .selected(self.quick_index == index)
                    .label(label)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.activate_quick(target.clone(), window, cx)
                    })),
            );
        }
        content
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("↑ ↓ 選擇 · Enter 開啟 · Esc 關閉"),
            )
            .into_any_element()
    }
}
#[derive(Clone)]
enum QuickTarget {
    Room(u64),
    Watch(Watchlist),
    Refresh,
    Statistics,
    Multi,
    Panel(PanelKind),
}
impl LivePanel {
    fn new(
        kind: PanelKind,
        parent: WeakEntity<BiliApp>,
        services: Arc<Services>,
        runtime: CoreRuntime,
        data: LiveWatchSnapshot,
        app_settings: AppSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let today = chrono::Local::now().date_naive();
        let appearance = app_settings.appearance.clone();
        let slider = |v: f32, min: f32, max: f32, cx: &mut App| {
            cx.new(|_| {
                SliderState::new()
                    .min(min)
                    .max(max)
                    .step(0.05)
                    .default_value(v)
            })
        };
        let mut panel = Self {
            kind,
            parent,
            services,
            runtime,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            busy: false,
            error: None,
            success: None,
            text: input("", "名稱／房號或網址", window, cx),
            search: input("", "搜尋房間或備註", window, cx),
            note: input("", "備註（最多 200 字）", window, cx),
            description: input("", "用途說明", window, cx),
            aux: input("", "", window, cx),
            from: input(
                &(today - chrono::Duration::days(6))
                    .format("%Y-%m-%d")
                    .to_string(),
                "YYYY-MM-DD",
                window,
                cx,
            ),
            to: input(
                &today.format("%Y-%m-%d").to_string(),
                "YYYY-MM-DD",
                window,
                cx,
            ),
            original: None,
            group: None,
            pinned: false,
            notify: true,
            watch: None,
            selected: vec![],
            delete_confirm: false,
            batch: vec![],
            batch_text: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .placeholder("每行一筆房號或網址")
                    .auto_grow(4, 6)
            }),
            stop_batch: false,
            filters: AdvancedFilters::default(),
            theme: app_settings.theme,
            appearance: appearance.clone(),
            settings: data.data.settings.clone(),
            poll_interval: app_settings.poll_interval_live_sec,
            poll_interval_edited: false,
            prefs: data.data.native.preferences.clone(),
            ui_sound: data.data.native.ui_sound_enabled,
            font_scale: slider(appearance.font_scale, 0.5, 2., cx),
            card_scale: slider(appearance.card_scale, 0.5, 2., cx),
            volume: slider(data.data.settings.sound_volume, 0., 1., cx),
            data,
            pending_import: None,
            history_state: "全部".into(),
            quick_index: 0,
        };
        if let PanelKind::Details(id) = panel.kind {
            if let Some(room) = panel.data.rooms.iter().find(|r| r.saved.room_id == id) {
                panel.original = Some(room.saved.clone());
                panel.group = room.saved.group_id.clone();
                panel.pinned = room.saved.pinned;
                panel.notify = room.saved.notify;
                panel
                    .note
                    .update(cx, |s, cx| s.set_value(room.saved.note.clone(), window, cx));
            }
        }
        if matches!(panel.kind, PanelKind::Preferences) {
            panel.aux.update(cx, |s, cx| {
                s.set_value(
                    appearance.accent_custom.clone().unwrap_or_default(),
                    window,
                    cx,
                )
            });
            panel.description.update(cx, |s, cx| {
                s.set_value(appearance.font_family.clone(), window, cx)
            });
        }
        for entity in [
            panel.text.clone(),
            panel.search.clone(),
            panel.from.clone(),
            panel.to.clone(),
        ] {
            cx.subscribe_in(&entity, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.quick_index = 0;
                    cx.notify();
                }
            })
            .detach();
        }
        for entity in [
            panel.font_scale.clone(),
            panel.card_scale.clone(),
            panel.volume.clone(),
        ] {
            cx.observe(&entity, |_, _, cx| cx.notify()).detach();
        }
        panel
    }
    fn run<T: Send + 'static>(
        &mut self,
        future: impl Future<Output = Result<T>> + Send + 'static,
        cx: &mut Context<Self>,
        done: impl FnOnce(&mut Self, T, &mut Context<Self>) + 'static,
    ) {
        self.busy = true;
        self.error = None;
        self.success = None;
        match self.runtime.submit(future) {
            Ok(receiver) => {
                cx.spawn(async move |weak, cx| {
                    let result = receiver
                        .recv()
                        .await
                        .unwrap_or_else(|_| Err(anyhow!("背景服務未回應")));
                    let _ = weak.update(cx, |this, cx| {
                        this.busy = false;
                        match result {
                            Ok(result) => done(this, result, cx),
                            Err(error) => this.error = Some(error.to_string()),
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            Err(error) => {
                self.busy = false;
                self.error = Some(error.to_string());
            }
        }
        cx.notify();
    }
    fn change(
        &mut self,
        change: impl FnOnce(&mut LiveWatchData) -> Result<()> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        let services = self.services.clone();
        self.run(
            async move {
                services.store.update_live(change).await?;
                services.poller.notify_update();
                services.live_watch.snapshot().await
            },
            cx,
            |this, data, cx| {
                this.data = data;
                if let PanelKind::Details(id) = this.kind {
                    this.original = this
                        .data
                        .rooms
                        .iter()
                        .find(|r| r.saved.room_id == id)
                        .map(|r| r.saved.clone());
                }
                this.prefs.snooze_until = this.data.data.native.preferences.snooze_until;
                this.success = Some("已儲存".into());
                let _ = this
                    .parent
                    .update(cx, |parent, cx| parent.reload_snapshot(cx));
            },
        );
    }
    fn reload(&mut self, cx: &mut Context<Self>) {
        let services = self.services.clone();
        self.run(
            async move { services.live_watch.snapshot().await },
            cx,
            |this, data, cx| {
                this.data = data;
                cx.notify();
            },
        );
    }
    fn parent_action(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
        action: impl FnOnce(&mut BiliApp, &mut Window, &mut Context<BiliApp>) + 'static,
    ) {
        // 父層可能讀取或關閉目前抽屜，必須等目前 Entity 更新結束再執行。
        let parent = self.parent.clone();
        let handle = window.window_handle();
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| {
                let _ = parent.update(cx, |parent, cx| action(parent, window, cx));
            });
        });
    }
    fn group_picker(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut row = div().flex().flex_wrap().gap_1().child(
            Button::new("panel-ungrouped")
                .small()
                .outline()
                .selected(self.group.is_none())
                .label("未分組")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.group = None;
                    cx.notify();
                })),
        );
        for group in &self.data.data.groups {
            let id = group.id.clone();
            row = row.child(
                Button::new(SharedString::from(format!("panel-group-{id}")))
                    .small()
                    .outline()
                    .selected(self.group.as_ref() == Some(&id))
                    .label(group.name.clone())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.group = Some(id.clone());
                        cx.notify();
                    })),
            );
        }
        row.into_any_element()
    }
    fn room_choices(&self, cx: &mut Context<Self>) -> AnyElement {
        let query = value(&self.search, cx).to_lowercase();
        let mut list = div().flex().flex_col().gap_1();
        for room in self
            .data
            .rooms
            .iter()
            .filter(|r| {
                format!(
                    "{} {} {} {}",
                    r.saved.room_id, r.info.uname, r.info.title, r.saved.note
                )
                .to_lowercase()
                .contains(&query)
            })
            .take(100)
        {
            let id = room.saved.room_id;
            list = list.child(
                Checkbox::new(("panel-room", id))
                    .label(format!(
                        "{} · #{} · {}",
                        room.info.uname,
                        id,
                        room.info.status.label()
                    ))
                    .checked(self.selected.contains(&id))
                    .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                        if *checked {
                            if !this.selected.contains(&id)
                                && this.selected.len()
                                    < if matches!(this.kind, PanelKind::BatchManage) {
                                        MAX_ROOMS
                                    } else {
                                        50
                                    }
                            {
                                this.selected.push(id);
                            }
                        } else {
                            this.selected.retain(|r| *r != id);
                        }
                        cx.notify();
                    })),
            );
        }
        list.child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("最多顯示前 100 間；可用搜尋找到其他房間。"),
        )
        .into_any_element()
    }
    fn start_batch(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if self.batch.is_empty() {
            let text = self.batch_text.read(cx).value().to_string();
            let items: Vec<_> = text
                .split(|c: char| c == '\n' || c == '\r' || c == ',' || c == '，' || c == ';')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect();
            if items.is_empty() || items.len() > 50 {
                self.error = Some("每次請輸入 1–50 筆房號或網址".into());
                return;
            }
            self.batch = items
                .into_iter()
                .map(|s| (s, BatchResult::Pending))
                .collect();
        }
        self.stop_batch = false;
        self.next_batch(cx);
    }
    fn next_batch(&mut self, cx: &mut Context<Self>) {
        if self.stop_batch {
            return;
        }
        let Some(index) = self
            .batch
            .iter()
            .position(|(_, s)| matches!(s, BatchResult::Pending))
        else {
            self.success = Some("批次處理完成；失敗項目可重試。".into());
            cx.notify();
            return;
        };
        let input = self.batch[index].0.clone();
        let services = self.services.clone();
        let group = self.group.clone();
        self.busy = true;
        self.error = None;
        let result = self.runtime.submit(async move {
            let (id, added) = services.live_watch.add_input(input).await?;
            if added && group.is_some() {
                services
                    .store
                    .update_live(move |d| {
                        d.patch_room(
                            id,
                            RoomPatch {
                                group_id: Some(group),
                                ..Default::default()
                            },
                        )
                    })
                    .await?;
            }
            services.poller.notify_update();
            Ok((id, added))
        });
        match result {
            Ok(receiver) => {
                cx.spawn(async move |weak, cx| {
                    let result = receiver
                        .recv()
                        .await
                        .unwrap_or_else(|_| Err(anyhow!("背景服務未回應")));
                    let _ = weak.update(cx, |this, cx| {
                        this.busy = false;
                        this.batch[index].1 = match result {
                            Ok((id, true)) => BatchResult::Added(id),
                            Ok((id, false)) => BatchResult::Existing(id),
                            Err(error) => BatchResult::Failed(error.to_string()),
                        };
                        let _ = this
                            .parent
                            .update(cx, |parent, cx| parent.reload_snapshot(cx));
                        this.next_batch(cx);
                        cx.notify();
                    });
                })
                .detach();
            }
            Err(error) => {
                self.busy = false;
                self.error = Some(error.to_string());
            }
        }
    }
    fn save_details(&mut self, cx: &mut Context<Self>) {
        let Some(original) = self.original.clone() else {
            return;
        };
        let note = value(&self.note, cx).trim().to_owned();
        let id = original.room_id;
        let patch = RoomPatch {
            note: (note != original.note).then_some(note),
            group_id: (self.group != original.group_id).then_some(self.group.clone()),
            pinned: (self.pinned != original.pinned).then_some(self.pinned),
            notify: (self.notify != original.notify).then_some(self.notify),
        };
        self.change(move |d| d.patch_room(id, patch), cx);
    }
    fn edit_watchlist(&mut self, list: Watchlist, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = list.room_ids.clone();
        self.text
            .update(cx, |s, cx| s.set_value(list.name.clone(), window, cx));
        self.note.update(cx, |s, cx| {
            s.set_value(list.description.clone(), window, cx)
        });
        self.watch = Some(list);
        self.delete_confirm = false;
        cx.notify();
    }
    fn save_watchlist(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let mut list = self.watch.clone().unwrap_or_default();
        list.name = value(&self.text, cx);
        list.description = value(&self.note, cx);
        list.room_ids = self.selected.clone();
        let services = self.services.clone();
        self.run(
            async move {
                let id = services
                    .store
                    .update_live(move |d| d.save_watchlist(list))
                    .await?;
                services.poller.notify_update();
                Ok((id, services.live_watch.snapshot().await?))
            },
            cx,
            |this, (id, data), cx| {
                this.watch = data.data.watchlists.iter().find(|w| w.id == id).cloned();
                this.data = data;
                this.success = Some("觀看清單已儲存".into());
                let _ = this.parent.update(cx, |p, cx| p.reload_snapshot(cx));
            },
        );
    }
    fn history_rows(&self, cx: &App) -> Result<Vec<HistoryRow>> {
        let range = local_date_range(&value(&self.from, cx), &value(&self.to, cx))?;
        let room = if let PanelKind::History(id) = self.kind {
            id
        } else {
            None
        };
        let input = value(&self.aux, cx);
        let room = if room.is_some() {
            room
        } else if input.trim().is_empty() {
            None
        } else {
            Some(parse_room_input(&input)?)
        };
        Ok(
            history_rows(&self.data.data, range, room, bili_core::now_ms())
                .into_iter()
                .filter(|r| self.history_state == "全部" || r.state == self.history_state)
                .collect(),
        )
    }
    fn save_file(&mut self, bytes: Vec<u8>, name: &str, cx: &mut Context<Self>) {
        let directory =
            dirs::document_dir().unwrap_or_else(|| self.services.store.directory().into());
        let prompt = cx.prompt_for_new_path(&directory, Some(name));
        cx.spawn(async move |weak, cx| match prompt.await {
            Ok(Ok(Some(path))) => {
                let _ = weak.update(cx, |this, cx| {
                    this.run(
                        async move {
                            tokio::task::spawn_blocking(move || -> Result<()> {
                                std::fs::write(&path, bytes)?;
                                Ok(())
                            })
                            .await?
                        },
                        cx,
                        |this, _, _| this.success = Some("檔案已匯出".into()),
                    )
                });
            }
            Ok(Ok(None)) => {}
            _ => {
                let _ = weak.update(cx, |this, cx| {
                    this.error = Some("無法開啟儲存對話框".into());
                    cx.notify();
                });
            }
        })
        .detach();
    }
    fn choose_import(&mut self, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("選擇直播監看 JSON 備份".into()),
        });
        cx.spawn(async move |weak, cx| match prompt.await {
            Ok(Ok(Some(paths))) => {
                if let Some(path) = paths.into_iter().next() {
                    let _ = weak.update(cx, |this, cx| {
                        this.run(
                            async move {
                                tokio::task::spawn_blocking(move || -> Result<Value> {
                                    anyhow::ensure!(
                                        std::fs::metadata(&path)?.len() <= MAX_BACKUP_BYTES as u64,
                                        "直播備份超過 32 MiB 上限"
                                    );
                                    parse_backup_bytes(&std::fs::read(path)?)
                                })
                                .await?
                            },
                            cx,
                            |this, value, _| this.pending_import = Some(value),
                        )
                    });
                }
            }
            Ok(Ok(None)) => {}
            _ => {
                let _ = weak.update(cx, |this, cx| {
                    this.error = Some("無法開啟匯入對話框".into());
                    cx.notify();
                });
            }
        })
        .detach();
    }
}

impl BiliApp {
    pub(crate) fn open_live_panel(
        &mut self,
        kind: PanelKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(services) = self.services.clone() else {
            return;
        };
        let runtime = self.runtime.clone();
        let data = self.live.snapshot.clone();
        let settings = self.snapshot.settings.clone();
        let parent = cx.entity().downgrade();
        let save_current = matches!(kind, PanelKind::SaveCurrent);
        let filters = matches!(kind, PanelKind::Filters);
        let notifications = matches!(kind, PanelKind::Notifications);
        let panel = cx
            .new(|cx| LivePanel::new(kind, parent, services, runtime, data, settings, window, cx));
        if save_current {
            let list = Watchlist {
                room_ids: self.live.watch_ids.clone(),
                layout: self.live.layout.clone(),
                playback_limit: self.live.limit,
                ..Default::default()
            };
            panel.update(cx, |panel, cx| panel.edit_watchlist(list, window, cx));
        }
        if filters {
            panel.update(cx, |panel, cx| {
                panel.filters = self.live.filter.advanced.clone();
                let f = panel.filters.clone();
                panel
                    .search
                    .update(cx, |s, cx| s.set_value(f.area, window, cx));
                panel
                    .description
                    .update(cx, |s, cx| s.set_value(f.parent_area, window, cx));
                panel.aux.update(cx, |s, cx| {
                    s.set_value(f.online_min.to_string(), window, cx)
                });
                panel.note.update(cx, |s, cx| {
                    s.set_value(f.attention_min.to_string(), window, cx)
                });
            });
        }
        if notifications {
            panel.update(cx, |panel, cx| {
                panel.text.update(cx, |s, cx| {
                    s.set_value(panel.prefs.quiet_start.clone(), window, cx)
                });
                panel.note.update(cx, |s, cx| {
                    s.set_value(panel.prefs.quiet_end.clone(), window, cx)
                });
            });
        }
        self.live.previous_focus.push(window.focused(cx));
        self.live.panels.push(panel.clone());
        self.live.closing = false;
        self.live.panel_motion = MotionValue::settled(0.);
        self.live
            .panel_motion
            .target(1., motion::SCENE, cx.reduce_motion() || self.live.keyboard);
        self.live.host.borrow_mut().cover(true);
        let focus = panel.read(cx).focus.clone();
        focus.focus(window, cx);
        let first = panel.read(cx);
        let input_focus = match first.kind {
            PanelKind::QuickFind | PanelKind::Groups | PanelKind::SaveCurrent => {
                Some(first.text.focus_handle(cx))
            }
            PanelKind::BatchAdd => Some(first.batch_text.focus_handle(cx)),
            PanelKind::Details(_) => Some(first.note.focus_handle(cx)),
            PanelKind::History(_) => Some(first.from.focus_handle(cx)),
            _ => None,
        };
        if let Some(focus) = input_focus {
            focus.focus(window, cx);
        }
        cx.notify();
    }
    pub(crate) fn close_live_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.live.panels.last().is_some_and(|p| p.read(cx).busy) {
            return;
        }
        if self.live.closing {
            return;
        }
        if cx.reduce_motion() || self.live.keyboard {
            self.finish_live_panel(window, cx);
            return;
        }
        self.live.closing = true;
        self.live.panel_motion.target(0., motion::SCENE, false);
        let id = self.live.panels.last().map(Entity::entity_id);
        let handle = self.window;
        cx.spawn(async move |weak, cx| {
            cx.background_executor().timer(motion::SCENE).await;
            let _ = handle.update(cx, |_, window, cx| {
                let _ = weak.update(cx, |this, cx| {
                    if this.live.closing && this.live.panels.last().map(Entity::entity_id) == id {
                        this.finish_live_panel(window, cx);
                    }
                });
            });
        })
        .detach();
        cx.notify();
    }
    fn finish_live_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.live.panels.pop();
        if let Some(Some(focus)) = self.live.previous_focus.pop() {
            focus.focus(window, cx);
        }
        self.live.closing = false;
        self.live.panel_motion = MotionValue::settled(1.);
        self.live
            .host
            .borrow_mut()
            .cover(!self.live.panels.is_empty());
        if self.live.panels.is_empty()
            && let Some(id) = self.live.pending_focus.take()
        {
            self.focus_live_room(id, window, cx);
        }
        cx.notify();
    }
    pub(crate) fn render_live_drawer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let panel = self.live.panels.last()?.clone();
        let state = panel.read(cx);
        let focus = state.focus.clone();
        let busy = state.busy;
        let title = state.kind.title();
        let scroll = state.scroll.clone();
        if cx.reduce_motion() {
            self.live.panel_motion.target(
                if self.live.closing { 0. } else { 1. },
                motion::SCENE,
                true,
            );
        }
        let progress = self.live.panel_motion.value();
        if self.live.panel_motion.active() {
            window.request_animation_frame();
        }
        let width = (f32::from(window.viewport_size().width) * 0.7).clamp(460., 760.);
        let parent = cx.entity().downgrade();
        let surface =
            div()
                .absolute()
                .right_0()
                .top_0()
                .h_full()
                .w(px(width))
                .occlude()
                .bg(cx.theme().popover)
                .border_l_1()
                .border_color(cx.theme().border)
                .shadow_xl()
                .flex()
                .flex_col()
                .child(
                    div()
                        .p_4()
                        .flex_shrink_0()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .flex()
                        .justify_between()
                        .items_center()
                        .child(div().font_weight(FontWeight::BOLD).child(title))
                        .child(
                            Button::new("live-panel-close")
                                .ghost()
                                .small()
                                .icon(IconName::Close)
                                .accessibility_label("關閉面板")
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.close_live_panel(window, cx)
                                })),
                        ),
                )
                .child(crate::scroll::smooth_scroll_container(
                    "live-panel-scroll",
                    cx.entity_id(),
                    &scroll,
                    div().child(panel),
                ));
        Some(
            gpui::base::Sheet::new(cx)
                .focus_handle(focus)
                .overlay_closable(!busy)
                .overlay(
                    div()
                        .absolute()
                        .inset_0()
                        .bg(rgba(0x00000088))
                        .opacity(progress),
                )
                .surface(motion::translated(surface, width * (1. - progress), 0.))
                .request_close(move |window, cx| {
                    let _ = parent.update(cx, |parent, cx| {
                        parent.close_live_panel(window, cx);
                    });
                })
                .into_any_element(),
        )
    }
    pub(crate) fn render_live_sidebar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut content = div().pt_4().flex().flex_col().gap_1().child(
            div()
                .px_2()
                .py_1()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("直播房間分組"),
        );
        for (index, (label, id, ungrouped)) in [("全部直播間", None, false), ("未分組", None, true)]
            .into_iter()
            .enumerate()
        {
            content = content.child(
                Button::new(("live-sidebar", index))
                    .ghost()
                    .small()
                    .w_full()
                    .label(label)
                    .selected(
                        self.live.filter.group == id && self.live.filter.ungrouped == ungrouped,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.live.filter.group = id.clone();
                        this.live.filter.ungrouped = ungrouped;
                        this.filter_live(true, cx);
                    })),
            )
        }
        for group in &self.live.snapshot.data.groups {
            let id = group.id.clone();
            content = content.child(
                Button::new(SharedString::from(format!("live-sidebar-{id}")))
                    .ghost()
                    .small()
                    .w_full()
                    .selected(self.live.filter.group.as_ref() == Some(&id))
                    .label(group.name.clone())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.live.filter.group = Some(id.clone());
                        this.live.filter.ungrouped = false;
                        this.filter_live(true, cx);
                    })),
            );
        }
        content
            .child(
                Button::new("live-sidebar-manage")
                    .ghost()
                    .small()
                    .label("管理分組")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_live_panel(PanelKind::Groups, window, cx)
                    })),
            )
            .into_any_element()
    }
}

fn field(label: &str, element: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .child(label.to_owned()),
        )
        .child(element)
}
fn section(title: &str) -> Div {
    div()
        .pt_3()
        .pb_1()
        .border_b_1()
        .border_color(rgba(0x80808033))
        .font_weight(FontWeight::SEMIBOLD)
        .child(title.to_owned())
}

impl LivePanel {
    fn render_batch(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut content = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(field(
                "每行一筆，或以逗號分隔（最多 50 筆）",
                Textarea::new(&self.batch_text),
            ))
            .child(self.group_picker(cx));
        content = content.child(
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .child(
                    Button::new("batch-start")
                        .primary()
                        .disabled(self.busy)
                        .accessibility_label("開始批次新增")
                        .child(crate::ascii::button_label_with_style(
                            "batch-wait",
                            if self.batch.is_empty() {
                                "開始新增"
                            } else {
                                "繼續未處理"
                            },
                            self.busy,
                            crate::ascii::Style::Working,
                        ))
                        .on_click(cx.listener(|this, _, _, cx| this.start_batch(cx))),
                )
                .child(
                    Button::new("batch-stop")
                        .outline()
                        .disabled(!self.busy)
                        .label("停止後續新增")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.stop_batch = true;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("batch-retry")
                        .outline()
                        .disabled(self.busy)
                        .label("重試失敗項目")
                        .on_click(cx.listener(|this, _, _, cx| {
                            for (_, state) in &mut this.batch {
                                if matches!(state, BatchResult::Failed(_)) {
                                    *state = BatchResult::Pending;
                                }
                            }
                            this.start_batch(cx);
                        })),
                )
                .child(
                    Button::new("batch-new")
                        .ghost()
                        .disabled(self.busy)
                        .label("清空結果")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.batch.clear();
                            cx.notify();
                        })),
                ),
        );
        if !self.batch.is_empty() {
            content = content.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("已處理")
                    .child(crate::ascii::progress(
                        batch_completed(&self.batch),
                        self.batch.len(),
                    )),
            );
        }
        let current = self
            .batch
            .iter()
            .position(|(_, result)| matches!(result, BatchResult::Pending));
        for (index, (input, result)) in self.batch.iter().enumerate() {
            let processing = self.busy && current == Some(index);
            let text = match result {
                BatchResult::Pending if processing => "處理中…".into(),
                BatchResult::Pending => "待處理".into(),
                BatchResult::Added(id) => format!("已新增 #{id}"),
                BatchResult::Existing(id) => format!("已存在 #{id}"),
                BatchResult::Failed(error) => format!("失敗：{error}"),
            };
            content = content.child(
                div()
                    .p_2()
                    .rounded_md()
                    .bg(cx.theme().muted)
                    .text_sm()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(processing, |row| {
                        row.child(crate::ascii::loading(("batch-item-wait", index)))
                    })
                    .child(format!("{input}　{text}")),
            );
        }
        content.into_any_element()
    }
    fn render_details(&mut self, id: u64, cx: &mut Context<Self>) -> AnyElement {
        let Some(room) = self
            .data
            .rooms
            .iter()
            .find(|r| r.saved.room_id == id)
            .cloned()
        else {
            return div()
                .child("此房間已移出監看，請重新載入。")
                .into_any_element();
        };
        let refresh = self.services.clone();
        let mut content = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(room.info.uname.clone()),
            )
            .child(div().text_sm().child(room.info.title.clone()))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "#{id} · {} · {}／{}\n人氣 {} · 關注 {}\n最後檢查 {}",
                        room.info.status.label(),
                        room.info.parent_area_name,
                        room.info.area_name,
                        room.info.online,
                        room.info.attention,
                        room.info.last_fetched_at
                    )),
            )
            .child(
                Button::new("details-refresh")
                    .outline()
                    .disabled(self.busy)
                    .label("重新檢查此房間")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let services = refresh.clone();
                        this.run(
                            async move {
                                for notice in services.live_watch.refresh_room(id).await? {
                                    services.poller.notify_notice(notice);
                                }
                                services.poller.notify_update();
                                services.live_watch.snapshot().await
                            },
                            cx,
                            |this, data, _| this.data = data,
                        );
                    })),
            )
            .child(field("備註", Input::new(&self.note)))
            .child(self.group_picker(cx))
            .child(
                Checkbox::new("details-pinned")
                    .label("置頂此房間")
                    .checked(self.pinned)
                    .on_click(cx.listener(|this, value: &bool, _, cx| {
                        this.pinned = *value;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new("details-notify")
                    .label("此房開播提醒")
                    .checked(self.notify)
                    .on_click(cx.listener(|this, value: &bool, _, cx| {
                        this.notify = *value;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        Button::new("details-save")
                            .primary()
                            .disabled(self.busy)
                            .label("儲存變更")
                            .on_click(cx.listener(|this, _, _, cx| this.save_details(cx))),
                    )
                    .child(
                        Button::new("details-reset")
                            .outline()
                            .disabled(self.busy)
                            .label("取消變更")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Some(room) =
                                    this.data.rooms.iter().find(|r| r.saved.room_id == id)
                                {
                                    this.group = room.saved.group_id.clone();
                                    this.pinned = room.saved.pinned;
                                    this.notify = room.saved.notify;
                                    this.note.update(cx, |s, cx| {
                                        s.set_value(room.saved.note.clone(), window, cx)
                                    });
                                    this.original = Some(room.saved.clone());
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("details-external")
                            .ghost()
                            .label("官方直播間")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.parent_action(window, cx, move |parent, _, cx| {
                                    parent.open_url(format!("https://live.bilibili.com/{id}"), cx)
                                })
                            })),
                    ),
            )
            .child(section("排列順序"));
        for (up, label) in [(true, "向上移動"), (false, "向下移動")] {
            content = content.child(
                Button::new(if up { "room-move-up" } else { "room-move-down" })
                    .outline()
                    .small()
                    .disabled(self.busy)
                    .label(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.change(
                            move |data| {
                                let room = data
                                    .rooms
                                    .iter()
                                    .find(|r| r.room_id == id)
                                    .cloned()
                                    .ok_or_else(|| anyhow!("房間不存在"))?;
                                let mut rows: Vec<_> = data
                                    .rooms
                                    .iter()
                                    .filter(|r| {
                                        r.group_id == room.group_id && r.pinned == room.pinned
                                    })
                                    .collect();
                                rows.sort_by_key(|r| r.order);
                                let mut ids: Vec<_> = rows.iter().map(|r| r.room_id).collect();
                                let index = ids.iter().position(|i| *i == id).unwrap();
                                let next = if up {
                                    index.saturating_sub(1)
                                } else {
                                    (index + 1).min(ids.len() - 1)
                                };
                                ids.swap(index, next);
                                data.reorder_rooms(ids, room.group_id, room.pinned)
                            },
                            cx,
                        )
                    })),
            );
        }
        content = content.child(section("觀測場次")).child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("只包含本工具可靠觀測的期間；未知或中斷時長不推估。"),
        );
        for row in history_rows(
            &self.data.data,
            (0, i64::MAX),
            Some(id),
            bili_core::now_ms(),
        )
        .into_iter()
        .take(50)
        {
            content = content.child(
                div()
                    .p_2()
                    .rounded_md()
                    .bg(cx.theme().muted)
                    .text_xs()
                    .child(format!(
                        "{}　{}　{}",
                        row.started_at,
                        row.state,
                        observed_duration(row.seconds)
                    )),
            );
        }
        content = content
            .child(
                Button::new("details-history")
                    .outline()
                    .label("查詢與匯出")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.parent_action(window, cx, move |parent, window, cx| {
                            parent.open_live_panel(PanelKind::History(Some(id)), window, cx)
                        })
                    })),
            )
            .child(
                Button::new("details-delete")
                    .outline()
                    .label(if self.delete_confirm {
                        "確認移出監看（保留歷史）"
                    } else {
                        "移出監看"
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if this.delete_confirm {
                            this.parent_action(window, cx, move |parent, window, cx| {
                                parent.remove_live_rooms(vec![id], cx);
                                parent.close_live_panel(window, cx);
                            });
                        } else {
                            this.delete_confirm = true;
                            cx.notify();
                        }
                    })),
            );
        content.into_any_element()
    }
    fn render_groups(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut content = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(field("分組名稱", Input::new(&self.text)))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("group-save")
                            .primary()
                            .disabled(self.busy)
                            .label(if self.group.is_some() {
                                "儲存名稱"
                            } else {
                                "新增分組"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                let id = this.group.clone();
                                let name = value(&this.text, cx);
                                this.change(
                                    move |d| {
                                        d.save_group(id, name)?;
                                        Ok(())
                                    },
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new("group-new")
                            .ghost()
                            .label("新增另一組")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.group = None;
                                this.text.update(cx, |s, cx| s.set_value("", window, cx));
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("group-reload")
                            .ghost()
                            .disabled(self.busy)
                            .label("重新載入")
                            .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                    ),
            );
        for (index, group) in self.data.data.groups.clone().into_iter().enumerate() {
            let id = group.id.clone();
            let edit = group.clone();
            let remove = id.clone();
            let mut row = div()
                .p_2()
                .rounded_md()
                .bg(cx.theme().muted)
                .flex()
                .items_center()
                .gap_2()
                .child(
                    Button::new(("group-edit", index))
                        .ghost()
                        .flex_1()
                        .label(group.name)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.group = Some(edit.id.clone());
                            this.text
                                .update(cx, |s, cx| s.set_value(edit.name.clone(), window, cx));
                            cx.notify();
                        })),
                );
            for (up, label) in [(true, "↑"), (false, "↓")] {
                let mut ids: Vec<_> = self.data.data.groups.iter().map(|g| g.id.clone()).collect();
                let next = if up {
                    index.saturating_sub(1)
                } else {
                    (index + 1).min(ids.len() - 1)
                };
                ids.swap(index, next);
                row = row.child(
                    Button::new(SharedString::from(format!("group-order-{index}-{up}")))
                        .ghost()
                        .small()
                        .label(label)
                        .disabled(self.busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let ids = ids.clone();
                            this.change(move |d| d.reorder_groups(ids), cx);
                        })),
                );
            }
            row = row.child(
                Button::new(("group-remove", index))
                    .ghost()
                    .small()
                    .label("移除分組")
                    .disabled(self.busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let id = remove.clone();
                        this.change(move |d| d.remove_group(&id), cx);
                    })),
            );
            content = content.child(row);
        }
        content
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("移除分組會將房間移至未分組，房間與觀測紀錄仍會保留。"),
            )
            .into_any_element()
    }
    fn render_watchlists(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut content = div().flex().flex_col().gap_3();
        if let Some(watch) = self.watch.clone() {
            content = content
                .child(field("清單名稱", Input::new(&self.text)))
                .child(field("用途說明", Input::new(&self.note)))
                .child(field("搜尋可選房間", Input::new(&self.search)));
            let mut controls = div().flex().flex_wrap().gap_2();
            for (index, layout) in [
                MultiViewLayout::Auto,
                MultiViewLayout::Grid2,
                MultiViewLayout::Grid3,
                MultiViewLayout::Focus,
            ]
            .into_iter()
            .enumerate()
            {
                controls = controls.child(
                    Button::new(("watch-layout", index))
                        .small()
                        .outline()
                        .selected(watch.layout == layout)
                        .label(layout.label())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(w) = &mut this.watch {
                                w.layout = layout.clone();
                            }
                            cx.notify();
                        })),
                );
            }
            for limit in [1, 2, 4, 9] {
                controls = controls.child(
                    Button::new(("watch-limit", limit))
                        .small()
                        .outline()
                        .selected(watch.playback_limit == limit)
                        .label(format!("{limit} 間"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(w) = &mut this.watch {
                                w.playback_limit = limit;
                            }
                            cx.notify();
                        })),
                );
            }
            content = content.child(controls).child(section(&format!(
                "播放順序（已選 {}／50）",
                self.selected.len()
            )));
            for (index, id) in self.selected.clone().into_iter().enumerate() {
                let name = self
                    .data
                    .rooms
                    .iter()
                    .find(|r| r.saved.room_id == id)
                    .map(|r| format!("{} · {}", r.info.uname, r.info.status.label()))
                    .unwrap_or_else(|| "已移出監看".into());
                let mut row = div().flex().items_center().gap_2().child(
                    div()
                        .flex_1()
                        .text_sm()
                        .child(format!("{}. {name} · #{id}", index + 1)),
                );
                for (up, label) in [(true, "↑"), (false, "↓")] {
                    row = row.child(
                        Button::new(SharedString::from(format!("watch-order-{id}-{up}")))
                            .ghost()
                            .small()
                            .label(label)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let next = if up {
                                    index.saturating_sub(1)
                                } else {
                                    (index + 1).min(this.selected.len() - 1)
                                };
                                this.selected.swap(index, next);
                                cx.notify();
                            })),
                    );
                }
                row = row.child(
                    Button::new(("watch-unselect", id))
                        .ghost()
                        .small()
                        .label("移除")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.selected.retain(|r| *r != id);
                            cx.notify();
                        })),
                );
                content = content.child(row);
            }
            content = content.child(self.room_choices(cx)).child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        Button::new("watch-save")
                            .primary()
                            .disabled(self.busy)
                            .label("儲存觀看清單")
                            .on_click(cx.listener(|this, _, _, cx| this.save_watchlist(cx))),
                    )
                    .child(
                        Button::new("watch-copy")
                            .outline()
                            .label("另存副本")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(w) = &mut this.watch {
                                    w.id.clear();
                                }
                                this.save_watchlist(cx);
                            })),
                    )
                    .child(
                        Button::new("watch-list-back")
                            .ghost()
                            .label("返回清單")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.watch = None;
                                cx.notify();
                            })),
                    ),
            );
            if !watch.id.is_empty() {
                let id = watch.id;
                content = content.child(
                    Button::new("watch-delete")
                        .outline()
                        .disabled(self.busy)
                        .label(if self.delete_confirm {
                            "確認移除這份清單"
                        } else {
                            "移除清單"
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.delete_confirm {
                                this.delete_confirm = true;
                                cx.notify();
                                return;
                            }
                            let id = id.clone();
                            this.change(
                                move |d| {
                                    d.watchlists.retain(|w| w.id != id);
                                    Ok(())
                                },
                                cx,
                            );
                            this.watch = None;
                        })),
                );
            }
        } else {
            content = content.child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("watch-new")
                            .primary()
                            .label("建立觀看清單")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.edit_watchlist(Watchlist::default(), window, cx)
                            })),
                    )
                    .child(
                        Button::new("watch-reload")
                            .outline()
                            .disabled(self.busy)
                            .label("重新載入")
                            .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                    ),
            );
            if self.data.data.watchlists.is_empty() {
                content = content.child(
                    div()
                        .text_sm()
                        .child("尚未建立觀看清單。可保存比賽夜、背景音樂或日常觀看組合。"),
                );
            }
            for (index, list) in self.data.data.watchlists.clone().into_iter().enumerate() {
                let edit = list.clone();
                let view = list.clone();
                let play = list.clone();
                content = content.child(
                    div()
                        .p_3()
                        .rounded_lg()
                        .bg(cx.theme().muted)
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(list.name))
                        .child(div().text_xs().child(format!(
                            "{} 間 · {} · 最多 {} 間播放",
                            list.room_ids.len(),
                            list.layout.label(),
                            list.playback_limit
                        )))
                        .child(div().text_sm().child(list.description))
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(
                                    Button::new(("watch-edit", index))
                                        .outline()
                                        .small()
                                        .label("編輯")
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.edit_watchlist(edit.clone(), window, cx)
                                        })),
                                )
                                .child(
                                    Button::new(("watch-view", index))
                                        .ghost()
                                        .small()
                                        .label("在工作台查看")
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            let list = view.clone();
                                            this.parent_action(
                                                window,
                                                cx,
                                                move |parent, window, cx| {
                                                    parent.live.filter.only_ids =
                                                        Some(list.room_ids.into_iter().collect());
                                                    parent.live.statistics = false;
                                                    parent.switch_view(View::Live, window, cx);
                                                    parent.filter_live(true, cx);
                                                    parent.close_live_panel(window, cx);
                                                },
                                            );
                                        })),
                                )
                                .child(
                                    Button::new(("watch-play", index))
                                        .primary()
                                        .small()
                                        .label("開始同屏")
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            let list = play.clone();
                                            this.parent_action(
                                                window,
                                                cx,
                                                move |parent, window, cx| {
                                                    parent.live.statistics = false;
                                                    parent.switch_view(View::Live, window, cx);
                                                    parent.start_multi(
                                                        list.room_ids,
                                                        list.layout,
                                                        list.playback_limit,
                                                        window,
                                                        cx,
                                                    );
                                                    parent.close_live_panel(window, cx);
                                                },
                                            );
                                        })),
                                ),
                        ),
                );
            }
        }
        content.into_any_element()
    }
    fn render_history(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut content = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        "依裝置當地日期查詢，含起訖日。只累計可靠觀測期間；沒有紀錄不代表未開播。",
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(field("開始日期", Input::new(&self.from)))
                    .child(field("結束日期", Input::new(&self.to))),
            );
        if matches!(self.kind, PanelKind::History(None)) {
            content = content.child(field("房號（留空查全部）", Input::new(&self.aux)));
        }
        let mut states = div().flex().flex_wrap().gap_2();
        for (index, state) in ["全部", "完整", "觀測中斷", "進行中", "結束待確認"]
            .into_iter()
            .enumerate()
        {
            states = states.child(
                Button::new(("history-state", index))
                    .outline()
                    .small()
                    .selected(self.history_state == state)
                    .label(state)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.history_state = state.into();
                        cx.notify();
                    })),
            );
        }
        content = content.child(states).child(
            Button::new("history-csv")
                .primary()
                .disabled(self.busy)
                .label("匯出全部符合的 CSV")
                .on_click(cx.listener(|this, _, _, cx| match this.history_rows(cx) {
                    Ok(rows) => {
                        this.save_file(history_csv(&rows).into_bytes(), "直播觀測紀錄.csv", cx)
                    }
                    Err(error) => {
                        this.error = Some(error.to_string());
                        cx.notify();
                    }
                })),
        );
        match self.history_rows(cx) {
            Ok(rows) => {
                content = content.child(
                    div()
                        .text_sm()
                        .child(format!("符合 {} 筆；以下預覽前 20 筆。", rows.len())),
                );
                for row in rows.into_iter().take(20) {
                    content = content.child(
                        div()
                            .p_2()
                            .rounded_md()
                            .bg(cx.theme().muted)
                            .text_xs()
                            .child(format!(
                                "{} · #{}\n{} → {}\n{} · {}",
                                row.name,
                                row.room_id,
                                row.started_at,
                                row.ended_at.unwrap_or_else(|| "尚未確認結束".into()),
                                row.state,
                                observed_duration(row.seconds)
                            )),
                    );
                }
            }
            Err(error) => content = content.child(div().text_sm().child(error.to_string())),
        }
        content.into_any_element()
    }
    fn render_batch_manage(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let all: Vec<u64> = self.data.rooms.iter().map(|r| r.saved.room_id).collect();
        let count = all.len();
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(field("搜尋房間", Input::new(&self.search)))
            .child(
                Button::new("batch-select-all")
                    .outline()
                    .label(format!("選取全部 {count} 間"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected = all.clone();
                        cx.notify();
                    })),
            )
            .child(self.room_choices(cx))
            .child(section(&format!("已選取 {} 間", self.selected.len())))
            .child(self.group_picker(cx))
            .child(
                Button::new("batch-move")
                    .primary()
                    .disabled(self.busy || self.selected.is_empty())
                    .label("移動至所選分組")
                    .on_click(cx.listener(|this, _, _, cx| {
                        let ids = this.selected.clone();
                        let group = this.group.clone();
                        this.change(
                            move |d| {
                                for id in ids {
                                    d.patch_room(
                                        id,
                                        RoomPatch {
                                            group_id: Some(group.clone()),
                                            ..Default::default()
                                        },
                                    )?;
                                }
                                Ok(())
                            },
                            cx,
                        );
                    })),
            )
            .child(
                Button::new("batch-delete")
                    .outline()
                    .disabled(self.busy || self.selected.is_empty())
                    .label(if self.delete_confirm {
                        "確認移出所選直播監看"
                    } else {
                        "移出所選直播監看"
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        if !this.delete_confirm {
                            this.delete_confirm = true;
                            cx.notify();
                            return;
                        }
                        let ids = this.selected.clone();
                        this.parent_action(window, cx, move |parent, window, cx| {
                            parent.remove_live_rooms(ids, cx);
                            parent.close_live_panel(window, cx);
                        });
                    })),
            )
            .into_any_element()
    }
}
pub(crate) fn observed_duration(seconds: Option<u64>) -> String {
    match seconds {
        None => "待確認".into(),
        Some(s) if s < 60 => format!("{s} 秒"),
        Some(s) if s < 3600 => format!("{} 分", s / 60),
        Some(s) => format!("{} 小時 {} 分", s / 3600, s % 3600 / 60),
    }
}

impl Render for LivePanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut content = div().p_4().flex().flex_col().gap_4().w_full();
        if self.busy && !matches!(self.kind, PanelKind::BatchAdd) {
            content = content.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::ascii::working("panel-wait"))
                    .child("處理中…"),
            );
        }
        if let Some(error) = &self.error {
            content = content.child(
                div()
                    .p_3()
                    .rounded_md()
                    .bg(ui::tone_colors(Tone::Danger, cx.theme().is_dark()).bg)
                    .text_sm()
                    .child(error.clone()),
            );
        }
        if let Some(success) = &self.success {
            content = content.child(
                div()
                    .text_sm()
                    .text_color(ui::tone_colors(Tone::Success, cx.theme().is_dark()).fg)
                    .child(success.clone()),
            );
        }
        let body=match self.kind.clone(){PanelKind::BatchAdd=>self.render_batch(cx),PanelKind::Details(id)=>self.render_details(id,cx),PanelKind::Groups=>self.render_groups(cx),PanelKind::Watchlists|PanelKind::SaveCurrent=>self.render_watchlists(cx),PanelKind::History(_)=>self.render_history(cx),PanelKind::Notifications=>self.render_notifications(cx),PanelKind::Preferences=>self.render_preferences(cx),PanelKind::QuickFind=>self.render_quick_find(cx),PanelKind::Shortcuts=>div().text_sm().child("Ctrl+K：快速尋找\nCtrl+1–4：切換主頁\nR／F5：重新整理\nS：搜尋房間\nA：新增房間\nEsc：關閉最上層面板或退出預覽／同屏\n?：快捷鍵說明\n\n輸入文字與播放器操作優先。排序可在房間資訊內使用上下移動。").into_any_element(),PanelKind::Filters=>self.render_filters(cx),PanelKind::BatchManage=>self.render_batch_manage(cx)};
        let _ = window;
        content = content.child(body);
        content
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn saving_live_preferences_preserves_unedited_custom_poll_interval() {
        let mut follow = AppSettings {
            poll_interval_live_sec: 45,
            ..Default::default()
        };
        let mut live = LiveSettings::default();
        apply_live_poll_interval(&mut follow, &mut live, None);
        assert_eq!(follow.poll_interval_live_sec, 45);
        assert_eq!(live.refresh_interval, 60);
        apply_live_poll_interval(&mut follow, &mut live, Some(180));
        assert_eq!(follow.poll_interval_live_sec, 180);
        assert_eq!(live.refresh_interval, 180);
        follow.poll_interval_live_sec = 300;
        apply_live_poll_interval(&mut follow, &mut live, None);
        assert_eq!(follow.poll_interval_live_sec, 300);
        assert_eq!(live.refresh_interval, 300);
    }

    #[test]
    fn batch_progress_counts_settled_results_and_keeps_pending_after_stop_or_retry() {
        let mut batch = vec![
            ("1".into(), BatchResult::Added(100)),
            ("2".into(), BatchResult::Existing(200)),
            ("3".into(), BatchResult::Failed("測試失敗".into())),
            ("4".into(), BatchResult::Pending),
        ];
        assert_eq!(batch_completed(&batch), 3);
        batch[2].1 = BatchResult::Pending;
        assert_eq!(batch_completed(&batch), 2);
        assert_eq!(batch_completed(&[]), 0);
    }
}
