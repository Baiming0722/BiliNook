use crate::{
    groups::{validate_groups, validate_ups},
    live_watch::LiveWatchData,
    models::*,
    now_ms,
};
use anyhow::{Context, Result, anyhow, bail, ensure};
use fs2::FileExt;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{
    any::Any,
    collections::{BTreeMap, HashSet},
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};

const JOURNAL: &str = ".import-rollback.json";
const READ_FILE: &str = "read-dynamics.json";
const LIVE_FILE: &str = "live-watch.json";
pub(crate) const MAX_JSON_BYTES: u64 = 32 * 1024 * 1024;
type Answer = Box<dyn Any + Send>;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoreState {
    #[serde(flatten)]
    follow: Snapshot,
    #[serde(default)]
    live_watch: Arc<LiveWatchData>,
}
type Change = Box<dyn FnOnce(&mut StoreState) -> Result<Answer> + Send>;

enum Request {
    Update(Change, oneshot::Sender<Result<Answer>>),
    Snapshot(oneshot::Sender<Result<Snapshot>>),
    LiveSnapshot(oneshot::Sender<Result<Arc<LiveWatchData>>>),
    Backup(oneshot::Sender<Result<Value>>),
    Flush(oneshot::Sender<Result<()>>),
}

#[derive(Clone)]
pub struct JsonStore {
    dir: Arc<PathBuf>,
    sender: mpsc::Sender<Request>,
}

pub fn data_dir() -> Result<PathBuf> {
    if let Some(value) = std::env::var_os("BILI_DATA_DIR") {
        ensure!(!value.is_empty(), "BILI_DATA_DIR 不可為空");
        return Ok(PathBuf::from(value));
    }
    if cfg!(debug_assertions) {
        Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(".local/data"))
    } else {
        Ok(dirs::home_dir()
            .context("找不到使用者目錄")?
            .join(".bilibili-follow/data"))
    }
}

impl JsonStore {
    pub async fn open(dir: impl Into<PathBuf>) -> Result<Self> {
        let dir = dir.into();
        let (dir, initial, lock) = tokio::task::spawn_blocking(move || -> Result<_> {
            fs::create_dir_all(&dir).context("建立資料目錄失敗")?;
            let dir = fs::canonicalize(dir)?;
            let lock = OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(dir.join(".rust-writer.lock"))?;
            lock.try_lock_exclusive()
                .context("此資料目錄已由另一個 Rust 實例使用")?;
            recover(&dir)?;
            let follow = Snapshot {
                ups: read_json(&dir.join("ups.json"))?,
                settings: read_json(&dir.join("settings.json"))?,
                state: read_json(&dir.join("state.json"))?,
                read_dynamics: read_json(&dir.join(READ_FILE))?,
            };
            validate_ups(&follow.ups)?;
            validate_read_ids(&follow.read_dynamics)?;
            if let Some(mut groups) = follow.settings.follow_groups.clone() {
                validate_groups(&mut groups)?;
            }
            let live_watch: LiveWatchData = read_json(&dir.join(LIVE_FILE))?;
            live_watch.validate()?;
            Ok((
                dir,
                StoreState {
                    follow,
                    live_watch: Arc::new(live_watch),
                },
                lock,
            ))
        })
        .await??;
        let (sender, mut receiver) = mpsc::channel(128);
        let dir = Arc::new(dir);
        let actor_dir = dir.clone();
        tokio::spawn(async move {
            let _writer_lock: File = lock;
            let mut current = initial;
            let mut healthy = true;
            let mut unreported_write_error: Option<String> = None;
            while let Some(request) = receiver.recv().await {
                match request {
                    Request::Snapshot(reply) => {
                        let _ = reply.send(if healthy {
                            Ok(current.follow.clone())
                        } else {
                            Err(recovery_error())
                        });
                    }
                    Request::LiveSnapshot(reply) => {
                        let _ = reply.send(if healthy {
                            Ok(current.live_watch.clone())
                        } else {
                            Err(recovery_error())
                        });
                    }
                    Request::Backup(reply) => {
                        let answer = if healthy {
                            Ok(
                                json!({"version":"3.0.0","timestamp":now_ms(),"ups":current.follow.ups,"settings":current.follow.settings,"readDynamics":current.follow.read_dynamics,"liveWatch":current.live_watch}),
                            )
                        } else {
                            Err(recovery_error())
                        };
                        let _ = reply.send(answer);
                    }
                    Request::Flush(reply) => {
                        let _ = reply.send(if !healthy {
                            Err(recovery_error())
                        } else if let Some(error) = unreported_write_error.take() {
                            Err(anyhow!("先前一筆寫入失敗，該筆變更已回復舊資料：{error}"))
                        } else {
                            Ok(())
                        });
                    }
                    Request::Update(change, reply) => {
                        if !healthy {
                            let _ = reply.send(Err(recovery_error()));
                            continue;
                        }
                        let mut next = current.clone();
                        let answer = match change(&mut next) {
                            Ok(answer) => answer,
                            Err(error) => {
                                let _ = reply.send(Err(error));
                                continue;
                            }
                        };
                        if next == current {
                            let _ = reply.send(Ok(answer));
                            continue;
                        }
                        let dir = actor_dir.clone();
                        let previous = current.clone();
                        let write_next = next.clone();
                        // 寫入由 actor 擁有；呼叫端取消等待，也不會中斷落盤或打亂後續順序。
                        let result = tokio::task::spawn_blocking(move || {
                            commit(&dir, &previous, &write_next)
                        })
                        .await;
                        match result {
                            Ok(Ok(())) => {
                                current = next;
                                let _ = reply.send(Ok(answer));
                            }
                            result => {
                                healthy = !actor_dir.join(JOURNAL).exists();
                                let error = match result {
                                    Ok(Err(error)) => error,
                                    Err(error) => anyhow!(error),
                                    _ => unreachable!(),
                                };
                                // 即使等待者已被退出流程取消，也必須在 flush 時回報失敗一次。
                                unreported_write_error = Some(error.to_string());
                                let _ = reply.send(Err(error));
                            }
                        }
                    }
                }
            }
        });
        Ok(Self { dir, sender })
    }

    pub fn directory(&self) -> &Path {
        &self.dir
    }

    pub async fn snapshot(&self) -> Result<Snapshot> {
        let (tx, rx) = oneshot::channel();
        self.sender
            .send(Request::Snapshot(tx))
            .await
            .map_err(|_| anyhow!("存檔服務已關閉"))?;
        rx.await.context("存檔服務意外中止")?
    }

    pub async fn update<T: Send + 'static>(
        &self,
        change: impl FnOnce(&mut Snapshot) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let (tx, rx) = oneshot::channel();
        let boxed: Change = Box::new(move |state| {
            let old = state.follow.settings.clone();
            let answer = change(&mut state.follow)?;
            if old.theme != state.follow.settings.theme
                || old.appearance != state.follow.settings.appearance
            {
                let live = Arc::make_mut(&mut state.live_watch);
                live.settings.sync_appearance(&state.follow.settings);
                live.native.revision = live.native.revision.wrapping_add(1);
                live.validate()?;
            }
            Ok(Box::new(answer) as Answer)
        });
        self.sender
            .send(Request::Update(boxed, tx))
            .await
            .map_err(|_| anyhow!("存檔佇列已關閉"))?;
        let answer = rx.await.context("存檔服務意外中止")??;
        answer
            .downcast::<T>()
            .map(|answer| *answer)
            .map_err(|_| anyhow!("存檔結果型別不符"))
    }

    pub async fn flush(&self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.sender
            .send(Request::Flush(tx))
            .await
            .map_err(|_| anyhow!("存檔佇列已關閉"))?;
        rx.await.context("存檔服務意外中止")?
    }

    pub async fn live_snapshot(&self) -> Result<Arc<LiveWatchData>> {
        let (tx, rx) = oneshot::channel();
        self.sender
            .send(Request::LiveSnapshot(tx))
            .await
            .map_err(|_| anyhow!("存檔服務已關閉"))?;
        rx.await.context("直播存檔服務意外中止")?
    }

    pub async fn update_live<T: Send + 'static>(
        &self,
        change: impl FnOnce(&mut LiveWatchData) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        self.update_both(move |_, live| change(live)).await
    }

    pub async fn update_both<T: Send + 'static>(
        &self,
        change: impl FnOnce(&mut Snapshot, &mut LiveWatchData) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let (tx, rx) = oneshot::channel();
        let boxed: Change = Box::new(move |state| {
            let previous = state.live_watch.clone();
            let live = Arc::make_mut(&mut state.live_watch);
            let answer = change(&mut state.follow, live)?;
            for room in &live.rooms {
                if let Some(source) = live.native.tracking.get(&room.room_id) {
                    for up in &mut state.follow.ups {
                        if source.up_ids.contains(&up.mid) {
                            up.notify_on_live = Some(room.notify);
                        }
                    }
                }
            }
            live.settings.sync_appearance(&state.follow.settings);
            if live != previous.as_ref() {
                live.native.revision = previous.native.revision.wrapping_add(1);
            }
            live.validate()?;
            Ok(Box::new(answer) as Answer)
        });
        self.sender
            .send(Request::Update(boxed, tx))
            .await
            .map_err(|_| anyhow!("存檔佇列已關閉"))?;
        rx.await
            .context("直播存檔服務意外中止")??
            .downcast::<T>()
            .map(|answer| *answer)
            .map_err(|_| anyhow!("存檔結果型別不符"))
    }

    pub async fn mark_read(&self, ids: Vec<String>) -> Result<()> {
        validate_read_ids(&ids)?;
        self.update(move |snapshot| {
            let mut seen: HashSet<String> = snapshot.read_dynamics.iter().cloned().collect();
            snapshot
                .read_dynamics
                .extend(ids.into_iter().filter(|id| seen.insert(id.clone())));
            let extra = snapshot.read_dynamics.len().saturating_sub(5000);
            snapshot.read_dynamics.drain(..extra);
            Ok(())
        })
        .await
    }

    pub async fn export_backup(&self) -> Result<Value> {
        let (tx, rx) = oneshot::channel();
        self.sender
            .send(Request::Backup(tx))
            .await
            .map_err(|_| anyhow!("存檔服務已關閉"))?;
        rx.await.context("備份服務意外中止")?
    }

    pub async fn import_backup(&self, value: Value) -> Result<usize> {
        ensure!(
            value.is_object() && (value.get("ups").is_some() || value.get("settings").is_some()),
            "備份必須包含追蹤名單或設定"
        );
        let ups: Option<Vec<UpUser>> = value
            .get("ups")
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()
            .context("備份中的追蹤名單格式無效")?;
        if let Some(ups) = &ups {
            validate_ups(ups)?;
        }
        let settings_patch = value.get("settings").cloned();
        if let Some(settings) = &settings_patch {
            ensure!(settings.is_object(), "備份設定格式無效");
        }
        let read_ids: Option<Vec<String>> = value
            .get("readDynamics")
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()
            .context("備份已讀紀錄格式無效")?;
        if let Some(ids) = &read_ids {
            validate_read_ids(ids)?;
        }
        let live: Option<LiveWatchData> = value
            .get("liveWatch")
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()
            .context("備份直播資料格式無效")?;
        if let Some(live) = &live {
            live.validate()?;
        }
        let restore_up_notifications = live.is_none() && ups.is_some();
        self.update_both(move |snapshot, current_live| {
            if let Some(mut live) = live {
                for observation in live.stats.observations.values_mut() {
                    observation.run_id = "backup-restored".into();
                }
                *current_live = live;
            }
            if let Some(ups) = ups {
                snapshot.ups = ups;
            }
            if let Some(patch) = settings_patch {
                let mut settings = serde_json::to_value(&snapshot.settings)?;
                settings
                    .as_object_mut()
                    .context("設定格式無效")?
                    .extend(patch.as_object().context("設定格式無效")?.clone());
                snapshot.settings =
                    serde_json::from_value(settings).context("備份設定欄位格式無效")?;
                if let Some(groups) = &mut snapshot.settings.follow_groups {
                    validate_groups(groups)?;
                }
                validate_app_settings(&snapshot.settings)?;
            }
            if let Some(ids) = read_ids {
                let mut seen = HashSet::new();
                snapshot.read_dynamics = ids
                    .into_iter()
                    .filter(|id| seen.insert(id.clone()))
                    .collect();
            }
            current_live.sync_ups(&snapshot.ups);
            if restore_up_notifications {
                let restored: BTreeMap<_, _> = snapshot
                    .ups
                    .iter()
                    .filter_map(|up| up.notify_on_live.map(|enabled| (up.mid, enabled)))
                    .collect();
                for room in &mut current_live.rooms {
                    if let Some(enabled) = current_live
                        .native
                        .tracking
                        .get(&room.room_id)
                        .and_then(|source| source.up_ids.iter().find_map(|id| restored.get(id)))
                    {
                        room.notify = *enabled;
                    }
                }
            }
            Ok(snapshot.ups.len())
        })
        .await
    }
}

pub(crate) fn validate_app_settings(settings: &AppSettings) -> Result<()> {
    settings.appearance.validate()?;
    ensure!(
        (30..=86400).contains(&settings.poll_interval_live_sec),
        "直播輪詢間隔須為 30–86400 秒"
    );
    ensure!(
        (60..=86400).contains(&settings.poll_interval_dynamic_sec),
        "動態輪詢間隔須為 60–86400 秒"
    );
    ensure!(
        matches!(settings.theme.as_str(), "dark" | "light" | "system"),
        "主題設定無效"
    );
    crate::api::ApiClient::new(settings.custom_proxy_url.as_deref())?;
    Ok(())
}

pub fn validate_read_ids(ids: &[String]) -> Result<()> {
    ensure!(
        ids.len() <= 5000 && ids.iter().all(|id| !id.is_empty() && id.len() <= 256),
        "已讀紀錄格式無效（最多 5000 筆）"
    );
    Ok(())
}

fn recovery_error() -> anyhow::Error {
    anyhow!("匯入復原尚未完成，請保留資料並重新啟動")
}

fn read_json<T: DeserializeOwned + Default>(path: &Path) -> Result<T> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(T::default()),
        Err(error) => {
            return Err(error).with_context(|| format!("無法讀取 {}，原檔已保留", path.display()));
        }
    };
    ensure!(
        metadata.len() <= MAX_JSON_BYTES,
        "{} 超過 32 MiB 安全上限",
        path.display()
    );
    let content = fs::read_to_string(path)?;
    serde_json::from_str(content.trim_start_matches('\u{feff}'))
        .with_context(|| format!("{} JSON 損壞，原檔已保留", path.display()))
}

pub fn atomic_write(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path.parent().context("存檔路徑無父目錄")?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut temp, value)?;
    temp.write_all(b"\n")?;
    ensure!(
        temp.as_file().metadata()?.len() <= MAX_JSON_BYTES,
        "{} 超過 32 MiB 安全上限，原檔已保留",
        path.display()
    );
    temp.as_file().sync_all()?;
    for attempt in 0..5 {
        match temp.persist(path) {
            Ok(_) => return Ok(()),
            Err(error) => {
                let retryable = matches!(error.error.raw_os_error(), Some(5 | 32 | 33 | 183));
                if !retryable || attempt == 4 {
                    return Err(error.error)
                        .with_context(|| format!("原子寫入 {} 失敗", path.display()));
                }
                temp = error.file;
                std::thread::sleep(Duration::from_millis(50 * (attempt + 1)));
            }
        }
    }
    unreachable!()
}

fn commit(dir: &Path, previous: &StoreState, next: &StoreState) -> Result<()> {
    let mut changed: Vec<(&str, Value)> = Vec::new();
    if previous.follow.ups != next.follow.ups {
        changed.push(("ups.json", serde_json::to_value(&next.follow.ups)?));
    }
    if previous.follow.settings != next.follow.settings {
        changed.push((
            "settings.json",
            serde_json::to_value(&next.follow.settings)?,
        ));
    }
    if previous.follow.state != next.follow.state {
        changed.push(("state.json", serde_json::to_value(&next.follow.state)?));
    }
    if previous.follow.read_dynamics != next.follow.read_dynamics {
        changed.push((READ_FILE, serde_json::to_value(&next.follow.read_dynamics)?));
    }
    if !Arc::ptr_eq(&previous.live_watch, &next.live_watch)
        && previous.live_watch != next.live_watch
    {
        changed.push((LIVE_FILE, serde_json::to_value(&next.live_watch)?));
    }
    if changed.is_empty() {
        return Ok(());
    }
    if changed.len() == 1 {
        return atomic_write(&dir.join(changed[0].0), &changed[0].1);
    }
    // 與舊版同名的 rollback journal；新增 state/readDynamics 可被舊版安全忽略。
    atomic_write(&dir.join(JOURNAL), previous)?;
    let result = (|| -> Result<()> {
        for (name, value) in changed {
            atomic_write(&dir.join(name), &value)?;
        }
        fs::remove_file(dir.join(JOURNAL))?;
        Ok(())
    })();
    if let Err(error) = result {
        if let Err(rollback) = recover(dir) {
            bail!("寫入失敗：{error:#}；復原未完成：{rollback:#}");
        }
        return Err(error);
    }
    Ok(())
}

fn recover(dir: &Path) -> Result<()> {
    let journal = dir.join(JOURNAL);
    if !journal.exists() {
        return Ok(());
    }
    let value: Value = read_json(&journal)?;
    ensure!(
        value.get("ups").is_some() && value.get("settings").is_some(),
        "匯入復原日誌無效，原檔已保留"
    );
    let ups: Vec<UpUser> = serde_json::from_value(value["ups"].clone())?;
    validate_ups(&ups)?;
    let _: AppSettings = serde_json::from_value(value["settings"].clone())?;
    for (key, name) in [
        ("ups", "ups.json"),
        ("settings", "settings.json"),
        ("state", "state.json"),
        ("readDynamics", READ_FILE),
        ("liveWatch", LIVE_FILE),
    ] {
        if let Some(data) = value.get(key) {
            atomic_write(&dir.join(name), data)?;
        }
    }
    fs::remove_file(journal).context("清理復原日誌失敗")?;
    Ok(())
}
