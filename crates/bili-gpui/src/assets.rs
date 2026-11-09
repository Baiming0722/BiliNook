use gpui::{AssetSource, Result, SharedString};
use std::borrow::Cow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppIcon {
    BilibiliTv,
    Feed,
    Video,
    Live,
    Settings,
    Pin,
    Columns,
    Rows,
    Users,
    UserPlus,
    Bell,
    Database,
    MessageSquare,
    Share,
    Heart,
    CheckRead,
    LogOut,
    FolderOpen,
    Image,
    Sparkles,
}

impl AppIcon {
    pub const ALL: [Self; 20] = [
        Self::BilibiliTv,
        Self::Feed,
        Self::Video,
        Self::Live,
        Self::Settings,
        Self::Pin,
        Self::Columns,
        Self::Rows,
        Self::Users,
        Self::UserPlus,
        Self::Bell,
        Self::Database,
        Self::MessageSquare,
        Self::Share,
        Self::Heart,
        Self::CheckRead,
        Self::LogOut,
        Self::FolderOpen,
        Self::Image,
        Self::Sparkles,
    ];

    pub const fn as_path(self) -> &'static str {
        match self {
            Self::BilibiliTv => "app-icons/bilibili-tv.svg",
            Self::Feed => "app-icons/feed.svg",
            Self::Video => "app-icons/video.svg",
            Self::Live => "app-icons/live.svg",
            Self::Settings => "app-icons/settings.svg",
            Self::Pin => "app-icons/pin.svg",
            Self::Columns => "app-icons/columns.svg",
            Self::Rows => "app-icons/rows.svg",
            Self::Users => "app-icons/users.svg",
            Self::UserPlus => "app-icons/user-plus.svg",
            Self::Bell => "app-icons/bell.svg",
            Self::Database => "app-icons/database.svg",
            Self::MessageSquare => "app-icons/message-square.svg",
            Self::Share => "app-icons/share.svg",
            Self::Heart => "app-icons/heart.svg",
            Self::CheckRead => "app-icons/check-read.svg",
            Self::LogOut => "app-icons/log-out.svg",
            Self::FolderOpen => "app-icons/folder-open.svg",
            Self::Image => "app-icons/image.svg",
            Self::Sparkles => "app-icons/sparkles.svg",
        }
    }

    const fn bytes(self) -> &'static [u8] {
        match self {
            Self::BilibiliTv => include_bytes!("../assets/icons/bilibili-tv.svg"),
            Self::Feed => include_bytes!("../assets/icons/feed.svg"),
            Self::Video => include_bytes!("../assets/icons/video.svg"),
            Self::Live => include_bytes!("../assets/icons/live.svg"),
            Self::Settings => include_bytes!("../assets/icons/settings.svg"),
            Self::Pin => include_bytes!("../assets/icons/pin.svg"),
            Self::Columns => include_bytes!("../assets/icons/columns.svg"),
            Self::Rows => include_bytes!("../assets/icons/rows.svg"),
            Self::Users => include_bytes!("../assets/icons/users.svg"),
            Self::UserPlus => include_bytes!("../assets/icons/user-plus.svg"),
            Self::Bell => include_bytes!("../assets/icons/bell.svg"),
            Self::Database => include_bytes!("../assets/icons/database.svg"),
            Self::MessageSquare => include_bytes!("../assets/icons/message-square.svg"),
            Self::Share => include_bytes!("../assets/icons/share.svg"),
            Self::Heart => include_bytes!("../assets/icons/heart.svg"),
            Self::CheckRead => include_bytes!("../assets/icons/check-read.svg"),
            Self::LogOut => include_bytes!("../assets/icons/log-out.svg"),
            Self::FolderOpen => include_bytes!("../assets/icons/folder-open.svg"),
            Self::Image => include_bytes!("../assets/icons/image.svg"),
            Self::Sparkles => include_bytes!("../assets/icons/sparkles.svg"),
        }
    }
}

impl gpui::component::IconNamed for AppIcon {
    fn path(self) -> SharedString {
        self.as_path().into()
    }
}

/// 先解析專案內嵌 `app-icons/*` SVG，未命中再委派給 `gpui::assets::Assets`，
/// 避免 `gpui::assets::Assets::load` 對未知路徑直接回傳 `Err` 中斷查詢。
pub struct AppAssets;

impl AppAssets {
    fn lookup_custom(path: &str) -> Option<&'static [u8]> {
        AppIcon::ALL
            .into_iter()
            .find(|icon| icon.as_path() == path)
            .map(AppIcon::bytes)
    }
}

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path.is_empty() {
            return Ok(None);
        }
        if let Some(bytes) = Self::lookup_custom(path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        gpui::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui::assets::Assets.list(path).unwrap_or_default();
        paths.extend(
            AppIcon::ALL
                .into_iter()
                .map(AppIcon::as_path)
                .filter(|item| item.starts_with(path))
                .map(SharedString::from),
        );
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_custom_svg_icons_are_embedded_and_valid() {
        let source = AppAssets;
        for icon in AppIcon::ALL {
            let bytes = source
                .load(icon.as_path())
                .expect("asset load should succeed")
                .expect("custom icon must exist");
            let content = std::str::from_utf8(&bytes).expect("svg must be valid utf-8");
            assert!(
                content.contains("<svg") && content.contains("</svg>"),
                "invalid svg content for {}",
                icon.as_path()
            );
        }
    }

    #[test]
    fn built_in_kit_icons_remain_accessible() {
        let source = AppAssets;
        let search = source
            .load("icons/search.svg")
            .expect("built-in icon load should not fail");
        assert!(search.is_some());
    }
}
