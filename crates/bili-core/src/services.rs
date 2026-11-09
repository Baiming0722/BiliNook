use crate::{
    api::ApiClient,
    feed::FeedService,
    groups::{sort_ups, validate_groups, validate_ups},
    images::ImageCache,
    models::*,
    now_ms,
    poller::Poller,
    store::{JsonStore, validate_app_settings},
};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

pub struct Services {
    pub live_watch: Arc<crate::live_watch::LiveWatchService>,
    pub store: JsonStore,
    pub api: Arc<ApiClient>,
    pub images: Arc<ImageCache>,
    pub poller: Arc<Poller>,
    pub feed: Arc<FeedService>,
    pub stop: CancellationToken,
    poll_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    resumed: AtomicBool,
}

impl Services {
    pub async fn open(directory: PathBuf) -> Result<Arc<Self>> {
        let store = JsonStore::open(directory).await?;
        let snapshot = store.snapshot().await?;
        let api = Arc::new(ApiClient::new(
            snapshot.settings.custom_proxy_url.as_deref(),
        )?);
        let images = Arc::new(
            ImageCache::new(
                store.directory().join("cache/images"),
                api.http().clone(),
                api.gate.clone(),
            )
            .await?,
        );
        let live_watch = Arc::new(crate::live_watch::LiveWatchService::new(
            api.clone(),
            store.clone(),
        ));
        let poller =
            Arc::new(Poller::new(api.clone(), store.clone()).with_live_watch(live_watch.clone()));
        let feed = Arc::new(FeedService::new(api.clone()));
        Ok(Arc::new(Self {
            live_watch,
            store,
            api,
            images,
            poller,
            feed,
            stop: CancellationToken::new(),
            poll_task: Mutex::new(None),
            resumed: AtomicBool::new(false),
        }))
    }

    pub async fn start(&self) {
        let mut task = self.poll_task.lock().await;
        if task.is_none() {
            let poller = self.poller.clone();
            let stop = self.stop.clone();
            *task = Some(tokio::spawn(poller.run(stop)));
        }
    }

    pub async fn snapshot(&self) -> Result<Snapshot> {
        let mut snapshot = self.store.snapshot().await?;
        sort_ups(&mut snapshot.ups);
        Ok(snapshot)
    }

    pub async fn shutdown(&self) -> Result<()> {
        self.stop.cancel();
        let mut poll_task = self.poll_task.lock().await;
        if let Some(task) = poll_task.take() {
            task.await?;
        }
        drop(poll_task);
        self.store.flush().await
    }

    /// 安全退出失敗後沿用原存檔 actor，避免再次取得同一資料目錄的寫入鎖。
    pub async fn resume_after_shutdown(&self) -> Result<Arc<Self>> {
        let poll_task = self.poll_task.lock().await;
        ensure!(
            self.stop.is_cancelled() && poll_task.is_none(),
            "背景服務尚未停止"
        );
        ensure!(
            !self.resumed.swap(true, Ordering::AcqRel),
            "背景服務已經恢復"
        );
        let resumed = Arc::new(Self {
            live_watch: self.live_watch.clone(),
            store: self.store.clone(),
            api: self.api.clone(),
            images: self.images.clone(),
            poller: self.poller.clone(),
            feed: self.feed.clone(),
            stop: CancellationToken::new(),
            poll_task: Mutex::new(None),
            resumed: AtomicBool::new(false),
        });
        drop(poll_task);
        resumed.start().await;
        Ok(resumed)
    }

    pub async fn add_up(&self, input: String) -> Result<UpUser> {
        let mid = self.api.resolve_up(&input).await?;
        let profile = self.api.get_profile(mid).await?;
        ensure!(profile.mid == mid, "B 站回傳的 UID 不符");
        let user = self
            .store
            .update_both(move |snapshot, live| {
                let mut user = snapshot
                    .ups
                    .iter()
                    .find(|up| up.mid == mid)
                    .cloned()
                    .unwrap_or_else(|| UpUser {
                        mid,
                        created_at: now_ms(),
                        ..Default::default()
                    });
                user.name = profile.name;
                user.face = profile.face;
                user.sign = profile.sign;
                user.sex = profile.sex;
                user.level = profile.level;
                user.official = profile.official;
                if profile.live_room.is_some() {
                    user.live_room = profile.live_room;
                }
                user.updated_at = now_ms();
                if let Some(old) = snapshot.ups.iter_mut().find(|up| up.mid == mid) {
                    *old = user.clone();
                } else {
                    snapshot.ups.insert(0, user.clone());
                }
                validate_ups(&snapshot.ups)?;
                live.sync_ups(&snapshot.ups);
                Ok(user)
            })
            .await?;
        self.feed.invalidate().await;
        self.poller.notify_update();
        Ok(user)
    }

    pub async fn update_up(&self, mid: u64, patch: UpPatch) -> Result<()> {
        self.store
            .update_both(move |snapshot, live| {
                let up = snapshot
                    .ups
                    .iter_mut()
                    .find(|up| up.mid == mid)
                    .context("找不到此 UP 主")?;
                if let Some(note) = patch.note {
                    ensure!(note.chars().count() <= 1000, "備註最多 1000 字");
                    up.custom_note = Some(note.trim().into());
                }
                if let Some(tags) = patch.tags {
                    ensure!(
                        tags.len() <= 50
                            && tags
                                .iter()
                                .all(|tag| !tag.trim().is_empty() && tag.chars().count() <= 50),
                        "標籤格式無效"
                    );
                    let mut seen = std::collections::HashSet::new();
                    up.tags = Some(
                        tags.into_iter()
                            .map(|tag| tag.trim().to_owned())
                            .filter(|tag| seen.insert(tag.clone()))
                            .collect(),
                    );
                }
                if let Some(value) = patch.pinned {
                    up.is_pinned = Some(value);
                }
                if let Some(value) = patch.pin_order {
                    up.pin_order = Some(value);
                }
                if let Some(value) = patch.notify_live {
                    up.notify_on_live = Some(value);
                    for room in &mut live.rooms {
                        if live
                            .native
                            .tracking
                            .get(&room.room_id)
                            .is_some_and(|source| source.up_ids.contains(&mid))
                        {
                            room.notify = value;
                        }
                    }
                }
                if let Some(value) = patch.notify_video {
                    up.notify_on_video = Some(value);
                }
                if let Some(value) = patch.notify_dynamic {
                    up.notify_on_dynamic = Some(value);
                }
                up.updated_at = now_ms();
                Ok(())
            })
            .await?;
        self.poller.notify_update();
        Ok(())
    }

    pub async fn remove_up(&self, mid: u64) -> Result<()> {
        self.store
            .update_both(move |snapshot, live| {
                snapshot.ups.retain(|up| up.mid != mid);
                if let Some(groups) = &mut snapshot.settings.follow_groups {
                    for group in groups {
                        group.mids.retain(|uid| *uid != mid);
                    }
                }
                snapshot.state.known_live_uids.retain(|uid| *uid != mid);
                snapshot.state.known_dynamic_ids.remove(&mid.to_string());
                snapshot.state.known_video_bvids.remove(&mid.to_string());
                live.sync_ups(&snapshot.ups);
                Ok(())
            })
            .await?;
        self.feed.invalidate().await;
        self.poller.notify_update();
        Ok(())
    }

    pub async fn save_groups(&self, mut groups: Vec<FollowGroup>) -> Result<()> {
        validate_groups(&mut groups)?;
        self.store
            .update(move |snapshot| {
                snapshot.settings.follow_groups = Some(groups);
                Ok(())
            })
            .await?;
        self.poller.notify_update();
        Ok(())
    }

    pub async fn patch_settings(&self, patch: Value) -> Result<()> {
        ensure!(
            patch.is_object() && patch.get("followGroups").is_none(),
            "一般設定不可覆寫分組"
        );
        self.store
            .update(move |snapshot| {
                let mut value = serde_json::to_value(&snapshot.settings)?;
                value
                    .as_object_mut()
                    .context("設定格式無效")?
                    .extend(patch.as_object().context("設定格式無效")?.clone());
                let settings: AppSettings = serde_json::from_value(value)?;
                validate_app_settings(&settings)?;
                snapshot.settings = settings;
                Ok(())
            })
            .await?;
        self.poller.reconfigure();
        self.poller.notify_update();
        Ok(())
    }

    pub async fn import_backup(&self, value: Value) -> Result<usize> {
        let count = self.store.import_backup(value).await?;
        self.feed.invalidate().await;
        self.poller.reconfigure();
        self.poller.notify_update();
        Ok(count)
    }
}

#[derive(Clone, Default)]
pub struct UpPatch {
    pub note: Option<String>,
    pub tags: Option<Vec<String>>,
    pub pinned: Option<bool>,
    pub pin_order: Option<i64>,
    pub notify_live: Option<bool>,
    pub notify_video: Option<bool>,
    pub notify_dynamic: Option<bool>,
}
