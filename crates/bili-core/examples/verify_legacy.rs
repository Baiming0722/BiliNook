use anyhow::{Context, Result, ensure};
use bili_core::models::{AppSettings, PollerState, UpUser};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::path::Path;

fn verify<T: DeserializeOwned + Serialize>(path: &Path) -> Result<Value> {
    let before: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let model: T = serde_json::from_value(before.clone())?;
    let after = serde_json::to_value(model)?;
    fn contains(before: &Value, after: &Value) -> bool {
        match before {
            Value::Object(map) => map
                .iter()
                .all(|(key, value)| after.get(key).is_some_and(|after| contains(value, after))),
            Value::Array(values) => after.as_array().is_some_and(|after| {
                values.len() == after.len() && values.iter().zip(after).all(|(a, b)| contains(a, b))
            }),
            _ => before == after,
        }
    }
    ensure!(
        contains(&before, &after),
        "{} 有欄位未能無損往返",
        path.display()
    );
    Ok(after)
}

fn main() -> Result<()> {
    let directory = std::env::args_os().nth(1).context("請指定唯讀樣本目錄")?;
    let directory = Path::new(&directory);
    let users = verify::<Vec<UpUser>>(&directory.join("ups.json"))?;
    verify::<AppSettings>(&directory.join("settings.json"))?;
    verify::<PollerState>(&directory.join("state.json"))?;
    println!(
        "三個舊版 JSON 無損往返通過；{} 位 UP 主。未寫入來源。",
        users.as_array().map_or(0, Vec::len)
    );
    Ok(())
}
