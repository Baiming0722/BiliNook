use super::*;
use anyhow::Context;
use chrono::{Datelike, Local, NaiveDate, TimeZone};

pub fn merge_sessions(a: &[LiveSession], b: &[LiveSession]) -> Vec<LiveSession> {
    let mut result = BTreeMap::<i64, LiveSession>::new();
    for session in a.iter().chain(b) {
        let Some(start) = timestamp(&session.started_at) else {
            continue;
        };
        let mut next = session.clone();
        next.started_at = iso(start.max(0) as u64);
        if let Some(end) = &next.ended_at {
            let Some(end) = timestamp(end).filter(|end| *end >= start) else {
                continue;
            };
            next.ended_at = Some(iso(end.max(0) as u64));
        }
        let replace = result.get(&start).is_none_or(|old| {
            old.ended_at.is_none() && next.ended_at.is_some()
                || next.ended_at.is_some()
                    && (old.incomplete && !next.incomplete
                        || old.incomplete == next.incomplete && next.ended_at > old.ended_at)
        });
        if replace {
            result.insert(start, next);
        }
    }
    let skip = result.len().saturating_sub(50);
    result.into_values().skip(skip).collect()
}

impl LiveWatchData {
    pub fn observe(&mut self, rooms: Vec<ObservedRoom>, run_id: &str) -> Vec<LiveNotice> {
        let mut notices = vec![];
        for info in rooms {
            let id = info.room_id;
            let Some(saved) = self.rooms.iter_mut().find(|r| r.room_id == id) else {
                continue;
            };
            let Some(now) = timestamp(&info.last_fetched_at) else {
                continue;
            };
            let old = self.stats.observations.get(&id).cloned();
            if old
                .as_ref()
                .is_some_and(|o| timestamp(&o.observed_at).is_some_and(|at| at >= now))
            {
                continue;
            }
            if let Some(short) = info.short_id {
                saved.short_id = Some(short);
            }
            if !info.status.reliable() {
                if let Some(old) = self.stats.observations.get_mut(&id) {
                    old.interrupted = true;
                }
                self.native.cached_rooms.insert(id, info);
                continue;
            }
            let live = info.status == RoomStatus::Live;
            let normalized = if live {
                RoomStatus::Live
            } else {
                RoomStatus::Offline
            };
            let prior_time = old.as_ref().and_then(|o| timestamp(&o.observed_at));
            let same_run = old.as_ref().is_some_and(|o| o.run_id == run_id);
            let interrupted = old.as_ref().is_some_and(|o| o.interrupted)
                || !same_run
                || prior_time.is_some_and(|at| now - at >= 600_000);
            let sessions = self.stats.history.entry(id).or_default();
            if interrupted {
                let until = prior_time
                    .or_else(|| self.stats.last_live_at.get(&id).and_then(|s| timestamp(s)))
                    .unwrap_or(now);
                for session in sessions.iter_mut().filter(|s| s.ended_at.is_none()) {
                    let start = timestamp(&session.started_at).unwrap_or(now);
                    session.ended_at = Some(iso(until.min(now).max(start).max(0) as u64));
                    session.incomplete = true;
                }
                if live {
                    sessions.push(LiveSession {
                        started_at: info.last_fetched_at.clone(),
                        ended_at: None,
                        incomplete: true,
                    });
                }
            } else if let Some(old) = &old {
                if old.status != normalized {
                    if live {
                        sessions.push(LiveSession {
                            started_at: info.last_fetched_at.clone(),
                            ..Default::default()
                        });
                        notices.push(LiveNotice {
                            id: format!("{id}-{now}"),
                            room_id: id,
                            name: info.uname.clone(),
                            title: info.title.clone(),
                            at: info.last_fetched_at.clone(),
                            read: false,
                        });
                    } else {
                        for session in sessions.iter_mut().filter(|s| s.ended_at.is_none()) {
                            session.ended_at = Some(info.last_fetched_at.clone());
                        }
                    }
                } else if live {
                    let delta = (now / 1000 - prior_time.unwrap_or(now) / 1000).max(0) as u64;
                    *self.stats.live_duration.entry(id).or_default() += delta;
                }
            }
            *sessions = merge_sessions(sessions, &[]);
            if live {
                self.stats
                    .last_live_at
                    .insert(id, info.last_fetched_at.clone());
            }
            self.stats.observations.insert(
                id,
                Observation {
                    status: normalized,
                    observed_at: info.last_fetched_at.clone(),
                    run_id: run_id.into(),
                    interrupted: false,
                },
            );
            if self
                .stats
                .last_calculated_at
                .as_ref()
                .and_then(|s| timestamp(s))
                .is_none_or(|at| now > at)
            {
                self.stats.last_calculated_at = Some(info.last_fetched_at.clone());
            }
            self.native.cached_rooms.insert(id, info);
        }
        for notice in &notices {
            if !self.native.notices.iter().any(|n| n.id == notice.id) {
                self.native.notices.insert(0, notice.clone());
            }
        }
        self.native.notices.truncate(100);
        notices
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryRow {
    pub room_id: u64,
    pub name: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub observed_until: Option<String>,
    pub seconds: Option<u64>,
    pub state: &'static str,
    pub crosses_start: bool,
}
pub fn local_date_range(from: &str, to: &str) -> Result<(i64, i64)> {
    fn parse(s: &str) -> Result<NaiveDate> {
        ensure!(s.len() == 10, "日期格式須為 YYYY-MM-DD");
        Ok(NaiveDate::parse_from_str(s, "%Y-%m-%d").context("日期無效")?)
    }
    let from = parse(from)?;
    let to = parse(to)?;
    ensure!(from <= to, "開始日期不可晚於結束日期");
    let end = to.succ_opt().context("日期超出範圍")?;
    let start = Local
        .from_local_datetime(&from.and_hms_opt(0, 0, 0).unwrap())
        .earliest()
        .context("當地開始日期無效")?
        .timestamp_millis();
    let end = Local
        .from_local_datetime(&end.and_hms_opt(0, 0, 0).unwrap())
        .earliest()
        .context("當地結束日期無效")?
        .timestamp_millis();
    Ok((start, end))
}

pub fn history_rows(
    data: &LiveWatchData,
    range: (i64, i64),
    room_id: Option<u64>,
    now: u64,
) -> Vec<HistoryRow> {
    let mut rows = vec![];
    for (id, sessions) in &data.stats.history {
        if room_id.is_some_and(|room| room != *id) {
            continue;
        }
        let room = data.rooms.iter().find(|r| r.room_id == *id);
        let info = data.native.cached_rooms.get(id);
        let observed = data
            .stats
            .observations
            .get(id)
            .and_then(|o| timestamp(&o.observed_at))
            .or_else(|| data.stats.last_live_at.get(id).and_then(|s| timestamp(s)));
        for session in sessions {
            let Some(start) = timestamp(&session.started_at).filter(|at| *at <= now as i64) else {
                continue;
            };
            let end = session.ended_at.as_deref().and_then(timestamp);
            let until = end
                .or(observed)
                .map(|at| at.min(now as i64))
                .filter(|at| *at >= start);
            if start >= range.1
                || until.map_or(start < range.0, |end| {
                    if end == start {
                        start < range.0
                    } else {
                        end <= range.0
                    }
                })
            {
                continue;
            }
            let state = if session.ended_at.is_some() {
                if session.incomplete {
                    "觀測中斷"
                } else {
                    "完整"
                }
            } else if room.is_some() && info.is_some_and(|r| r.status == RoomStatus::Live) {
                "進行中"
            } else {
                "結束待確認"
            };
            rows.push(HistoryRow {
                room_id: *id,
                name: if room.is_some() {
                    info.map(|i| i.uname.clone())
                        .unwrap_or_else(|| format!("房間 {id}"))
                } else {
                    format!("房間 {id}（已移出監看）")
                },
                started_at: session.started_at.clone(),
                ended_at: session.ended_at.clone(),
                observed_until: until.map(|at| iso(at.max(0) as u64)),
                seconds: until
                    .map(|at| ((at.min(range.1) - start.max(range.0)).max(0) / 1000) as u64),
                state,
                crosses_start: start < range.0,
            });
        }
    }
    rows.sort_by(|a, b| {
        b.started_at
            .cmp(&a.started_at)
            .then(a.room_id.cmp(&b.room_id))
    });
    rows
}

pub fn history_csv(rows: &[HistoryRow]) -> String {
    fn cell(value: &str) -> String {
        let mut text = value.to_owned();
        if text
            .trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
            .starts_with(['=', '+', '-', '@'])
        {
            text.insert(0, '\'');
        }
        format!("\"{}\"", text.replace('"', "\"\""))
    }
    let mut result = String::from("\u{feff}");
    result.push_str(
        &[
            "房號",
            "主播／目前房間名",
            "紀錄開始（ISO8601）",
            "紀錄結束（ISO8601）",
            "最後可靠觀測（ISO8601）",
            "查詢區間內觀測秒數",
            "紀錄狀態",
            "跨查詢起點",
        ]
        .map(cell)
        .join(","),
    );
    result.push_str("\r\n");
    for row in rows {
        let fields = [
            row.room_id.to_string(),
            row.name.clone(),
            row.started_at.clone(),
            row.ended_at.clone().unwrap_or_default(),
            row.observed_until.clone().unwrap_or_default(),
            row.seconds.map(|n| n.to_string()).unwrap_or_default(),
            row.state.into(),
            if row.crosses_start { "是" } else { "否" }.into(),
        ];
        result.push_str(&fields.iter().map(|s| cell(s)).collect::<Vec<_>>().join(","));
        result.push_str("\r\n");
    }
    result
}

#[derive(Clone, Debug)]
pub struct TrendDay {
    pub date: String,
    pub label: String,
    pub count: usize,
    pub duration_seconds: u64,
}
pub fn seven_day_trend(data: &LiveWatchData, now: u64) -> Vec<TrendDay> {
    let now_date = Local
        .timestamp_millis_opt(now.min(i64::MAX as u64) as i64)
        .single()
        .unwrap_or_else(Local::now)
        .date_naive();
    (0..7)
        .rev()
        .map(|offset| {
            let date = now_date - chrono::Duration::days(offset);
            let key = date.format("%Y-%m-%d").to_string();
            let range = local_date_range(&key, &key).unwrap_or((0, 0));
            let rows = history_rows(data, range, None, now);
            TrendDay {
                date: key,
                label: if offset == 0 {
                    "今日".into()
                } else {
                    format!("{}/{}", date.month(), date.day())
                },
                count: rows
                    .iter()
                    .filter(|r| {
                        timestamp(&r.started_at).is_some_and(|at| at >= range.0 && at < range.1)
                    })
                    .count(),
                duration_seconds: rows.iter().filter_map(|r| r.seconds).sum(),
            }
        })
        .collect()
}
