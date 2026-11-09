//! 直播工作台領域：原版 JSON 相容模型、可靠觀測與原生背景服務。
mod data;
mod history;
mod service;
pub use data::*;
pub use history::*;
pub use service::*;

use anyhow::{Result, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_SAFE_ID: u64 = 9_007_199_254_740_991;
pub const MAX_BACKUP_BYTES: usize = crate::store::MAX_JSON_BYTES as usize;
pub const MAX_ROOMS: usize = 5000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoomStatus {
    Live,
    Offline,
    Round,
    Locked,
    Hidden,
    Error,
    #[default]
    Unknown,
}
impl RoomStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Live => "直播中",
            Self::Offline => "未開播",
            Self::Round => "輪播中",
            Self::Locked => "鎖定",
            Self::Hidden => "隱藏",
            Self::Error => "錯誤",
            Self::Unknown => "未知",
        }
    }
    pub fn reliable(self) -> bool {
        !matches!(self, Self::Error | Self::Unknown)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StoredRoom {
    pub room_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    pub order: i64,
    pub added_at: String,
    pub note: String,
    pub pinned: bool,
    pub notify: bool,
}
impl Default for StoredRoom {
    fn default() -> Self {
        Self {
            room_id: 0,
            short_id: None,
            group_id: None,
            order: 0,
            added_at: iso(0),
            note: String::new(),
            pinned: false,
            notify: true,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ObservedRoom {
    pub room_id: u64,
    pub short_id: Option<u64>,
    pub uid: u64,
    pub title: String,
    pub uname: String,
    pub cover: String,
    pub face: String,
    pub online: u64,
    pub attention: u64,
    pub area_name: String,
    pub parent_area_name: String,
    pub live_time: String,
    pub status: RoomStatus,
    pub last_fetched_at: String,
    pub error: Option<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayRoom {
    pub saved: StoredRoom,
    pub info: ObservedRoom,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RoomGroup {
    pub id: String,
    pub name: String,
    pub order: i64,
    pub created_at: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LiveSession {
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub incomplete: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Observation {
    pub status: RoomStatus,
    pub observed_at: String,
    pub run_id: String,
    pub interrupted: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LiveStats {
    pub live_duration: BTreeMap<u64, u64>,
    pub last_live_at: BTreeMap<u64, String>,
    pub history: BTreeMap<u64, Vec<LiveSession>>,
    pub observations: BTreeMap<u64, Observation>,
    pub last_calculated_at: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MultiViewLayout {
    #[default]
    Auto,
    #[serde(rename = "2x2")]
    Grid2,
    #[serde(rename = "3x3")]
    Grid3,
    Focus,
}
impl MultiViewLayout {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Auto => "自適應",
            Self::Grid2 => "四宮格",
            Self::Grid3 => "九宮格",
            Self::Focus => "焦點版面",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Watchlist {
    pub id: String,
    pub name: String,
    pub description: String,
    pub room_ids: Vec<u64>,
    pub layout: MultiViewLayout,
    pub playback_limit: usize,
    pub created_at: String,
    pub updated_at: String,
}
impl Default for Watchlist {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            description: String::new(),
            room_ids: vec![],
            layout: MultiViewLayout::Auto,
            playback_limit: 4,
            created_at: iso(0),
            updated_at: iso(0),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AdvancedFilters {
    pub area: String,
    pub parent_area: String,
    pub online_min: u64,
    pub attention_min: u64,
    pub sort_by: String,
    pub sort_dir: String,
}
impl Default for AdvancedFilters {
    fn default() -> Self {
        Self {
            area: "all".into(),
            parent_area: "all".into(),
            online_min: 0,
            attention_min: 0,
            sort_by: "order".into(),
            sort_dir: "asc".into(),
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FilterPreset {
    pub id: String,
    pub name: String,
    pub filters: AdvancedFilters,
    pub sort_by: String,
    pub sort_order: String,
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AppearanceSettings {
    pub accent_color: String,
    pub accent_custom: Option<String>,
    pub card_scale: f32,
    pub font_scale: f32,
    pub font_family: String,
    pub density: String,
    pub ascii_animation_enabled: bool,
}
impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            accent_color: "pink".into(),
            accent_custom: None,
            card_scale: 1.,
            font_scale: 1.,
            font_family: "jhenghei".into(),
            density: "standard".into(),
            ascii_animation_enabled: true,
        }
    }
}
impl AppearanceSettings {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.card_scale.is_finite()
                && (0.5..=2.).contains(&self.card_scale)
                && self.font_scale.is_finite()
                && (0.5..=2.).contains(&self.font_scale),
            "縮放須介於 0.5–2 倍"
        );
        ensure!(
            ["blue", "pink", "purple", "green", "orange"].contains(&self.accent_color.as_str()),
            "主題色無效"
        );
        ensure!(
            ["compact", "standard", "comfortable"].contains(&self.density.as_str()),
            "密度無效"
        );
        ensure!(
            !self.font_family.trim().is_empty() && self.font_family.len() <= 200,
            "字體名稱無效"
        );
        if let Some(color) = &self.accent_custom {
            ensure!(
                (color.len() == 7 || color.len() == 9)
                    && color.starts_with('#')
                    && color[1..].bytes().all(|b| b.is_ascii_hexdigit()),
                "自訂色彩須為十六進位色碼"
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LiveSettings {
    pub theme: String,
    pub card_scale: f32,
    pub font_scale: f32,
    pub font_family: String,
    pub refresh_interval: u64,
    pub accent_color: String,
    pub accent_custom: Option<String>,
    pub notify_on_live: bool,
    pub sound_enabled: bool,
    pub sound_volume: f32,
    pub density: String,
    pub room_view: String,
    pub filter_presets: Vec<FilterPreset>,
}
impl Default for LiveSettings {
    fn default() -> Self {
        Self {
            theme: "auto".into(),
            card_scale: 1.,
            font_scale: 1.,
            font_family: "system".into(),
            refresh_interval: 60,
            accent_color: "blue".into(),
            accent_custom: None,
            notify_on_live: true,
            sound_enabled: true,
            sound_volume: 0.5,
            density: "standard".into(),
            room_view: "grid".into(),
            filter_presets: vec![],
        }
    }
}
impl LiveSettings {
    pub fn sync_appearance(&mut self, settings: &crate::models::AppSettings) {
        let a = &settings.appearance;
        self.theme = match settings.theme.as_str() {
            "light" => "light",
            "system" => "auto",
            _ => "dark",
        }
        .into();
        self.card_scale = a.card_scale;
        self.font_scale = a.font_scale;
        self.font_family = a.font_family.clone();
        self.density = a.density.clone();
        self.accent_color = a.accent_color.clone();
        self.accent_custom = a.accent_custom.clone();
    }
    pub fn appearance(&self) -> AppearanceSettings {
        AppearanceSettings {
            accent_color: self.accent_color.clone(),
            accent_custom: self.accent_custom.clone(),
            card_scale: self.card_scale,
            font_scale: self.font_scale,
            font_family: self.font_family.clone(),
            density: self.density.clone(),
            ..Default::default()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NotificationPreferences {
    pub snooze_until: u64,
    pub quiet_enabled: bool,
    pub quiet_start: String,
    pub quiet_end: String,
}
impl Default for NotificationPreferences {
    fn default() -> Self {
        Self {
            snooze_until: 0,
            quiet_enabled: false,
            quiet_start: "23:00".into(),
            quiet_end: "08:00".into(),
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LiveNotice {
    pub id: String,
    pub room_id: u64,
    pub name: String,
    pub title: String,
    pub at: String,
    pub read: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TrackingSource {
    pub manual: bool,
    pub up_ids: BTreeSet<u64>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NativeLiveData {
    pub revision: u64,
    pub tracking: BTreeMap<u64, TrackingSource>,
    pub excluded_up_ids: BTreeSet<u64>,
    pub notices: Vec<LiveNotice>,
    pub preferences: NotificationPreferences,
    pub ui_sound_enabled: bool,
    pub migration: String,
    pub migration_error: Option<String>,
    pub migrated_appearance: Option<LiveSettings>,
    pub cached_rooms: BTreeMap<u64, ObservedRoom>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LiveWatchData {
    pub version: u32,
    pub rooms: Vec<StoredRoom>,
    pub groups: Vec<RoomGroup>,
    pub watchlists: Vec<Watchlist>,
    pub settings: LiveSettings,
    pub stats: LiveStats,
    pub native: NativeLiveData,
}
impl Default for LiveWatchData {
    fn default() -> Self {
        Self {
            version: 2,
            rooms: vec![],
            groups: vec![],
            watchlists: vec![],
            settings: LiveSettings::default(),
            stats: LiveStats::default(),
            native: NativeLiveData::default(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct LiveWatchSnapshot {
    pub data: std::sync::Arc<LiveWatchData>,
    pub rooms: Vec<DisplayRoom>,
}
impl LiveWatchSnapshot {
    pub fn new(data: std::sync::Arc<LiveWatchData>) -> Self {
        let rooms = data
            .rooms
            .iter()
            .map(|saved| DisplayRoom {
                saved: saved.clone(),
                info: data
                    .native
                    .cached_rooms
                    .get(&saved.room_id)
                    .cloned()
                    .unwrap_or_else(|| ObservedRoom {
                        room_id: saved.room_id,
                        uname: format!("房間 {}", saved.room_id),
                        ..Default::default()
                    }),
            })
            .collect();
        Self { data, rooms }
    }
}
impl Default for LiveWatchSnapshot {
    fn default() -> Self {
        Self::new(std::sync::Arc::new(LiveWatchData::default()))
    }
}

#[derive(Clone, Debug, Default)]
pub struct RoomPatch {
    pub group_id: Option<Option<String>>,
    pub note: Option<String>,
    pub pinned: Option<bool>,
    pub notify: Option<bool>,
}

pub fn iso(ms: u64) -> String {
    DateTime::<Utc>::from_timestamp_millis(ms.min(i64::MAX as u64) as i64)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
pub fn timestamp(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|v| v.timestamp_millis())
}
pub fn valid_id(id: u64) -> bool {
    id > 0 && id <= MAX_SAFE_ID
}
pub fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 80
        && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}
pub fn new_id(prefix: &str) -> String {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!(
        "{prefix}-{}-{}",
        crate::now_ms(),
        SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}
pub fn parse_room_input(input: &str) -> Result<u64> {
    let value = input.trim();
    let number = if value.bytes().all(|b| b.is_ascii_digit()) {
        value.to_owned()
    } else {
        let url = url::Url::parse(
            &if value.starts_with("https://") || value.starts_with("http://") {
                value.to_owned()
            } else {
                format!("https://{value}")
            },
        )?;
        ensure!(
            [Some("live.bilibili.com"), Some("www.live.bilibili.com")].contains(&url.host_str())
                && ["http", "https"].contains(&url.scheme()),
            "請輸入 B 站直播網址或房號"
        );
        let parts: Vec<_> = url
            .path_segments()
            .map(|v| v.filter(|s| !s.is_empty()).collect())
            .unwrap_or_default();
        let part = if parts.first() == Some(&"blanc") {
            parts.get(1)
        } else {
            parts.first()
        };
        part.copied().unwrap_or_default().to_owned()
    };
    let id = number
        .parse::<u64>()
        .map_err(|_| anyhow::anyhow!("無法解析直播房號"))?;
    ensure!(valid_id(id), "直播房號須為有效正整數");
    Ok(id)
}

pub fn normalize_info(id: u64, value: &Value, at: u64) -> Result<ObservedRoom> {
    let data = &value["data"]["by_room_ids"];
    let info = data
        .get(id.to_string())
        .or_else(|| {
            data.as_object().and_then(|v| {
                v.values().find(|v| {
                    v["room_id"].as_u64() == Some(id) || v["short_id"].as_u64() == Some(id)
                })
            })
        })
        .ok_or_else(|| anyhow::anyhow!("直播 API 未回傳房間 {id}"))?;
    let num = |key: &str| {
        info[key]
            .as_u64()
            .or_else(|| info[key].as_str().and_then(|s| s.parse().ok()))
            .unwrap_or(0)
    };
    let text = |key: &str| info[key].as_str().unwrap_or_default().to_owned();
    let room_id = num("room_id");
    ensure!(valid_id(room_id), "直播 API 回傳無效房號");
    let status = if num("hidden_status") != 0 {
        RoomStatus::Hidden
    } else if num("lock_status") != 0 || info["is_encrypted"].as_bool() == Some(true) {
        RoomStatus::Locked
    } else {
        match info["live_status"]
            .as_u64()
            .or_else(|| info["live_status"].as_str().and_then(|s| s.parse().ok()))
        {
            Some(0) => RoomStatus::Offline,
            Some(1) => RoomStatus::Live,
            Some(2) => RoomStatus::Round,
            _ => RoomStatus::Unknown,
        }
    };
    Ok(ObservedRoom {
        room_id,
        short_id: (num("short_id") > 0).then(|| num("short_id")),
        uid: num("uid"),
        title: text("title"),
        uname: text("uname"),
        cover: text("cover"),
        face: text("face"),
        online: num("online"),
        attention: num("attention"),
        area_name: text("area_name"),
        parent_area_name: text("parent_area_name"),
        live_time: text("live_time"),
        status,
        last_fetched_at: iso(at),
        error: None,
    })
}
