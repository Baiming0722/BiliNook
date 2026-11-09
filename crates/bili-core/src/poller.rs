use crate::{api::BiliApi, models::*, now_ms, store::JsonStore};
use anyhow::{Result, anyhow, bail};
use futures::{StreamExt, stream};
use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
    time::Duration,
};
use tokio::sync::{Mutex, Notify, broadcast};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeKind {
    Live,
    Video,
    Dynamic,
}

#[derive(Clone, Debug)]
pub struct Notice {
    pub kind: NoticeKind,
    pub mid: u64,
    pub title: String,
    pub body: String,
    pub url: String,
}

#[derive(Clone, Debug)]
pub enum CoreEvent {
    Updated,
    Notice(Notice),
    Error(String),
    Polling { kind: &'static str, active: bool },
}

pub struct Poller {
    api: Arc<dyn BiliApi>,
    store: JsonStore,
    events: broadcast::Sender<CoreEvent>,
    live_guard: Mutex<()>,
    dynamic_guard: Mutex<()>,
    changed: Notify,
    live_watch: Option<Arc<crate::live_watch::LiveWatchService>>,
}

impl Poller {
    pub fn new(api: Arc<dyn BiliApi>, store: JsonStore) -> Self {
        let (events, _) = broadcast::channel(128);
        Self {
            api,
            store,
            events,
            live_guard: Mutex::new(()),
            dynamic_guard: Mutex::new(()),
            changed: Notify::new(),
            live_watch: None,
        }
    }

    pub fn with_live_watch(mut self, service: Arc<crate::live_watch::LiveWatchService>) -> Self {
        self.live_watch = Some(service);
        self
    }

    pub fn subscribe(&self) -> broadcast::Receiver<CoreEvent> {
        self.events.subscribe()
    }
    pub fn reconfigure(&self) {
        self.changed.notify_waiters();
    }

    pub fn notify_update(&self) {
        self.emit(CoreEvent::Updated);
    }
    pub fn notify_notice(&self, notice: Notice) {
        self.emit(CoreEvent::Notice(notice));
    }
    fn emit(&self, event: CoreEvent) {
        let _ = self.events.send(event);
    }

    pub async fn run(self: Arc<Self>, stop: CancellationToken) {
        // 兩條計時鏈互相獨立；大量 UP 主的動態請求不會阻塞直播巡檢。
        tokio::join!(
            self.poll_loop(true, stop.clone()),
            self.poll_loop(false, stop)
        );
    }

    async fn poll_loop(&self, live: bool, stop: CancellationToken) {
        loop {
            let result = tokio::select! {
                biased;
                _ = stop.cancelled() => break,
                result = async { if live { self.poll_live().await } else { self.poll_dynamic().await } } => result,
            };
            if let Err(error) = result {
                self.emit(CoreEvent::Error(error.to_string()));
            }
            let started = tokio::time::Instant::now();
            loop {
                let settings = match self.store.snapshot().await {
                    Ok(snapshot) => snapshot.settings,
                    Err(error) => {
                        self.emit(CoreEvent::Error(error.to_string()));
                        break;
                    }
                };
                let seconds = if live {
                    settings.poll_interval_live_sec.max(30)
                } else {
                    settings.poll_interval_dynamic_sec.max(60)
                };
                let deadline = started + Duration::from_secs(seconds);
                tokio::select! {
                    _ = stop.cancelled() => return,
                    _ = self.changed.notified() => continue,
                    _ = tokio::time::sleep_until(deadline) => break,
                }
            }
        }
    }

    pub async fn refresh(&self) -> Result<()> {
        let (live, dynamic) = tokio::join!(self.poll_live_with(true), self.poll_dynamic());
        let mut errors = Vec::new();
        if let Err(error) = live {
            errors.push(error.to_string());
        }
        if let Err(error) = dynamic {
            errors.push(error.to_string());
        }
        if !errors.is_empty() {
            bail!("{}", errors.join("；"));
        }
        Ok(())
    }

    pub async fn poll_live(&self) -> Result<bool> {
        self.poll_live_with(false).await
    }
    pub async fn refresh_live(&self) -> Result<()> {
        self.poll_live_with(true).await.map(|_| ())
    }
    async fn poll_live_with(&self, fresh: bool) -> Result<bool> {
        let _guard = if fresh {
            self.live_guard.lock().await
        } else {
            let Ok(guard) = self.live_guard.try_lock() else {
                return Ok(false);
            };
            guard
        };
        self.emit(CoreEvent::Polling {
            kind: "直播",
            active: true,
        });
        let result = self.check_live(fresh).await;
        self.emit(CoreEvent::Polling {
            kind: "直播",
            active: false,
        });
        result
    }

    async fn check_live(&self, fresh: bool) -> Result<bool> {
        if let Some(service) = &self.live_watch {
            let notices = service.refresh(fresh).await?;
            self.emit(CoreEvent::Updated);
            for notice in notices {
                self.emit(CoreEvent::Notice(notice));
            }
            return Ok(true);
        }
        let snapshot = self.store.snapshot().await?;
        if snapshot.ups.is_empty() {
            return Ok(true);
        }
        let statuses = self
            .api
            .live(snapshot.ups.iter().map(|up| up.mid).collect())
            .await?;
        let notices = self
            .store
            .update(move |snapshot| Ok(reconcile_live(snapshot, &statuses, now_ms())))
            .await?;
        self.emit(CoreEvent::Updated);
        for notice in notices {
            self.emit(CoreEvent::Notice(notice));
        }
        Ok(true)
    }

    pub async fn poll_dynamic(&self) -> Result<bool> {
        let Ok(_guard) = self.dynamic_guard.try_lock() else {
            return Ok(false);
        };
        self.emit(CoreEvent::Polling {
            kind: "動態",
            active: true,
        });
        let result = self.check_dynamic().await;
        self.emit(CoreEvent::Polling {
            kind: "動態",
            active: false,
        });
        result
    }

    async fn check_dynamic(&self) -> Result<bool> {
        let snapshot = self.store.snapshot().await?;
        if snapshot.ups.is_empty() {
            return Ok(true);
        }
        let uids: Vec<u64> = snapshot.ups.into_iter().map(|up| up.mid).collect();
        let results = stream::iter(uids)
            .map(|mid| async move { (mid, self.api.dynamics(mid, String::new()).await) })
            .buffer_unordered(4)
            .collect::<Vec<_>>()
            .await;
        let mut pages = BTreeMap::new();
        let mut failures = Vec::new();
        for (mid, result) in results {
            match result {
                Ok(page) if page.warning.is_none() => {
                    pages.insert(mid, page);
                }
                Ok(page) => {
                    failures.push(format!("UP {mid}：{}", page.warning.unwrap_or_default()))
                }
                Err(error) => failures.push(format!("UP {mid}：{error}")),
            }
        }
        let notices = self
            .store
            .update(move |snapshot| Ok(reconcile_dynamics(snapshot, &pages, now_ms())))
            .await?;
        self.emit(CoreEvent::Updated);
        for notice in notices {
            self.emit(CoreEvent::Notice(notice));
        }
        if !failures.is_empty() {
            return Err(anyhow!("部分動態輪詢失敗：{}", failures.join("；")));
        }
        Ok(true)
    }
}

pub fn reconcile_live(
    snapshot: &mut Snapshot,
    statuses: &BTreeMap<u64, LiveStatusInfo>,
    timestamp: u64,
) -> Vec<Notice> {
    let previous: HashSet<_> = snapshot.state.known_live_uids.iter().copied().collect();
    let mut current = Vec::new();
    let mut notices = Vec::new();
    for up in &mut snapshot.ups {
        let Some(live) = statuses.get(&up.mid) else {
            if previous.contains(&up.mid) {
                current.push(up.mid);
            }
            continue;
        };
        if live.live_status == 1 {
            current.push(up.mid);
            if !previous.contains(&up.mid)
                && snapshot.settings.enable_notifications
                && snapshot.settings.notify_live
                && up.notify_on_live != Some(false)
            {
                notices.push(Notice {
                    kind: NoticeKind::Live,
                    mid: up.mid,
                    title: format!("{} 開播了", up.name),
                    body: live.title.clone(),
                    url: format!("https://live.bilibili.com/{}", live.room_id),
                });
            }
        }
        let mut room = up.live_room.clone().unwrap_or_default();
        room.room_status = u32::from(live.room_id > 0);
        room.live_status = live.live_status;
        room.url = format!("https://live.bilibili.com/{}", live.room_id);
        room.title = live.title.clone();
        room.cover = if !live.cover_from_user.is_empty() {
            live.cover_from_user.clone()
        } else {
            live.keyframe.clone()
        };
        room.online = live.online;
        room.roomid = live.room_id;
        up.live_room = Some(room);
    }
    snapshot.state.known_live_uids = current;
    snapshot.state.last_live_check_time = timestamp;
    notices
}

pub fn reconcile_dynamics(
    snapshot: &mut Snapshot,
    pages: &BTreeMap<u64, DynamicPage>,
    timestamp: u64,
) -> Vec<Notice> {
    let mut notices = Vec::new();
    for up in &snapshot.ups {
        let Some(page) = pages.get(&up.mid).filter(|page| page.warning.is_none()) else {
            continue;
        };
        let key = up.mid.to_string();
        let known = snapshot
            .state
            .known_dynamic_ids
            .entry(key.clone())
            .or_default();
        let videos = snapshot.state.known_video_bvids.entry(key).or_default();
        let initial = known.is_empty();
        for item in &page.items {
            let new = !known.contains(&item.id_str);
            if new {
                known.push(item.id_str.clone());
            }
            let new_video = item.bvid.as_ref().is_none_or(|bvid| !videos.contains(bvid));
            if new && !initial && snapshot.settings.enable_notifications {
                let video = item.r#type == DynamicType::Video;
                if (video
                    && new_video
                    && snapshot.settings.notify_video
                    && up.notify_on_video != Some(false))
                    || (!video
                        && snapshot.settings.notify_dynamic
                        && up.notify_on_dynamic != Some(false))
                {
                    notices.push(Notice {
                        kind: if video {
                            NoticeKind::Video
                        } else {
                            NoticeKind::Dynamic
                        },
                        mid: up.mid,
                        title: format!(
                            "{} 發布了{}",
                            up.name,
                            if video { "新影片" } else { "新動態" }
                        ),
                        body: item
                            .title
                            .clone()
                            .or_else(|| item.text.clone())
                            .unwrap_or_default(),
                        url: item
                            .jump_url
                            .clone()
                            .unwrap_or_else(|| format!("https://t.bilibili.com/{}", item.id_str)),
                    });
                }
            }
            if let Some(bvid) = &item.bvid
                && !videos.contains(bvid)
            {
                videos.push(bvid.clone());
            }
        }
        let excess = known.len().saturating_sub(50);
        known.drain(..excess);
        let excess = videos.len().saturating_sub(50);
        videos.drain(..excess);
    }
    snapshot.state.last_dynamic_check_time = timestamp;
    notices
}
