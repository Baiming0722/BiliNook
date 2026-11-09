use super::*;
use crate::{
    api::BiliApi,
    models::LiveRoom,
    now_ms,
    poller::{Notice, NoticeKind},
    store::JsonStore,
};
use anyhow::Context;
use chrono::Timelike;
use futures::{StreamExt, stream};
use lru::LruCache;
use std::{
    num::NonZeroUsize,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

type RoomCache = LruCache<u64, Arc<Mutex<Option<(Instant, ObservedRoom)>>>>;
pub struct LiveWatchService {
    pub store: JsonStore,
    api: Arc<dyn BiliApi>,
    cache: Mutex<RoomCache>,
    refresh_guard: Mutex<()>,
    run_id: String,
}
impl LiveWatchService {
    pub fn new(api: Arc<dyn BiliApi>, store: JsonStore) -> Self {
        Self {
            store,
            api,
            cache: Mutex::new(LruCache::new(NonZeroUsize::new(200).unwrap())),
            refresh_guard: Mutex::new(()),
            run_id: new_id("run"),
        }
    }
    pub async fn snapshot(&self) -> Result<LiveWatchSnapshot> {
        Ok(LiveWatchSnapshot::new(self.store.live_snapshot().await?))
    }
    pub async fn sync_links(&self) -> Result<()> {
        self.store
            .update_both(|follow, live| {
                live.sync_ups(&follow.ups);
                Ok(())
            })
            .await
    }

    pub async fn migrate_once(&self, source: Option<PathBuf>, retry: bool) -> Result<()> {
        let current = self.store.live_snapshot().await?;
        if !retry && !current.native.migration.is_empty() {
            return Ok(());
        }
        let source = match source {
            Some(path) => path,
            None => migration_source()?,
        };
        let result = tokio::task::spawn_blocking(move || {
            if !source.exists() {
                return Ok(None);
            }
            read_migration(&source).map(Some)
        })
        .await?;
        self.store
            .update_live(move |live| {
                match result {
                    Ok(Some(incoming)) => {
                        let appearance = incoming.native.migrated_appearance.clone();
                        if live.rooms.is_empty() {
                            *live = incoming;
                        } else {
                            live.import(incoming.export_legacy(), false)?;
                        }
                        live.native.migration = "copied".into();
                        live.native.migrated_appearance = appearance;
                        live.native.migration_error = None;
                    }
                    Ok(None) => {
                        live.native.migration = "absent".into();
                        live.native.migration_error = None;
                    }
                    Err(error) => {
                        live.native.migration = "pending".into();
                        live.native.migration_error =
                            Some(format!("舊直播資料未能帶入：{error:#}"));
                    }
                }
                Ok(())
            })
            .await
    }

    pub async fn fetch_room(&self, id: u64, fresh: bool) -> Result<ObservedRoom> {
        ensure!(valid_id(id), "直播房號無效");
        let requested = Instant::now();
        let entry = {
            let mut cache = self.cache.lock().await;
            cache
                .get_or_insert(id, || Arc::new(Mutex::new(None)))
                .clone()
        };
        let mut slot = entry.lock().await;
        if let Some((at, room)) = &*slot {
            if (!fresh && at.elapsed() < Duration::from_secs(30)) || *at >= requested {
                return Ok(room.clone());
            }
        }
        let mut room = tokio::time::timeout(Duration::from_secs(18), self.api.room(id))
            .await
            .context("直播房間查詢逾時")??;
        room.last_fetched_at = iso(now_ms());
        *slot = Some((Instant::now(), room.clone()));
        Ok(room)
    }
    pub async fn add_input(&self, input: String) -> Result<(u64, bool)> {
        let id = parse_room_input(&input)?;
        let info = self.fetch_room(id, false).await?;
        let canonical = info.room_id;
        let added = self
            .store
            .update_live(move |live| live.add_room(info))
            .await?;
        Ok((canonical, added))
    }

    pub async fn refresh(&self, fresh: bool) -> Result<Vec<Notice>> {
        self.refresh_subset(fresh, None).await
    }
    pub async fn refresh_room(&self, id: u64) -> Result<Vec<Notice>> {
        ensure!(valid_id(id), "直播房號無效");
        self.refresh_subset(true, Some(id)).await
    }
    async fn refresh_subset(&self, fresh: bool, only: Option<u64>) -> Result<Vec<Notice>> {
        let _guard = if fresh {
            self.refresh_guard.lock().await
        } else {
            let Ok(guard) = self.refresh_guard.try_lock() else {
                return Ok(vec![]);
            };
            guard
        };
        let follow = self.store.snapshot().await?;
        let native = self.store.live_snapshot().await?;
        let unresolved: Vec<_> = follow
            .ups
            .iter()
            .filter(|up| {
                up.live_room
                    .as_ref()
                    .is_none_or(|room| !valid_id(room.roomid))
                    || native.native.excluded_up_ids.contains(&up.mid)
            })
            .map(|up| up.mid)
            .collect();
        let mut discovery_error = None;
        let discovered = if unresolved.is_empty() {
            BTreeMap::new()
        } else {
            match self.api.live(unresolved).await {
                Ok(v) => v,
                Err(error) => {
                    discovery_error = Some(error.to_string());
                    BTreeMap::new()
                }
            }
        };
        self.store
            .update_both(move |follow, live| {
                for up in &mut follow.ups {
                    if let Some(info) = discovered.get(&up.mid) {
                        let room = up.live_room.get_or_insert_with(LiveRoom::default);
                        room.roomid = info.room_id;
                        room.live_status = info.live_status;
                        room.title = info.title.clone();
                        room.cover = if !info.cover_from_user.is_empty() {
                            info.cover_from_user.clone()
                        } else {
                            info.keyframe.clone()
                        };
                        room.online = info.online;
                    }
                }
                live.sync_ups(&follow.ups);
                Ok(())
            })
            .await?;
        let current = self.store.live_snapshot().await?;
        let ids: Vec<u64> = current
            .rooms
            .iter()
            .filter(|room| only.is_none_or(|id| room.room_id == id))
            .map(|room| room.room_id)
            .collect();
        let results = stream::iter(ids)
            .map(|id| async move { (id, self.fetch_room(id, fresh).await) })
            .buffer_unordered(3)
            .collect::<Vec<_>>()
            .await;
        let at = iso(now_ms());
        let aliases: BTreeMap<u64, u64> = results
            .iter()
            .filter_map(|(id, result)| {
                result
                    .as_ref()
                    .ok()
                    .filter(|room| room.status.reliable() && room.room_id != *id)
                    .map(|room| (*id, room.room_id))
            })
            .collect();
        let infos: Vec<_> = results
            .into_iter()
            .map(|(id, result)| match result {
                Ok(mut info) => {
                    info.room_id = id;
                    info
                }
                Err(error) => {
                    let mut info =
                        current
                            .native
                            .cached_rooms
                            .get(&id)
                            .cloned()
                            .unwrap_or(ObservedRoom {
                                room_id: id,
                                uname: format!("房間 {id}"),
                                ..Default::default()
                            });
                    info.status = RoomStatus::Error;
                    info.error = Some(error.to_string());
                    info.last_fetched_at = at.clone();
                    info
                }
            })
            .collect();
        let run = self.run_id.clone();
        let notices = self
            .store
            .update_both(move |follow, live| {
                let accepted: BTreeSet<u64> = live.rooms.iter().map(|r| r.room_id).collect();
                for (from, to) in &aliases {
                    live.canonicalize_room(*from, *to);
                }
                let mut latest = BTreeMap::<u64, ObservedRoom>::new();
                for mut info in infos {
                    if !accepted.contains(&info.room_id) {
                        continue;
                    }
                    info.room_id = aliases.get(&info.room_id).copied().unwrap_or(info.room_id);
                    if latest.get(&info.room_id).is_none_or(|old| {
                        timestamp(&old.last_fetched_at) <= timestamp(&info.last_fetched_at)
                    }) {
                        latest.insert(info.room_id, info);
                    }
                }
                let notices = live.observe(latest.into_values().collect(), &run);
                for up in &mut follow.ups {
                    let info = live
                        .native
                        .tracking
                        .iter()
                        .find(|(_, s)| s.up_ids.contains(&up.mid))
                        .and_then(|(id, _)| live.native.cached_rooms.get(id));
                    if let Some(info) = info.filter(|i| i.status.reliable()) {
                        let room = up.live_room.get_or_insert_with(LiveRoom::default);
                        room.roomid = info.room_id;
                        room.live_status = match info.status {
                            RoomStatus::Live => 1,
                            RoomStatus::Round => 2,
                            _ => 0,
                        };
                        room.title = info.title.clone();
                        room.cover = info.cover.clone();
                        room.online = info.online;
                        room.url = format!("https://live.bilibili.com/{}", info.room_id);
                        room.room_status = 1;
                    }
                }
                follow.state.last_live_check_time = now_ms();
                follow.state.known_live_uids = follow
                    .ups
                    .iter()
                    .filter(|up| up.live_room.as_ref().is_some_and(|r| r.live_status == 1))
                    .map(|up| up.mid)
                    .collect();
                let local = chrono::Local::now();
                let paused = reminders_paused(
                    &live.native.preferences,
                    now_ms(),
                    local.hour() * 60 + local.minute(),
                );
                let enabled = follow.settings.enable_notifications
                    && follow.settings.notify_live
                    && (live.settings.notify_on_live || live.settings.sound_enabled)
                    && !paused;
                Ok(if enabled {
                    notices
                        .into_iter()
                        .filter(|n| {
                            live.rooms
                                .iter()
                                .any(|r| r.room_id == n.room_id && r.notify)
                        })
                        .map(|n| Notice {
                            kind: NoticeKind::Live,
                            mid: n.room_id,
                            title: format!("{} 開播了", n.name),
                            body: n.title,
                            url: format!("bili-live:{}", n.room_id),
                        })
                        .collect()
                } else {
                    vec![]
                })
            })
            .await?;
        if let Some(error) = discovery_error {
            eprintln!("[直播房號探索] {error}");
        }
        Ok(notices)
    }
}
