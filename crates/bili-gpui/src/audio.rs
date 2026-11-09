//! 有界背景音效佇列；同步 WinMM 呼叫持有 WAV 緩衝區直到播放結束。
use std::{
    sync::{Mutex, OnceLock, mpsc},
    time::{Duration, Instant},
};
use windows::{
    Win32::Media::Audio::{PlaySoundW, SND_MEMORY, SND_NODEFAULT, SND_SYNC},
    core::PCWSTR,
};
static AUDIO: OnceLock<mpsc::SyncSender<(bool, f32)>> = OnceLock::new();
static LAST_CHIME: Mutex<Option<Instant>> = Mutex::new(None);
pub fn play(chime: bool, volume: f32) {
    if volume <= 0. {
        return;
    }
    if chime {
        let Ok(mut last) = LAST_CHIME.lock() else {
            return;
        };
        if last.is_some_and(|at| at.elapsed() < Duration::from_secs(1)) {
            return;
        }
        *last = Some(Instant::now());
    }
    let sender = AUDIO.get_or_init(|| {
        let (sender, receiver) = mpsc::sync_channel(8);
        std::thread::Builder::new()
            .name("bili-audio".into())
            .spawn(move || {
                while let Ok((chime, volume)) = receiver.recv() {
                    let bytes = wave(chime, volume);
                    let success = unsafe {
                        PlaySoundW(
                            PCWSTR(bytes.as_ptr().cast()),
                            None,
                            SND_MEMORY | SND_SYNC | SND_NODEFAULT,
                        )
                    };
                    if !success.as_bool() {
                        eprintln!("[音效] Windows 無法播放提示音");
                    }
                }
            })
            .expect("無法建立音效執行緒");
        sender
    });
    if let Err(mpsc::TrySendError::Disconnected(_)) = sender.try_send((chime, volume.clamp(0., 1.)))
    {
        eprintln!("[音效] 播放服務已中止");
    }
}
fn wave(chime: bool, volume: f32) -> Vec<u8> {
    let rate = 44100_u32;
    let duration = if chime { 0.5 } else { 0.025 };
    let count = (rate as f32 * duration) as usize;
    let length = (count * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + length as usize);
    bytes.extend(b"RIFF");
    bytes.extend((36 + length).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16_u32.to_le_bytes());
    bytes.extend(1_u16.to_le_bytes());
    bytes.extend(1_u16.to_le_bytes());
    bytes.extend(rate.to_le_bytes());
    bytes.extend((rate * 2).to_le_bytes());
    bytes.extend(2_u16.to_le_bytes());
    bytes.extend(16_u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(length.to_le_bytes());
    let mut phase = 0_f32;
    for index in 0..count {
        let t = index as f32 / rate as f32;
        let frequency = if chime {
            440. * 2_f32.powf(t / duration)
        } else {
            1700.
        };
        phase += std::f32::consts::TAU * frequency / rate as f32;
        let envelope = (t / 0.008).min(1.) * (1. - t / duration).max(0.).powi(2);
        let amplitude = volume * if chime { 0.3 } else { 0.035 };
        bytes.extend(((phase.sin() * envelope * amplitude * 32767.) as i16).to_le_bytes());
    }
    bytes
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_wave_has_valid_length_and_volume() {
        let quiet = wave(true, 0.);
        assert!(quiet[44..].iter().all(|v| *v == 0));
        let loud = wave(false, 1.);
        assert_eq!(
            u32::from_le_bytes(loud[40..44].try_into().unwrap()) as usize,
            loud.len() - 44
        );
        assert!(loud[44..].iter().any(|v| *v != 0));
    }
}
