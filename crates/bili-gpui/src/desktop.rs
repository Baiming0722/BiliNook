use anyhow::{Context, Result};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::path::Path;
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem},
};
use windows::{
    Win32::{
        Foundation::{
            CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LPARAM, WAIT_OBJECT_0,
            WPARAM,
        },
        System::Threading::{CreateEventW, CreateMutexW, SetEvent, WaitForSingleObject},
        UI::WindowsAndMessaging::{
            AllowSetForegroundWindow, CreateIcon, DestroyIcon, EnumWindows, GetForegroundWindow,
            GetPropW, GetWindowThreadProcessId, HICON, ICON_BIG, ICON_SMALL, IsIconic,
            IsWindowVisible, RemovePropW, SW_HIDE, SW_RESTORE, SW_SHOW, SendMessageW,
            SetForegroundWindow, SetPropW, ShowWindow, WM_SETICON,
        },
    },
    core::{BOOL, PCWSTR},
};

struct OwnedHandle(HANDLE);
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

pub struct Instance {
    _mutex: OwnedHandle,
    wake: OwnedHandle,
    property: Vec<u16>,
    identity: String,
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

impl Instance {
    pub fn acquire(directory: &Path) -> Result<Option<Self>> {
        // 先正規化尚未建立的路徑，讓相對/絕對路徑、大小寫與 extended prefix 共用同一把鎖。
        let mut absolute = std::path::absolute(directory)?;
        let mut missing = Vec::new();
        while !absolute.exists() {
            if let Some(name) = absolute.file_name() {
                missing.push(name.to_owned());
            }
            if !absolute.pop() {
                break;
            }
        }
        absolute = std::fs::canonicalize(&absolute).context("無法解析資料目錄")?;
        for name in missing.into_iter().rev() {
            absolute.push(name);
        }
        let identity = identity(&absolute);
        // 與更名前的版本共用喚醒機制，避免兩個程序同時管理同一份資料。
        let event_name = wide(&format!(r"Local\BiliFollow.Wake.{identity}"));
        let mutex_name = wide(&format!(r"Local\BiliFollow.Instance.{identity}"));
        let property = wide(&format!("BiliFollow.Window.{identity}"));
        let wake =
            OwnedHandle(unsafe { CreateEventW(None, false, false, PCWSTR(event_name.as_ptr()))? });
        let mutex = unsafe { CreateMutexW(None, false, PCWSTR(mutex_name.as_ptr()))? };
        let exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        let mutex = OwnedHandle(mutex);
        if exists {
            // 由新啟動的前景程序授權既有程序取得焦點；不讀寫 PID 檔或啟動第二個 Poller。
            unsafe {
                let _ = EnumWindows(
                    Some(allow_existing_foreground),
                    LPARAM(property.as_ptr() as isize),
                );
                SetEvent(wake.0)?;
            }
            return Ok(None);
        }
        Ok(Some(Self {
            _mutex: mutex,
            wake,
            property,
            identity,
        }))
    }
}

fn identity(path: &Path) -> String {
    let text = path.to_string_lossy().replace('/', "\\").to_lowercase();
    let text = text
        .strip_prefix(r"\\?\")
        .unwrap_or(&text)
        .trim_end_matches('\\');
    format!("{:x}", md5::compute(text.as_bytes()))
}

unsafe extern "system" fn allow_existing_foreground(hwnd: HWND, parameter: LPARAM) -> BOOL {
    let property = PCWSTR(parameter.0 as *const u16);
    if !unsafe { GetPropW(hwnd, property) }.is_invalid() {
        let mut process = 0;
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&mut process));
            let _ = AllowSetForegroundWindow(process);
        }
    }
    true.into()
}

pub enum DesktopEvent {
    Show,
    Toggle,
    Quit,
}

pub struct Desktop {
    instance: Instance,
    hwnd: HWND,
    tray: Option<TrayIcon>,
    icon_small: Option<HICON>,
    icon_big: Option<HICON>,
    toggle: MenuId,
    quit: MenuId,
    tooltip: String,
    pub warning: Option<String>,
}

fn create_win32_icon(decoded: &image::DynamicImage, size: u32) -> Option<HICON> {
    let mut bgra = decoded
        .resize_exact(size, size, image::imageops::FilterType::Lanczos3)
        .into_rgba8()
        .into_raw();
    for pixel in bgra.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let mask_len = (size as usize).div_ceil(8) * (size as usize);
    let and_mask = vec![0u8; mask_len];
    unsafe {
        CreateIcon(
            None,
            size as i32,
            size as i32,
            1,
            32,
            and_mask.as_ptr(),
            bgra.as_ptr(),
        )
        .ok()
    }
}

impl Desktop {
    pub fn attach(instance: Instance, window: &gpui::Window) -> Result<Self> {
        let RawWindowHandle::Win32(handle) = HasWindowHandle::window_handle(window)
            .map_err(|error| anyhow::anyhow!("原生視窗代碼不可用：{error:?}"))?
            .as_raw()
        else {
            anyhow::bail!("此版本需要原生 Windows 視窗");
        };
        let hwnd = HWND(handle.hwnd.get() as *mut _);
        unsafe {
            SetPropW(
                hwnd,
                PCWSTR(instance.property.as_ptr()),
                Some(HANDLE(1usize as *mut _)),
            )?;
        }
        let decoded_icon = image::load_from_memory(include_bytes!("../assets/icon.png")).ok();
        let icon_small = decoded_icon
            .as_ref()
            .and_then(|img| create_win32_icon(img, 16));
        let icon_big = decoded_icon
            .as_ref()
            .and_then(|img| create_win32_icon(img, 32));
        unsafe {
            if let Some(hicon) = icon_small {
                SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(ICON_SMALL as usize)),
                    Some(LPARAM(hicon.0 as isize)),
                );
            }
            if let Some(hicon) = icon_big {
                SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(ICON_BIG as usize)),
                    Some(LPARAM(hicon.0 as isize)),
                );
            }
        }
        let toggle = MenuItem::with_id("bili-toggle", "顯示／隱藏視窗", true, None);
        let quit = MenuItem::with_id("bili-quit", "退出應用", true, None);
        let tray = (|| -> Result<TrayIcon> {
            let menu = Menu::new();
            menu.append_items(&[&toggle, &PredefinedMenuItem::separator(), &quit])?;
            let image = image::load_from_memory(include_bytes!("../assets/icon.png"))?
                .resize_exact(32, 32, image::imageops::FilterType::Lanczos3)
                .into_rgba8();
            let icon = Icon::from_rgba(image.into_raw(), 32, 32)?;
            let tray = TrayIconBuilder::new()
                .with_id(format!("bili-{}", instance.identity))
                .with_icon(icon)
                .with_tooltip(bili_core::APP_NAME)
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(false)
                .build()?;
            // Builder 允許 Explorer 尚未啟動，並不代表圖示已登錄；確認可用才允許關閉至托盤。
            tray.set_tooltip(Some(bili_core::APP_NAME))?;
            Ok(tray)
        })();
        let (tray, warning) = match tray {
            Ok(tray) => (Some(tray), None),
            Err(error) => (
                None,
                Some(format!("系統托盤無法建立，關閉視窗將退出應用：{error}")),
            ),
        };
        Ok(Self {
            instance,
            hwnd,
            tray,
            icon_small,
            icon_big,
            toggle: toggle.id().clone(),
            quit: quit.id().clone(),
            tooltip: String::new(),
            warning,
        })
    }

    pub fn has_tray(&self) -> bool {
        self.tray.is_some()
    }
    pub fn is_foreground(&self) -> bool {
        unsafe {
            let active = GetForegroundWindow();
            let mut process = 0;
            GetWindowThreadProcessId(active, Some(&mut process));
            process == std::process::id()
                && IsWindowVisible(active).as_bool()
                && !IsIconic(active).as_bool()
        }
    }
    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }
    pub fn show(&self) {
        unsafe {
            let _ = ShowWindow(
                self.hwnd,
                if IsIconic(self.hwnd).as_bool() {
                    SW_RESTORE
                } else {
                    SW_SHOW
                },
            );
            let _ = SetForegroundWindow(self.hwnd);
        }
    }
    pub fn toggle(&self) {
        if unsafe { IsWindowVisible(self.hwnd).as_bool() }
            && !unsafe { IsIconic(self.hwnd).as_bool() }
        {
            self.hide();
        } else {
            self.show();
        }
    }
    pub fn poll(&self) -> Vec<DesktopEvent> {
        let mut events = Vec::new();
        if unsafe { WaitForSingleObject(self.instance.wake.0, 0) } == WAIT_OBJECT_0 {
            events.push(DesktopEvent::Show);
        }
        for event in MenuEvent::receiver().try_iter().take(64) {
            if event.id == self.toggle {
                events.push(DesktopEvent::Toggle);
            }
            if event.id == self.quit {
                events.push(DesktopEvent::Quit);
            }
        }
        for event in TrayIconEvent::receiver().try_iter().take(64) {
            if self
                .tray
                .as_ref()
                .is_none_or(|tray| event.id() != tray.id())
            {
                continue;
            }
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } | TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
            ) {
                events.push(DesktopEvent::Show);
            }
        }
        events
    }
    pub fn update_tooltip(&mut self, live: usize, refreshing: bool) {
        let text = format!(
            "{} · {live} 位直播中{}",
            bili_core::APP_NAME,
            if refreshing { " · 檢查中" } else { "" }
        );
        if text == self.tooltip {
            return;
        }
        if let Some(tray) = &self.tray
            && let Err(error) = tray.set_tooltip(Some(&text))
        {
            eprintln!("[托盤] 更新提示失敗：{error}");
        }
        self.tooltip = text;
    }
}
impl Drop for Desktop {
    fn drop(&mut self) {
        unsafe {
            let _ = RemovePropW(self.hwnd, PCWSTR(self.instance.property.as_ptr()));
            if let Some(hicon) = self.icon_small.take() {
                let _ = DestroyIcon(hicon);
            }
            if let Some(hicon) = self.icon_big.take() {
                let _ = DestroyIcon(hicon);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identity_matches_windows_path_aliases() {
        assert_eq!(
            identity(Path::new(r"C:\Users\A\data")),
            identity(Path::new(r"\\?\c:\users\a\data\"))
        );
        assert_ne!(
            identity(Path::new(r"C:\test-a")),
            identity(Path::new(r"C:\test-b"))
        );
    }
}
