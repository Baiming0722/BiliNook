use super::*;
use crate::models::UpUser;
use anyhow::{Context, bail};

impl LiveWatchData {
    pub fn from_legacy_value(mut value: Value) -> Result<Self> {
        if value.is_array() {
            value = serde_json::json!({"version":1,"rooms":value});
        }
        ensure!(
            value.is_object() && value["rooms"].is_array(),
            "備份須包含直播房間陣列"
        );
        ensure!(
            value.get("version").is_none() || matches!(value["version"].as_u64(), Some(1 | 2)),
            "不支援此直播備份版本"
        );
        let has_native = value.get("native").is_some();
        let mut data: Self = serde_json::from_value(value).context("直播備份欄位格式無效")?;
        data.version = 2;
        dedup_by(&mut data.rooms, |v| v.room_id);
        dedup_by(&mut data.groups, |v| v.id.clone());
        dedup_by(&mut data.watchlists, |v| v.id.clone());
        dedup_by(&mut data.settings.filter_presets, |v| v.id.clone());
        for room in &mut data.rooms {
            room.note = room.note.trim().chars().take(200).collect();
            if !has_native {
                data.native.tracking.insert(
                    room.room_id,
                    TrackingSource {
                        manual: true,
                        ..Default::default()
                    },
                );
            }
        }
        data.native.notices.truncate(100);
        data.validate()?;
        for sessions in data.stats.history.values_mut() {
            *sessions = merge_sessions(sessions, &[]);
        }
        Ok(data)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 2 && self.rooms.len() <= MAX_ROOMS,
            "直播資料版本或房間數量無效"
        );
        let groups: BTreeSet<_> = self.groups.iter().map(|v| v.id.as_str()).collect();
        ensure!(
            groups.len() == self.groups.len() && self.groups.len() <= 200,
            "分組 ID 重複或超過上限"
        );
        for group in &self.groups {
            ensure!(
                valid_key(&group.id)
                    && !group.name.trim().is_empty()
                    && group.name.chars().count() <= 80,
                "直播分組名稱或 ID 無效"
            );
        }
        let mut ids = BTreeSet::new();
        for room in &self.rooms {
            ensure!(
                valid_id(room.room_id) && ids.insert(room.room_id),
                "無效或重複的直播房號"
            );
            ensure!(room.short_id.is_none_or(valid_id), "短房號無效");
            ensure!(
                room.group_id
                    .as_ref()
                    .is_none_or(|id| groups.contains(id.as_str())),
                "直播房間的分組不存在"
            );
            ensure!(room.note.chars().count() <= 200, "備註最多 200 字");
        }
        let mut keys = BTreeSet::new();
        for list in &self.watchlists {
            validate_watchlist(list)?;
            ensure!(keys.insert(&list.id), "觀看清單 ID 重複");
        }
        self.settings.appearance().validate()?;
        ensure!(
            ["auto", "system", "light", "dark"].contains(&self.settings.theme.as_str()),
            "主題無效"
        );
        ensure!(
            [30, 60, 180, 300].contains(&self.settings.refresh_interval),
            "更新間隔無效"
        );
        ensure!(
            ["grid", "list"].contains(&self.settings.room_view.as_str()),
            "房間檢視無效"
        );
        ensure!(
            self.settings.sound_volume.is_finite()
                && (0. ..=1.).contains(&self.settings.sound_volume),
            "音量須介於 0–1"
        );
        for (id, sessions) in &self.stats.history {
            ensure!(valid_id(*id), "觀測歷史含無效房號");
            for session in sessions {
                let from = timestamp(&session.started_at).context("觀測歷史開始時間無效")?;
                if let Some(end) = &session.ended_at {
                    ensure!(
                        timestamp(end).is_some_and(|end| end >= from),
                        "觀測歷史結束時間無效"
                    );
                }
            }
        }
        ensure!(
            clock_minutes(&self.native.preferences.quiet_start).is_some()
                && clock_minutes(&self.native.preferences.quiet_end).is_some(),
            "勿擾時間無效"
        );
        Ok(())
    }

    pub fn export_legacy(&self) -> Value {
        serde_json::json!({"version":2,"exportedAt":iso(crate::now_ms()),"rooms":self.rooms,"groups":self.groups,"watchlists":self.watchlists,"settings":self.settings,"stats":self.stats})
    }

    pub fn import(&mut self, mut payload: Value, replace: bool) -> Result<usize> {
        if !replace && let Some(object) = payload.as_object_mut() {
            let mut groups = object
                .get("groups")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            groups.extend(self.groups.iter().map(|g| serde_json::to_value(g).unwrap()));
            object.insert("groups".into(), Value::Array(groups));
        }
        let mut incoming = Self::from_legacy_value(payload)?;
        let imported_appearance = incoming.settings.clone();
        let imported_run = new_id("import");
        for observation in incoming.stats.observations.values_mut() {
            observation.run_id = imported_run.clone();
        }
        if replace {
            // 直播備份覆蓋仍保留原生排除、通知與移轉狀態；完整整合備份由 store 還原。
            incoming.native = self.native.clone();
            incoming.native.migrated_appearance = Some(imported_appearance);
            incoming.native.tracking = incoming
                .rooms
                .iter()
                .map(|r| {
                    (
                        r.room_id,
                        TrackingSource {
                            manual: true,
                            ..Default::default()
                        },
                    )
                })
                .collect();
            incoming
                .native
                .cached_rooms
                .retain(|id, _| incoming.rooms.iter().any(|r| r.room_id == *id));
            let count = incoming.rooms.len();
            *self = incoming;
            return Ok(count);
        }
        let mut added = 0;
        self.native.migrated_appearance = Some(imported_appearance);
        let mut order = self.rooms.iter().map(|r| r.order).max().unwrap_or(-1);
        for mut room in incoming.rooms {
            if self.rooms.iter().any(|r| r.room_id == room.room_id) {
                continue;
            }
            order += 1;
            room.order = order;
            self.native.tracking.insert(
                room.room_id,
                TrackingSource {
                    manual: true,
                    ..Default::default()
                },
            );
            self.rooms.push(room);
            added += 1;
        }
        let mut order = self.groups.iter().map(|g| g.order).max().unwrap_or(-1);
        for mut group in incoming.groups {
            if !self.groups.iter().any(|g| g.id == group.id) {
                order += 1;
                group.order = order;
                self.groups.push(group);
            }
        }
        for list in incoming.watchlists {
            if !self.watchlists.iter().any(|v| v.id == list.id) {
                self.watchlists.push(list);
            }
        }
        for (id, duration) in incoming.stats.live_duration {
            let current = self.stats.live_duration.entry(id).or_default();
            *current = (*current).max(duration);
        }
        for (id, at) in incoming.stats.last_live_at {
            let current = self.stats.last_live_at.entry(id).or_default();
            if timestamp(&at) > timestamp(current) {
                *current = at;
            }
        }
        for (id, sessions) in incoming.stats.history {
            let current = self.stats.history.entry(id).or_default();
            *current = merge_sessions(current, &sessions);
        }
        self.validate()?;
        Ok(added)
    }

    pub fn add_room(&mut self, info: ObservedRoom) -> Result<bool> {
        ensure!(valid_id(info.room_id), "房號無效");
        let id = info.room_id;
        if self.rooms.iter().any(|r| r.room_id == id) {
            self.native.tracking.entry(id).or_default().manual = true;
            self.native.cached_rooms.insert(id, info);
            return Ok(false);
        }
        ensure!(self.rooms.len() < MAX_ROOMS, "監看房間已達上限");
        self.rooms.push(StoredRoom {
            room_id: id,
            short_id: info.short_id,
            added_at: iso(crate::now_ms()),
            order: self.rooms.iter().map(|r| r.order).max().unwrap_or(-1) + 1,
            ..Default::default()
        });
        self.native.tracking.insert(
            id,
            TrackingSource {
                manual: true,
                ..Default::default()
            },
        );
        self.native.cached_rooms.insert(id, info);
        Ok(true)
    }

    pub fn sync_ups(&mut self, ups: &[UpUser]) {
        for source in self.native.tracking.values_mut() {
            source.up_ids.clear();
        }
        for up in ups {
            if self.native.excluded_up_ids.contains(&up.mid) {
                continue;
            }
            let Some(room) = up.live_room.as_ref().filter(|r| valid_id(r.roomid)) else {
                continue;
            };
            let id = room.roomid;
            if !self.rooms.iter().any(|r| r.room_id == id) && self.rooms.len() < MAX_ROOMS {
                self.rooms.push(StoredRoom {
                    room_id: id,
                    added_at: iso(crate::now_ms()),
                    order: self.rooms.iter().map(|r| r.order).max().unwrap_or(-1) + 1,
                    note: up
                        .custom_note
                        .as_deref()
                        .unwrap_or_default()
                        .chars()
                        .take(200)
                        .collect(),
                    notify: up.notify_on_live != Some(false),
                    ..Default::default()
                });
            }
            self.native
                .tracking
                .entry(id)
                .or_default()
                .up_ids
                .insert(up.mid);
        }
        self.rooms.retain(|r| {
            self.native
                .tracking
                .get(&r.room_id)
                .is_none_or(|s| s.manual || !s.up_ids.is_empty())
        });
    }

    pub fn patch_room(&mut self, id: u64, patch: RoomPatch) -> Result<()> {
        if let Some(Some(group)) = &patch.group_id {
            ensure!(self.groups.iter().any(|v| &v.id == group), "分組不存在");
        }
        if let Some(note) = &patch.note {
            ensure!(note.chars().count() <= 200, "備註最多 200 字");
        }
        let room = self
            .rooms
            .iter_mut()
            .find(|r| r.room_id == id)
            .context("直播間已移出監看")?;
        if let Some(group) = patch.group_id {
            room.group_id = group;
        }
        if let Some(note) = patch.note {
            room.note = note.trim().to_owned();
        }
        if let Some(value) = patch.pinned {
            room.pinned = value;
        }
        if let Some(value) = patch.notify {
            room.notify = value;
        }
        Ok(())
    }

    pub fn canonicalize_room(&mut self, from: u64, to: u64) {
        if from == to || !valid_id(to) || !self.rooms.iter().any(|r| r.room_id == from) {
            return;
        }
        if self.rooms.iter().any(|r| r.room_id == to) {
            self.rooms.retain(|r| r.room_id != from);
        } else if let Some(room) = self.rooms.iter_mut().find(|r| r.room_id == from) {
            room.short_id = Some(from);
            room.room_id = to;
        }
        if let Some(source) = self.native.tracking.remove(&from) {
            let target = self.native.tracking.entry(to).or_default();
            target.manual |= source.manual;
            target.up_ids.extend(source.up_ids);
        }
        for list in &mut self.watchlists {
            let mut seen = BTreeSet::new();
            for id in &mut list.room_ids {
                if *id == from {
                    *id = to;
                }
            }
            list.room_ids.retain(|id| seen.insert(*id));
        }
        if let Some(value) = self.stats.live_duration.remove(&from) {
            let entry = self.stats.live_duration.entry(to).or_default();
            *entry = (*entry).max(value);
        }
        if let Some(value) = self.stats.last_live_at.remove(&from) {
            let entry = self.stats.last_live_at.entry(to).or_default();
            if timestamp(&value) > timestamp(entry) {
                *entry = value;
            }
        }
        if let Some(sessions) = self.stats.history.remove(&from) {
            let current = self.stats.history.entry(to).or_default();
            *current = merge_sessions(current, &sessions);
        }
        if let Some(observation) = self.stats.observations.remove(&from) {
            if self
                .stats
                .observations
                .get(&to)
                .is_none_or(|old| timestamp(&old.observed_at) < timestamp(&observation.observed_at))
            {
                self.stats.observations.insert(to, observation);
            }
        }
        if let Some(mut cached) = self.native.cached_rooms.remove(&from) {
            cached.room_id = to;
            self.native.cached_rooms.entry(to).or_insert(cached);
        }
        for notice in &mut self.native.notices {
            if notice.room_id == from {
                notice.room_id = to;
            }
        }
    }

    pub fn remove_rooms(&mut self, ids: &[u64]) -> Vec<(StoredRoom, TrackingSource)> {
        let mut removed = vec![];
        self.rooms.retain(|room| {
            if !ids.contains(&room.room_id) {
                return true;
            }
            let source = self
                .native
                .tracking
                .remove(&room.room_id)
                .unwrap_or(TrackingSource {
                    manual: true,
                    ..Default::default()
                });
            self.native
                .excluded_up_ids
                .extend(source.up_ids.iter().copied());
            self.stats.observations.remove(&room.room_id);
            removed.push((room.clone(), source));
            false
        });
        removed
    }
    pub fn restore_rooms(&mut self, removed: Vec<(StoredRoom, TrackingSource)>) -> Result<()> {
        for (mut room, source) in removed {
            if self.rooms.iter().any(|r| r.room_id == room.room_id) {
                continue;
            }
            if room
                .group_id
                .as_ref()
                .is_some_and(|id| !self.groups.iter().any(|g| &g.id == id))
            {
                room.group_id = None;
            }
            for id in &source.up_ids {
                self.native.excluded_up_ids.remove(id);
            }
            self.native.tracking.insert(room.room_id, source);
            self.rooms.push(room);
        }
        self.rooms.sort_by_key(|r| r.order);
        self.validate()
    }
    pub fn save_group(&mut self, id: Option<String>, name: String) -> Result<String> {
        let name = name.trim();
        ensure!(
            !name.is_empty() && name.chars().count() <= 80,
            "分組名稱須為 1–80 字"
        );
        if let Some(id) = id {
            self.groups
                .iter_mut()
                .find(|g| g.id == id)
                .context("分組已不存在")?
                .name = name.to_owned();
            Ok(id)
        } else {
            ensure!(self.groups.len() < 200, "分組數量已達上限");
            let id = new_id("group");
            self.groups.push(RoomGroup {
                id: id.clone(),
                name: name.to_owned(),
                order: self.groups.len() as i64,
                created_at: iso(crate::now_ms()),
            });
            Ok(id)
        }
    }
    pub fn remove_group(&mut self, id: &str) -> Result<()> {
        ensure!(self.groups.iter().any(|g| g.id == id), "分組不存在");
        self.groups.retain(|g| g.id != id);
        for room in &mut self.rooms {
            if room.group_id.as_deref() == Some(id) {
                room.group_id = None;
            }
        }
        Ok(())
    }
    pub fn reorder_groups(&mut self, ids: Vec<String>) -> Result<()> {
        ensure!(
            ids.len() == self.groups.len()
                && ids.iter().collect::<BTreeSet<_>>().len() == ids.len()
                && self.groups.iter().all(|g| ids.contains(&g.id)),
            "分組名單已變更，請重新載入"
        );
        for group in &mut self.groups {
            group.order = ids.iter().position(|id| id == &group.id).unwrap() as i64;
        }
        self.groups.sort_by_key(|g| g.order);
        Ok(())
    }
    pub fn reorder_rooms(
        &mut self,
        ids: Vec<u64>,
        group: Option<String>,
        pinned: bool,
    ) -> Result<()> {
        let current: Vec<_> = self
            .rooms
            .iter()
            .filter(|r| r.group_id == group && r.pinned == pinned)
            .map(|r| r.room_id)
            .collect();
        ensure!(
            ids.len() == current.len()
                && ids.iter().collect::<BTreeSet<_>>().len() == ids.len()
                && current.iter().all(|id| ids.contains(id)),
            "房間名單已變更，請重新載入排序"
        );
        for room in &mut self.rooms {
            if let Some(index) = ids.iter().position(|id| *id == room.room_id) {
                room.order = index as i64;
            }
        }
        Ok(())
    }
    pub fn save_watchlist(&mut self, mut list: Watchlist) -> Result<String> {
        list.name = list.name.trim().to_owned();
        list.description = list.description.trim().to_owned();
        if list.id.is_empty() {
            list.id = new_id("watch");
            list.created_at = iso(crate::now_ms());
        }
        list.updated_at = iso(crate::now_ms());
        validate_watchlist(&list)?;
        let id = list.id.clone();
        if let Some(existing) = self.watchlists.iter_mut().find(|v| v.id == id) {
            list.created_at = existing.created_at.clone();
            *existing = list;
        } else {
            ensure!(self.watchlists.len() < 200, "觀看清單已達上限");
            self.watchlists.push(list);
        }
        Ok(id)
    }
}

fn dedup_by<T, K: Ord>(items: &mut Vec<T>, key: impl Fn(&T) -> K) {
    let mut seen = BTreeSet::new();
    items.retain(|item| seen.insert(key(item)));
}
pub fn validate_watchlist(list: &Watchlist) -> Result<()> {
    ensure!(
        valid_key(&list.id)
            && !list.name.trim().is_empty()
            && list.name.chars().count() <= 80
            && list.description.chars().count() <= 500,
        "觀看清單名稱或說明無效"
    );
    ensure!(
        list.room_ids.len() <= 50
            && list.room_ids.iter().all(|id| valid_id(*id))
            && list.room_ids.iter().collect::<BTreeSet<_>>().len() == list.room_ids.len(),
        "觀看清單最多 50 間，房號不可重複"
    );
    ensure!([1, 2, 4, 9].contains(&list.playback_limit), "播放上限無效");
    Ok(())
}

#[derive(Clone, Debug, Default)]
pub struct RoomFilter {
    pub query: String,
    pub status: Option<RoomStatus>,
    pub group: Option<String>,
    pub ungrouped: bool,
    pub only_ids: Option<BTreeSet<u64>>,
    pub advanced: AdvancedFilters,
}
pub fn filter_rooms(rooms: &[DisplayRoom], filter: &RoomFilter) -> Vec<DisplayRoom> {
    let words: Vec<_> = filter
        .query
        .to_lowercase()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let mut result: Vec<_> = rooms
        .iter()
        .filter(|r| {
            let text = format!(
                "{} {} {} {} {} {}",
                r.saved.room_id,
                r.saved.short_id.unwrap_or(0),
                r.info.uname,
                r.info.title,
                r.saved.note,
                r.info.uid
            )
            .to_lowercase();
            words.iter().all(|s| text.contains(s))
                && filter.status.is_none_or(|s| r.info.status == s)
                && filter
                    .group
                    .as_ref()
                    .is_none_or(|g| r.saved.group_id.as_ref() == Some(g))
                && (!filter.ungrouped || r.saved.group_id.is_none())
                && filter
                    .only_ids
                    .as_ref()
                    .is_none_or(|ids| ids.contains(&r.saved.room_id))
                && (filter.advanced.area == "all" || r.info.area_name == filter.advanced.area)
                && (filter.advanced.parent_area == "all"
                    || r.info.parent_area_name == filter.advanced.parent_area)
                && r.info.online >= filter.advanced.online_min
                && r.info.attention >= filter.advanced.attention_min
        })
        .cloned()
        .collect();
    result.sort_by(|a, b| {
        let order = match filter.advanced.sort_by.as_str() {
            "online" => a.info.online.cmp(&b.info.online),
            "attention" => a.info.attention.cmp(&b.info.attention),
            "addedAt" => a.saved.added_at.cmp(&b.saved.added_at),
            "liveFirst" => {
                (a.info.status != RoomStatus::Live).cmp(&(b.info.status != RoomStatus::Live))
            }
            _ => a.saved.order.cmp(&b.saved.order),
        };
        b.saved
            .pinned
            .cmp(&a.saved.pinned)
            .then(if filter.advanced.sort_dir == "desc" {
                order.reverse()
            } else {
                order
            })
            .then(a.saved.room_id.cmp(&b.saved.room_id))
    });
    result
}

pub fn clock_minutes(value: &str) -> Option<u32> {
    let (h, m) = value.split_once(':')?;
    if h.len() != 2 || m.len() != 2 {
        return None;
    }
    let h: u32 = h.parse().ok()?;
    let m: u32 = m.parse().ok()?;
    (h < 24 && m < 60).then_some(h * 60 + m)
}
pub fn reminders_paused(
    preferences: &NotificationPreferences,
    now_ms: u64,
    local_minutes: u32,
) -> bool {
    if preferences.snooze_until > now_ms {
        return true;
    }
    if !preferences.quiet_enabled {
        return false;
    }
    match (
        clock_minutes(&preferences.quiet_start),
        clock_minutes(&preferences.quiet_end),
    ) {
        (Some(start), Some(end)) if start < end => local_minutes >= start && local_minutes < end,
        (Some(start), Some(end)) if start > end => local_minutes >= start || local_minutes < end,
        _ => false,
    }
}

pub fn migration_source() -> Result<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("BILILIVE_DATA_FILE") {
        ensure!(!path.is_empty(), "BILILIVE_DATA_FILE 不可為空");
        return Ok(path.into());
    }
    Ok(dirs::home_dir()
        .context("找不到使用者目錄")?
        .join(".bililive-watch/rooms.json"))
}

pub fn parse_backup_bytes(bytes: &[u8]) -> Result<Value> {
    ensure!(bytes.len() <= MAX_BACKUP_BYTES, "直播備份超過 32 MiB 上限");
    let text = std::str::from_utf8(bytes).context("直播備份須為 UTF-8")?;
    let value =
        serde_json::from_str(text.trim_start_matches('\u{feff}')).context("直播備份 JSON 損壞")?;
    Ok(value)
}

pub fn read_migration(path: &std::path::Path) -> Result<LiveWatchData> {
    let size = std::fs::metadata(path)?.len();
    ensure!(size <= 32 * 1024 * 1024, "舊直播資料超過 32 MiB 上限");
    let text = std::fs::read_to_string(path)?;
    let mut data = LiveWatchData::from_legacy_value(serde_json::from_str(
        text.trim_start_matches('\u{feff}'),
    )?)?;
    data.native.migrated_appearance = Some(data.settings.clone());
    data.native.migration = "copied".into();
    data.native.migration_error = None;
    // 來源執行識別不可延續到原生程式，讓下一輪明確標示觀測中斷。
    for item in data.stats.observations.values_mut() {
        item.run_id = "legacy-import".into();
    }
    Ok(data)
}

pub fn validate_import_mode(mode: &str) -> Result<bool> {
    match mode {
        "merge" => Ok(false),
        "replace" => Ok(true),
        _ => bail!("匯入模式無效"),
    }
}
