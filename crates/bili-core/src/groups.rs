use crate::models::{FollowGroup, UpUser};
use anyhow::{Result, ensure};
use std::collections::HashSet;

pub const MAX_SAFE_UID: u64 = 9_007_199_254_740_991;

pub fn default_groups() -> Vec<FollowGroup> {
    (1..=2)
        .map(|n| FollowGroup {
            id: format!("group-{n}"),
            name: format!("組別{n}"),
            ..Default::default()
        })
        .collect()
}

pub fn validate_groups(groups: &mut [FollowGroup]) -> Result<()> {
    ensure!(groups.len() <= 50, "最多可建立 50 個分組");
    let (mut ids, mut names) = (HashSet::new(), HashSet::new());
    for group in groups {
        ensure!(
            !group.id.is_empty()
                && group.id.len() <= 80
                && group
                    .id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
                && group.id != "all"
                && group.id != "ungrouped"
                && ids.insert(group.id.clone()),
            "分組識別碼無效或重複"
        );
        group.name = group.name.trim().to_owned();
        ensure!(
            !group.name.is_empty()
                && group.name.encode_utf16().count() <= 24
                && names.insert(group.name.to_lowercase()),
            "分組名稱須為 1–24 字且不可重複"
        );
        ensure!(
            group
                .mids
                .iter()
                .all(|mid| *mid > 0 && *mid <= MAX_SAFE_UID),
            "分組成員 UID 無效"
        );
        let mut seen = HashSet::new();
        group.mids.retain(|mid| seen.insert(*mid));
    }
    Ok(())
}

pub fn group_members<'a>(ups: &'a [UpUser], groups: &[FollowGroup], id: &str) -> Vec<&'a UpUser> {
    let assigned: HashSet<u64> = groups.iter().flat_map(|g| g.mids.iter().copied()).collect();
    ups.iter()
        .filter(|up| match id {
            "all" => true,
            "ungrouped" => !assigned.contains(&up.mid),
            _ => groups
                .iter()
                .any(|g| g.id == id && g.mids.contains(&up.mid)),
        })
        .collect()
}

pub fn sort_ups(ups: &mut [UpUser]) {
    ups.sort_by(|a, b| {
        b.is_pinned
            .unwrap_or(false)
            .cmp(&a.is_pinned.unwrap_or(false))
            .then_with(|| {
                if a.is_pinned == Some(true) && b.is_pinned == Some(true) {
                    a.pin_order
                        .unwrap_or_default()
                        .cmp(&b.pin_order.unwrap_or_default())
                } else {
                    b.updated_at.cmp(&a.updated_at)
                }
            })
            .then_with(|| a.mid.cmp(&b.mid))
    });
}

pub fn validate_ups(ups: &[UpUser]) -> Result<()> {
    let mut seen = HashSet::new();
    ensure!(ups.len() <= 1000, "最多追蹤 1000 位 UP 主，請縮小名單");
    for up in ups {
        ensure!(
            up.mid > 0
                && up.mid <= MAX_SAFE_UID
                && !up.name.trim().is_empty()
                && seen.insert(up.mid),
            "備份包含無效或重複的 UP 主"
        );
    }
    Ok(())
}
