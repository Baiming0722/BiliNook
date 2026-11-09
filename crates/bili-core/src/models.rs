use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub type Extra = BTreeMap<String, Value>;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UpUser {
    pub mid: u64,
    pub name: String,
    pub face: String,
    pub sign: String,
    pub sex: String,
    pub level: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_senior_member: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub official: Option<Official>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live_room: Option<LiveRoom>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_pinned: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin_order: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notify_on_live: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notify_on_video: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notify_on_dynamic: Option<bool>,
    pub created_at: u64,
    pub updated_at: u64,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Official {
    pub role: i32,
    pub title: String,
    pub desc: String,
    pub r#type: i32,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LiveRoom {
    pub room_status: u32,
    pub live_status: u32,
    pub url: String,
    pub title: String,
    pub cover: String,
    pub online: u64,
    pub roomid: u64,
    pub round_status: u32,
    pub broadcast_type: u32,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct VideoItem {
    pub bvid: String,
    pub aid: u64,
    pub title: String,
    pub description: String,
    pub pic: String,
    pub duration: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub length_seconds: Option<u64>,
    pub pubdate: u64,
    pub created: u64,
    pub play: u64,
    pub video_review: u64,
    pub comment: u64,
    pub author: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_face: Option<String>,
    pub mid: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_union_video: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review: Option<u64>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DynamicType {
    Video,
    Article,
    Draw,
    Text,
    Forward,
    Live,
    #[default]
    #[serde(other)]
    Other,
}

impl DynamicType {
    pub fn label(self) -> &'static str {
        match self {
            Self::Video => "影片",
            Self::Article => "專欄",
            Self::Draw => "圖文",
            Self::Text => "文字",
            Self::Forward => "轉發",
            Self::Live => "直播",
            Self::Other => "其他",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DynamicItem {
    #[serde(rename = "id_str")]
    pub id_str: String,
    pub r#type: DynamicType,
    pub raw_type: Option<String>,
    pub mid: u64,
    pub author_name: String,
    pub author_face: String,
    pub pub_time: u64,
    pub pub_action: Option<String>,
    pub text: Option<String>,
    pub title: Option<String>,
    pub cover: Option<String>,
    pub images: Option<Vec<String>>,
    pub jump_url: Option<String>,
    pub bvid: Option<String>,
    pub live_info: Option<DynamicLiveInfo>,
    pub comment_id: Option<String>,
    pub comment_type: Option<u32>,
    pub stat: Option<DynamicStat>,
    pub orig: Option<Box<DynamicItem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reservation: Option<DynamicReservation>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DynamicReservation {
    pub title: String,
    pub time: Option<String>,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DynamicLiveInfo {
    pub room_id: u64,
    pub title: String,
    pub cover: String,
    pub area_name: String,
    pub watched_show: Option<String>,
    pub live_status: Option<u32>,
    pub link: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DynamicStat {
    pub reply: u64,
    pub like: u64,
    pub forward: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CommentEmote {
    pub url: String,
    pub size: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CommentItem {
    pub rpid: String,
    pub mid: u64,
    pub uname: String,
    pub avatar: String,
    pub level: u32,
    pub pub_time: u64,
    pub like: u64,
    pub message: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub emotes: BTreeMap<String, CommentEmote>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pictures: Vec<String>,
    pub sub_replies: Option<Vec<CommentItem>>,
    pub sub_reply_count: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CommentsResult {
    pub list: Vec<CommentItem>,
    pub total: u64,
    pub has_more: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LiveStatusInfo {
    pub uid: u64,
    pub room_id: u64,
    pub title: String,
    pub cover_from_user: String,
    pub keyframe: String,
    pub live_status: u32,
    pub live_time: String,
    pub online: u64,
    pub area_name: String,
    pub parent_area_name: String,
    pub uname: Option<String>,
    pub face: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FollowGroup {
    pub id: String,
    pub name: String,
    pub mids: Vec<u64>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AppSettings {
    pub appearance: crate::live_watch::AppearanceSettings,
    pub poll_interval_live_sec: u64,
    pub poll_interval_dynamic_sec: u64,
    pub enable_notifications: bool,
    pub notify_live: bool,
    pub notify_video: bool,
    pub notify_dynamic: bool,
    pub sound_enabled: bool,
    pub theme: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_proxy_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub follow_groups: Option<Vec<FollowGroup>>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            appearance: crate::live_watch::AppearanceSettings::default(),
            poll_interval_live_sec: 60,
            poll_interval_dynamic_sec: 300,
            enable_notifications: true,
            notify_live: true,
            notify_video: true,
            notify_dynamic: true,
            sound_enabled: true,
            theme: "dark".into(),
            custom_proxy_url: None,
            follow_groups: None,
            extra: Extra::new(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PollerState {
    pub last_live_check_time: u64,
    pub last_dynamic_check_time: u64,
    pub known_live_uids: Vec<u64>,
    pub known_dynamic_ids: BTreeMap<String, Vec<String>>,
    pub known_video_bvids: BTreeMap<String, Vec<String>>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Snapshot {
    pub ups: Vec<UpUser>,
    pub settings: AppSettings,
    pub state: PollerState,
    pub read_dynamics: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DynamicPage {
    pub items: Vec<DynamicItem>,
    pub offset: String,
    pub has_more: bool,
    pub warning: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct VideoPage {
    pub list: Vec<VideoItem>,
    pub total: u64,
    pub has_more: bool,
    pub total_estimated: bool,
    pub warning: Option<String>,
}
