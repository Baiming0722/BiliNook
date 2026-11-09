use anyhow::{Context, Result, ensure};
use lru::LruCache;
use std::{num::NonZeroUsize, sync::mpsc};
use windows::{
    Data::Xml::Dom::XmlDocument,
    Foundation::TypedEventHandler,
    UI::Notifications::{
        NotificationSetting, ToastFailedEventArgs, ToastNotification, ToastNotificationManager,
        ToastNotifier,
    },
    core::{IInspectable, Interface, h},
};

// Windows 依這個固定 ID 保存通知偏好，顯示名稱則使用 APP_NAME。
pub const APP_ID: &str = "BilibiliFollow.RustGPUI";
#[derive(Clone)]
pub enum Target { External(String), LiveRoom(u64) }
pub enum Response {
    Open(Option<Target>),
    Failed(String),
}

pub struct Notifications {
    notifier: Option<ToastNotifier>,
    active: LruCache<String, ToastNotification>,
    sender: mpsc::SyncSender<Response>,
    receiver: mpsc::Receiver<Response>,
}
impl Notifications {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::sync_channel(128);
        Self {
            notifier: None,
            active: LruCache::new(NonZeroUsize::new(128).unwrap()),
            sender,
            receiver,
        }
    }
    pub fn responses(&self) -> impl Iterator<Item = Response> + '_ {
        self.receiver.try_iter().take(64)
    }
    pub fn show(
        &mut self,
        tag: String,
        title: &str,
        body: &str,
        target: Option<Target>,
        sound: bool,
    ) -> Result<()> {
        if self.notifier.is_none() {
            let key = windows_registry::CURRENT_USER
                .create(format!(r"Software\Classes\AppUserModelId\{APP_ID}"))
                .context("無法登錄 Windows 通知應用名稱")?;
            key.set_string("DisplayName", bili_core::APP_NAME)?;
            self.notifier = Some(
                ToastNotificationManager::CreateToastNotifierWithId(&APP_ID.into())
                    .context("建立 Windows ToastNotifier 失敗")?,
            );
        }
        let notifier = self.notifier.as_ref().unwrap();
        match notifier.Setting() {
            Ok(setting) => ensure!(
                setting == NotificationSetting::Enabled,
                "Windows 已停用此應用的系統通知"
            ),
            // 第一次傳送前，未封裝應用的通知設定項目可能尚未由 Shell 建立。
            Err(error) if error.code().0 as u32 == 0x80070490 => {}
            Err(error) => return Err(error).context("讀取 Windows 通知設定失敗"),
        }
        let document = XmlDocument::new()?;
        let root = document.CreateElement(h!("toast"))?;
        document.AppendChild(&root)?;
        let visual = document.CreateElement(h!("visual"))?;
        root.AppendChild(&visual)?;
        let binding = document.CreateElement(h!("binding"))?;
        binding.SetAttribute(h!("template"), h!("ToastGeneric"))?;
        visual.AppendChild(&binding)?;
        for text in [title, body] {
            let element = document.CreateElement(h!("text"))?;
            element.SetInnerText(&text.into())?;
            binding.AppendChild(&element)?;
        }
        if target.is_some() {
            let actions = document.CreateElement(h!("actions"))?;
            let action = document.CreateElement(h!("action"))?;
            action.SetAttribute(h!("content"), h!("在 B 站開啟"))?;
            action.SetAttribute(h!("arguments"), h!("open"))?;
            actions.AppendChild(&action)?;
            root.AppendChild(&actions)?;
        }
        // 讓 Windows 控制音效與勿擾模式；GPUI 目前的通知 API 固定靜音且不回傳錯誤。
        if !sound {
            let audio = document.CreateElement(h!("audio"))?;
            audio.SetAttribute(h!("silent"), h!("true"))?;
            root.AppendChild(&audio)?;
        }
        let toast = ToastNotification::CreateToastNotification(&document)?;
        toast.SetTag(&tag.clone().into())?;
        toast.SetGroup(h!("bili-follow"))?;
        let sender = self.sender.clone();
        toast.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(
            move |_, args| {
                let open_external=args.as_ref().and_then(|args|args.cast::<windows::UI::Notifications::ToastActivatedEventArgs>().ok()).and_then(|args|args.Arguments().ok()).is_some_and(|args|args=="open");
                let destination=match &target {Some(Target::LiveRoom(id)) if open_external=>Some(Target::External(format!("https://live.bilibili.com/{id}"))),other=>other.clone()};
                if sender.try_send(Response::Open(destination)).is_err() {
                    eprintln!("[通知] 點擊回應佇列已滿");
                }
                Ok(())
            },
        ))?;
        let sender = self.sender.clone();
        toast.Failed(
            &TypedEventHandler::<ToastNotification, ToastFailedEventArgs>::new(move |_, args| {
                let detail = args
                    .as_ref()
                    .and_then(|args| args.ErrorCode().ok())
                    .map(|code| format!("{code:?}"))
                    .unwrap_or_default();
                if sender
                    .try_send(Response::Failed(format!("Windows 通知傳送失敗：{detail}")))
                    .is_err()
                {
                    eprintln!("[通知] 失敗回應佇列已滿");
                }
                Ok(())
            }),
        )?;
        if let Some(old) = self.active.pop(&tag) {
            notifier.Hide(&old)?;
        }
        notifier.Show(&toast).context("傳送 Windows Toast 失敗")?;
        if let Some((_, old)) = self.active.push(tag, toast) {
            notifier.Hide(&old)?;
        }
        Ok(())
    }
}
