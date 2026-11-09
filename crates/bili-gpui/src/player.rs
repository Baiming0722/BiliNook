//! 官方播放器的原生子視窗。所有 COM 與視窗操作均留在 GPUI 的 STA 執行緒。
use anyhow::{Context, Result, ensure};
use gpui::*;
use raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, WindowHandle};
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    sync::mpsc,
};
use webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG;
use webview2_com::{
    CapturePreviewCompletedHandler, ContainsFullScreenElementChangedEventHandler,
    NavigationCompletedEventHandler,
};
use windows::Win32::{
    Graphics::Gdi::{CreateRectRgn, SetWindowRgn},
    System::Com::{STREAM_SEEK_SET, StructuredStorage::CreateStreamOnHGlobal},
};
use wry::{NewWindowResponse, WebContext, WebView, WebViewBuilder, WebViewExtWindows};

#[derive(Debug)]
pub enum PlayerEvent {
    Loaded(u64),
    Error(u64, String),
    Snapshot(u64, Vec<u8>),
    Fullscreen(u64, bool),
    Shortcut(String),
    Open(String),
}

struct Player {
    webview: WebView,
    danmaku: bool,
    rect: Option<[i32; 8]>,
    seen: bool,
}

pub struct PlayerHost {
    context: Option<WebContext>,
    players: BTreeMap<u64, Player>,
    pending: BTreeMap<u64, PlayerRequest>,
    queue: VecDeque<PlayerRequest>,
    next_request: u64,
    worker_running: bool,
    sender: mpsc::SyncSender<PlayerEvent>,
    receiver: mpsc::Receiver<PlayerEvent>,
    covered: bool,
    pub fullscreen: Option<u64>,
    pub error: Option<String>,
    pub failed_room: Option<u64>,
}

#[derive(Clone, Copy)]
struct ParentHandle(RawWindowHandle);
impl HasWindowHandle for ParentHandle {
    fn window_handle(&self) -> std::result::Result<WindowHandle<'_>, HandleError> {
        // The app keeps its window alive while a player is being constructed.
        Ok(unsafe { WindowHandle::borrow_raw(self.0) })
    }
}

#[derive(Clone, Copy)]
struct PlayerRequest {
    id: u64,
    danmaku: bool,
    parent: ParentHandle,
    serial: u64,
}

pub struct PlayerJob {
    request: PlayerRequest,
    context: WebContext,
    sender: mpsc::SyncSender<PlayerEvent>,
}

pub struct PlayerJobResult {
    request: PlayerRequest,
    context: WebContext,
    player: Result<Player>,
}

impl PlayerJob {
    pub fn run(mut self) -> PlayerJobResult {
        let player = create_player(&mut self.context, &self.sender, self.request);
        PlayerJobResult {
            request: self.request,
            context: self.context,
            player,
        }
    }
}

impl PlayerHost {
    pub fn new(directory: PathBuf) -> Self {
        let (sender, receiver) = mpsc::sync_channel(128);
        Self {
            context: Some(WebContext::new(Some(directory))),
            players: BTreeMap::new(),
            pending: BTreeMap::new(),
            queue: VecDeque::new(),
            next_request: 0,
            worker_running: false,
            sender,
            receiver,
            covered: false,
            fullscreen: None,
            error: None,
            failed_room: None,
        }
    }

    pub fn events(&self) -> impl Iterator<Item = PlayerEvent> + '_ {
        self.receiver.try_iter().take(64)
    }
    pub fn contains(&self, id: u64) -> bool {
        self.players.contains_key(&id)
    }
    pub fn clear(&mut self) {
        self.failed_room = None;
        self.fullscreen = None;
        self.players.clear();
        self.pending.clear();
        self.queue.clear();
    }
    pub fn remove(&mut self, id: u64) {
        self.players.remove(&id);
        self.pending.remove(&id);
        self.queue.retain(|request| request.id != id);
    }
    pub fn retain(&mut self, ids: &[u64]) {
        self.players.retain(|id, _| ids.contains(id));
        self.pending.retain(|id, _| ids.contains(id));
        self.queue.retain(|request| ids.contains(&request.id));
    }
    pub fn begin_frame(&mut self) {
        for player in self.players.values_mut() {
            player.seen = false;
        }
    }
    pub fn finish_frame(&self) {
        for player in self.players.values() {
            if !player.seen {
                let _ = player.webview.set_visible(false);
            }
        }
    }

    /// Queues creation; the caller runs jobs outside all GPUI and PlayerHost borrows.
    pub fn request(&mut self, id: u64, danmaku: bool, window: &Window) -> Result<bool> {
        ensure!(id > 0, "直播房號無效");
        if let Some(player) = self.players.get_mut(&id) {
            if player.danmaku != danmaku {
                player.webview.load_url(&player_url(id, danmaku))?;
                player.danmaku = danmaku;
            }
            return Ok(false);
        }
        let parent = ParentHandle(HasWindowHandle::window_handle(window)?.as_raw());
        self.queue_request(id, danmaku, parent)
    }

    fn queue_request(&mut self, id: u64, danmaku: bool, parent: ParentHandle) -> Result<bool> {
        if self
            .pending
            .get(&id)
            .is_some_and(|request| request.danmaku == danmaku)
        {
            return Ok(false);
        }
        ensure!(
            self.pending.contains_key(&id) || self.players.len() + self.pending.len() < 9,
            "最多同時載入九間直播"
        );
        self.next_request = self.next_request.wrapping_add(1);
        let request = PlayerRequest {
            id,
            danmaku,
            parent,
            serial: self.next_request,
        };
        self.queue.retain(|queued| queued.id != id);
        self.pending.insert(id, request);
        self.queue.push_back(request);
        let start = !self.worker_running;
        self.worker_running = true;
        Ok(start)
    }

    pub fn next_job(&mut self) -> Option<PlayerJob> {
        while let Some(request) = self.queue.pop_front() {
            if self
                .pending
                .get(&request.id)
                .is_some_and(|pending| pending.serial == request.serial)
            {
                return Some(PlayerJob {
                    request,
                    context: self.context.take().expect("player creation is serialized"),
                    sender: self.sender.clone(),
                });
            }
        }
        self.worker_running = false;
        None
    }

    pub fn creating(&self) -> bool {
        self.context.is_none()
    }

    pub fn finish_job(&mut self, result: PlayerJobResult) -> Option<(u64, String)> {
        let PlayerJobResult {
            request,
            context,
            player,
        } = result;
        self.context = Some(context);
        if !self
            .pending
            .get(&request.id)
            .is_some_and(|pending| pending.serial == request.serial)
        {
            return None;
        }
        self.pending.remove(&request.id);
        match player {
            Ok(player) => {
                self.players.insert(request.id, player);
                if self.failed_room == Some(request.id) {
                    self.error = None;
                    self.failed_room = None;
                }
                None
            }
            Err(error) => {
                self.failed_room = Some(request.id);
                Some((request.id, format!("{error:#}")))
            }
        }
    }

    pub fn cover(&mut self, covered: bool) {
        if self.covered == covered {
            return;
        }
        self.covered = covered;
        for (id, player) in &self.players {
            if covered {
                if let Err(error) = capture(*id, &player.webview, self.sender.clone()) {
                    eprintln!("[播放器 {id}] 快照失敗：{error:#}");
                }
                let _ = player.webview.set_visible(false);
            }
        }
    }

    pub fn exit_fullscreen(&self) {
        for player in self.players.values() {
            let _ = player
                .webview
                .evaluate_script("if(document.fullscreenElement)document.exitFullscreen()");
        }
    }

    #[cfg(debug_assertions)]
    pub fn trace_media(&self) {
        for (id, player) in &self.players {
            let id = *id;
            let _=player.webview.evaluate_script_with_callback("JSON.stringify([...document.querySelectorAll('video')].map(v=>({ready:v.readyState,paused:v.paused,time:Math.round(v.currentTime*10)/10,width:v.videoWidth,height:v.videoHeight,muted:v.muted,volume:v.volume,total:v.getVideoPlaybackQuality?.().totalVideoFrames,dropped:v.getVideoPlaybackQuality?.().droppedVideoFrames})))",move|result|eprintln!("[播放器媒體 {id}] {result}"));
        }
    }

    pub fn layout(&mut self, id: u64, bounds: Bounds<Pixels>, clip: Bounds<Pixels>, scale: f32) {
        let Some(player) = self.players.get_mut(&id) else {
            return;
        };
        player.seen = true;
        let intersection = bounds.intersect(&clip);
        let visible = intersection.size.width > px(1.)
            && intersection.size.height > px(1.)
            && !self.covered
            && self.fullscreen.is_none_or(|active| active == id);
        let x = f32::from(bounds.origin.x) * scale;
        let y = f32::from(bounds.origin.y) * scale;
        let width = (f32::from(bounds.size.width) * scale).max(1.);
        let height = (f32::from(bounds.size.height) * scale).max(1.);
        let left = ((f32::from(intersection.origin.x) * scale - x).max(0.)).round() as i32;
        let top = ((f32::from(intersection.origin.y) * scale - y).max(0.)).round() as i32;
        let right = left + (f32::from(intersection.size.width) * scale).max(0.).round() as i32;
        let bottom = top
            + (f32::from(intersection.size.height) * scale)
                .max(0.)
                .round() as i32;
        let rect = [
            x.round() as i32,
            y.round() as i32,
            width.round() as i32,
            height.round() as i32,
            left,
            top,
            right,
            bottom,
        ];
        if player.rect != Some(rect) {
            if std::env::var_os("BILI_PLAYER_TRACE").is_some() {
                eprintln!("[播放器 {id}] rect={rect:?} visible={visible} clip={clip:?}");
            }
            if let Err(error) = player.webview.set_bounds(wry::Rect {
                position: wry::dpi::PhysicalPosition::new(rect[0], rect[1]).into(),
                size: wry::dpi::PhysicalSize::new(rect[2] as u32, rect[3] as u32).into(),
            }) {
                self.error = Some(format!("播放器尺寸更新失敗：{error}"));
            }
            unsafe {
                // 成功時區域所有權移交系統；失敗時由我們釋放。
                let region = CreateRectRgn(left, top, right, bottom);
                if SetWindowRgn(player.webview.hwnd(), Some(region), true) == 0 {
                    let _ = windows::Win32::Graphics::Gdi::DeleteObject(region.into());
                }
            }
            player.rect = Some(rect);
        }
        let _ = player.webview.set_visible(visible);
    }
}

fn create_player(
    context: &mut WebContext,
    sender: &mpsc::SyncSender<PlayerEvent>,
    request: PlayerRequest,
) -> Result<Player> {
    let PlayerRequest {
        id,
        danmaku,
        parent,
        ..
    } = request;
    let loaded = sender.clone();
    let navigation = sender.clone();
    let popup = sender.clone();
    let ipc = sender.clone();
    let webview = WebViewBuilder::new_with_web_context(context)
        .with_url(player_url(id, danmaku))
        .with_visible(false)
        .with_background_color((9, 13, 22, 255))
        .with_autoplay(true)
        .with_on_page_load_handler(move |event, _| {
            if matches!(event, wry::PageLoadEvent::Finished) {
                let _ = loaded.try_send(PlayerEvent::Loaded(id));
            }
        })
        .with_navigation_handler(move |url| {
            if is_player_url(&url) {
                true
            } else {
                if is_bili_url(&url) {
                    let _ = navigation.try_send(PlayerEvent::Open(url));
                }
                false
            }
        })
        .with_new_window_req_handler(move |url, _| {
            if is_bili_url(&url) {
                let _ = popup.try_send(PlayerEvent::Open(url));
            }
            NewWindowResponse::Deny
        })
        .with_initialization_script(
            r#"document.addEventListener('keydown',e=>{
              const k=e.key.toLowerCase();
              if((e.ctrlKey&&['1','2','3','4','k','q'].includes(k))||k==='escape'){
                window.ipc.postMessage(JSON.stringify({key:e.ctrlKey?'ctrl-'+k:k}));
              }
            },true);"#,
        )
        .with_ipc_handler(move |request| {
            if !is_bili_url(&request.uri().to_string()) {
                return;
            }
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(request.body())
                && let Some(key) = value["key"].as_str()
                && [
                    "ctrl-1", "ctrl-2", "ctrl-3", "ctrl-4", "ctrl-k", "ctrl-q", "escape",
                ]
                .contains(&key)
            {
                let _ = ipc.try_send(PlayerEvent::Shortcut(key.to_owned()));
            }
        })
        .build_as_child(&parent)
        .context("無法建立官方播放器；請確認已安裝 Microsoft Edge WebView2 Runtime")?;
    let fullscreen = sender.clone();
    unsafe {
        let core = webview.controller().CoreWebView2()?;
        let mut token = 0;
        core.add_ContainsFullScreenElementChanged(
            &ContainsFullScreenElementChangedEventHandler::create(Box::new(move |core, _| {
                if let Some(core) = core {
                    let mut active = windows::core::BOOL::default();
                    core.ContainsFullScreenElement(&mut active)?;
                    let _ = fullscreen.try_send(PlayerEvent::Fullscreen(id, active.as_bool()));
                }
                Ok(())
            })),
            &mut token,
        )?;
        let errors = sender.clone();
        core.add_NavigationCompleted(
            &NavigationCompletedEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args {
                    let mut success = windows::core::BOOL::default();
                    args.IsSuccess(&mut success)?;
                    if !success.as_bool() {
                        let _ = errors.try_send(PlayerEvent::Error(
                            id,
                            "官方播放器頁面載入失敗，可收合後重試或開啟官方直播間".into(),
                        ));
                    }
                }
                Ok(())
            })),
            &mut token,
        )?;
    }
    Ok(Player {
        webview,
        danmaku,
        rect: None,
        seen: false,
    })
}

fn capture(id: u64, webview: &WebView, sender: mpsc::SyncSender<PlayerEvent>) -> Result<()> {
    unsafe {
        let stream = CreateStreamOnHGlobal(Default::default(), true)?;
        let read = stream.clone();
        webview.controller().CoreWebView2()?.CapturePreview(
            COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
            &stream,
            &CapturePreviewCompletedHandler::create(Box::new(move |status| {
                status?;
                let mut metadata = Default::default();
                read.Stat(&mut metadata, windows::Win32::System::Com::STATFLAG_NONAME)?;
                if metadata.cbSize > 16 * 1024 * 1024 {
                    return Ok(());
                }
                read.Seek(0, STREAM_SEEK_SET, None)?;
                let mut bytes = vec![0; metadata.cbSize as usize];
                let mut size = 0;
                read.Read(
                    bytes.as_mut_ptr().cast(),
                    bytes.len() as u32,
                    Some(&mut size),
                )
                .ok()?;
                bytes.truncate(size as usize);
                let _ = sender.try_send(PlayerEvent::Snapshot(id, bytes));
                Ok(())
            })),
        )?;
    }
    Ok(())
}

pub fn player_url(id: u64, danmaku: bool) -> String {
    format!(
        "https://www.bilibili.com/blackboard/live/live-mobile-playerV3.html?roomId={id}&autoplay=1&danmaku={}",
        u8::from(danmaku)
    )
}
fn is_bili_url(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|url| {
        url.scheme() == "https"
            && url
                .host_str()
                .is_some_and(|host| host == "bilibili.com" || host.ends_with(".bilibili.com"))
    })
}
fn is_player_url(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|url| {
        is_bili_url(value) && url.path() == "/blackboard/live/live-mobile-playerV3.html"
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroIsize;
    use std::prelude::v1::test;

    #[test]
    fn player_creation_queue_serializes_and_ignores_cancelled_results() -> Result<()> {
        let parent = ParentHandle(RawWindowHandle::Win32(
            raw_window_handle::Win32WindowHandle::new(NonZeroIsize::new(1).unwrap()),
        ));
        let mut host = PlayerHost::new(PathBuf::from("test-profile"));
        assert!(host.queue_request(1, true, parent)?);
        assert!(!host.queue_request(2, true, parent)?);
        let first = host.next_job().unwrap();
        assert_eq!(first.request.id, 1);
        assert!(host.creating());
        host.remove(1);
        assert!(!host.queue_request(1, false, parent)?);
        assert!(
            host.finish_job(PlayerJobResult {
                request: first.request,
                context: first.context,
                player: Err(anyhow::anyhow!("cancelled"))
            })
            .is_none()
        );
        assert!(host.failed_room.is_none());
        let second = host.next_job().unwrap();
        assert_eq!(second.request.id, 2);
        let failure = host.finish_job(PlayerJobResult {
            request: second.request,
            context: second.context,
            player: Err(anyhow::anyhow!("failure")),
        });
        assert_eq!(failure.map(|value| value.0), Some(2));
        let last = host.next_job().unwrap();
        assert_eq!(last.request.id, 1);
        assert!(!last.request.danmaku);
        host.clear();
        assert!(
            host.finish_job(PlayerJobResult {
                request: last.request,
                context: last.context,
                player: Err(anyhow::anyhow!("cancelled"))
            })
            .is_none()
        );
        assert!(host.next_job().is_none());
        assert!(!host.creating());
        assert!(host.queue_request(3, true, parent)?);
        Ok(())
    }
    #[test]
    fn player_navigation_is_restricted() {
        assert!(is_player_url(&player_url(123, true)));
        assert!(
            url::Url::parse(&player_url(123, true))
                .unwrap()
                .query_pairs()
                .any(|(key, value)| key == "autoplay" && value == "1")
        );
        assert!(!is_player_url("https://www.bilibili.com/video/1"));
        assert!(!is_bili_url("https://bilibili.com.attacker.example/"));
        assert!(!is_bili_url("file:///C:/secret"));
    }
}
