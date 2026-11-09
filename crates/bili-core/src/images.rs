use crate::api::{REFERER, USER_AGENT, bounded_body, normalize_url};
use anyhow::{Context, Result, ensure};
use lru::LruCache;
use reqwest::Client;
use std::{
    collections::HashMap,
    fs,
    io::{Cursor, Write},
    num::NonZeroUsize,
    path::PathBuf,
    sync::{Arc, Weak},
    time::SystemTime,
};
use tokio::sync::{Mutex, Semaphore};
use url::Url;

const MAX_DOWNLOAD: usize = 16 * 1024 * 1024;
const MEMORY_BYTES: usize = 32 * 1024 * 1024;
const DISK_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug)]
pub enum ImageKind {
    Png,
    Jpeg,
    Webp,
    Gif,
}

pub struct CachedImage {
    pub bytes: Arc<[u8]>,
    pub kind: ImageKind,
    pub width: u32,
    pub height: u32,
}

struct Memory {
    items: LruCache<String, Arc<CachedImage>>,
    bytes: usize,
}

pub struct ImageCache {
    directory: Arc<PathBuf>,
    http: Client,
    gate: Arc<Semaphore>,
    memory: Mutex<Memory>,
    flights: Mutex<HashMap<String, Weak<Mutex<()>>>>,
    cleanup: Mutex<()>,
}

pub fn image_url(input: &str) -> Result<String> {
    let mut url = Url::parse(&normalize_url(input)).context("圖片網址格式無效")?;
    let host = url.host_str().unwrap_or_default();
    let allowed = ["hdslb.com", "biliimg.com"].iter().any(|domain| {
        host == *domain
            || host
                .strip_suffix(domain)
                .is_some_and(|prefix| prefix.ends_with('.'))
    });
    ensure!(
        url.scheme() == "https"
            && allowed
            && url.username().is_empty()
            && url.password().is_none()
            && url.port_or_known_default() == Some(443),
        "圖片只允許 B 站 HTTPS 圖床"
    );
    url.set_fragment(None);
    Ok(url.into())
}

pub fn original_image_url(input: &str) -> Result<String> {
    let input = image_url(input)?;
    let mut url = Url::parse(&input)?;
    if let Some((path, _)) = url.path().rsplit_once('@') {
        let path = path.to_owned();
        url.set_path(&path);
    }
    Ok(url.into())
}

impl ImageCache {
    pub async fn new(directory: PathBuf, http: Client, gate: Arc<Semaphore>) -> Result<Self> {
        let path = directory.clone();
        tokio::task::spawn_blocking(move || {
            fs::create_dir_all(&path)?;
            prune(&path)
        })
        .await??;
        Ok(Self {
            directory: Arc::new(directory),
            http,
            gate,
            memory: Mutex::new(Memory {
                items: LruCache::new(NonZeroUsize::new(128).unwrap()),
                bytes: 0,
            }),
            flights: Mutex::new(HashMap::new()),
            cleanup: Mutex::new(()),
        })
    }

    pub async fn get(&self, input: &str) -> Result<Arc<CachedImage>> {
        let url = image_url(input)?;
        if let Some(hit) = self.memory.lock().await.items.get(&url).cloned() {
            return Ok(hit);
        }
        let flight = {
            let mut flights = self.flights.lock().await;
            if flights.len() > 512 {
                flights.retain(|_, flight| flight.strong_count() > 0);
            }
            if let Some(flight) = flights.get(&url).and_then(Weak::upgrade) {
                flight
            } else {
                let flight = Arc::new(Mutex::new(()));
                flights.insert(url.clone(), Arc::downgrade(&flight));
                flight
            }
        };
        let _flight = flight.lock().await;
        if let Some(hit) = self.memory.lock().await.items.get(&url).cloned() {
            return Ok(hit);
        }
        let key = format!("{:x}.bin", md5::compute(url.as_bytes()));
        let path = self.directory.join(key);
        let read_path = path.clone();
        let disk = tokio::task::spawn_blocking(move || -> Result<Option<CachedImage>> {
            if !read_path.exists() {
                return Ok(None);
            }
            ensure!(
                fs::metadata(&read_path)?.len() <= MAX_DOWNLOAD as u64,
                "圖片快取過大"
            );
            decode(fs::read(&read_path)?).map(Some)
        })
        .await?;
        let image = match disk {
            Ok(Some(image)) => image,
            _ => {
                let _permit = self.gate.acquire().await?;
                let mut last_error = None;
                let mut bytes = None;
                for attempt in 0..=1 {
                    let result: Result<Vec<u8>> = async {
                        let response = self
                            .http
                            .get(&url)
                            .header(reqwest::header::REFERER, REFERER)
                            .header(reqwest::header::USER_AGENT, USER_AGENT)
                            .send()
                            .await
                            .context("圖片下載失敗或逾時")?;
                        ensure!(
                            response.status().is_success(),
                            "圖片 HTTP {}",
                            response.status().as_u16()
                        );
                        bounded_body(response, MAX_DOWNLOAD).await
                    }
                    .await;
                    match result {
                        Ok(data) => {
                            bytes = Some(data);
                            break;
                        }
                        Err(error) => {
                            last_error = Some(error);
                            if attempt == 0 {
                                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                            }
                        }
                    }
                }
                let bytes = bytes
                    .ok_or_else(|| last_error.unwrap_or_else(|| anyhow::anyhow!("圖片下載失敗")))?;
                let directory = self.directory.clone();
                tokio::task::spawn_blocking(move || -> Result<CachedImage> {
                    let image = decode(bytes)?;
                    let mut temp = tempfile::NamedTempFile::new_in(directory.as_path())?;
                    temp.write_all(&image.bytes)?;
                    temp.as_file().sync_all()?;
                    temp.persist(path)
                        .map_err(|error| error.error)
                        .context("圖片快取寫入失敗")?;
                    Ok(image)
                })
                .await??
            }
        };
        let image = Arc::new(image);
        let mut memory = self.memory.lock().await;
        if let Some(previous) = memory.items.pop(&url) {
            memory.bytes -= previous.bytes.len();
        }
        while memory.items.len() >= 128 || memory.bytes + image.bytes.len() > MEMORY_BYTES {
            if let Some((_, old)) = memory.items.pop_lru() {
                memory.bytes -= old.bytes.len();
            } else {
                break;
            }
        }
        memory.bytes += image.bytes.len();
        memory.items.put(url, image.clone());
        drop(memory);
        // 清理只碰此快取目錄內、由本管線產生的 32 位 hash.bin 檔。
        if let Ok(_cleanup) = self.cleanup.try_lock() {
            let directory = self.directory.clone();
            tokio::task::spawn_blocking(move || prune(&directory)).await??;
        }
        Ok(image)
    }
}

fn decode(bytes: Vec<u8>) -> Result<CachedImage> {
    ensure!(bytes.len() <= MAX_DOWNLOAD, "圖片超過 16 MiB 上限");
    let reader = image::ImageReader::new(Cursor::new(&bytes)).with_guessed_format()?;
    let format = reader.format().context("圖片格式無法辨識")?;
    let kind = match format {
        image::ImageFormat::Png => ImageKind::Png,
        image::ImageFormat::Jpeg => ImageKind::Jpeg,
        image::ImageFormat::WebP => ImageKind::Webp,
        image::ImageFormat::Gif => ImageKind::Gif,
        _ => anyhow::bail!("不支援的圖片格式"),
    };
    let (width, height) = image::ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()?
        .into_dimensions()?;
    ensure!(
        width > 0
            && height > 0
            && width <= 16384
            && height <= 16384
            && u64::from(width) * u64::from(height) <= 32_000_000,
        "圖片尺寸超過安全上限"
    );
    let mut reader = image::ImageReader::new(Cursor::new(&bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(160 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().context("圖片內容損壞")?;
    Ok(CachedImage {
        bytes: bytes.into(),
        kind,
        width,
        height,
    })
}

fn prune(directory: &PathBuf) -> Result<()> {
    let mut files = Vec::new();
    let mut total = 0u64;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.len() != 36
            || !name.ends_with(".bin")
            || !name.as_bytes()[..32].iter().all(u8::is_ascii_hexdigit)
        {
            continue;
        }
        let metadata = entry.metadata()?;
        if !metadata.is_file() {
            continue;
        }
        total += metadata.len();
        files.push((
            metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            entry.path(),
            metadata.len(),
        ));
    }
    files.sort_by_key(|(time, _, _)| *time);
    let mut count = files.len();
    for (modified, path, length) in files {
        let expired = modified
            .elapsed()
            .is_ok_and(|age| age > std::time::Duration::from_secs(30 * 86400));
        if total > DISK_BYTES || count > 4096 || expired {
            fs::remove_file(path)?;
            total = total.saturating_sub(length);
            count -= 1;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn png() -> Vec<u8> {
        let mut buffer = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(2, 2)
            .write_to(&mut buffer, image::ImageFormat::Png)
            .unwrap();
        buffer.into_inner()
    }
    #[test]
    fn image_origin_rejects_lookalikes_credentials_and_non_https() {
        for url in [
            "https://hdslb.com.evil.test/a.png",
            "https://evilhdslb.com/a.png",
            "file:///tmp/a.png",
            "https://user:secret@i0.hdslb.com/a.png",
            "https://i0.hdslb.com:8080/a.png",
        ] {
            assert!(image_url(url).is_err());
        }
        assert_eq!(
            image_url("//i0.hdslb.com/a.png").unwrap(),
            "https://i0.hdslb.com/a.png"
        );
        assert_eq!(
            original_image_url("https://i0.hdslb.com/a.png@200w.webp").unwrap(),
            "https://i0.hdslb.com/a.png"
        );
    }
    #[test]
    fn corrupt_image_is_rejected() {
        assert!(decode(vec![0; 100]).is_err());
        assert_eq!(decode(png()).unwrap().width, 2);
    }
    #[tokio::test]
    async fn disk_cache_reopens_offline_and_memory_avoids_second_io() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let url = "https://i0.hdslb.com/test.png";
        let path = dir.path().join(format!("{:x}.bin", md5::compute(url)));
        fs::write(&path, png())?;
        let cache = ImageCache::new(
            dir.path().into(),
            Client::new(),
            Arc::new(Semaphore::new(4)),
        )
        .await?;
        let first = cache.get(url).await?;
        fs::remove_file(path)?;
        let second = cache.get(url).await?;
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(second.height, 2);
        Ok(())
    }
    #[test]
    fn cache_pruning_preserves_unrelated_files() -> Result<()> {
        let dir = tempfile::tempdir()?;
        fs::write(dir.path().join("keep-me.txt"), b"user file")?;
        prune(&dir.path().into())?;
        assert!(dir.path().join("keep-me.txt").exists());
        Ok(())
    }
}
