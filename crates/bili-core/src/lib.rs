//! BiliNook 的資料、持久化與背景服務。此 crate 不依賴 GPUI。

pub const APP_NAME: &str = "BiliNook";

pub mod api;
pub mod feed;
pub mod groups;
pub mod images;
pub mod live_watch;
pub mod models;
pub mod poller;
pub mod runtime;
pub mod services;
pub mod store;
pub mod wbi;

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
