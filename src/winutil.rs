//! Små Win32-hjälpare: meddelanderutor, single-instance-skydd och
//! loggning till fil (felsökning utan konsol i release-läge).

use std::io::Write;

pub const MB_ICONINFORMATION: u32 = 0x40;
pub const MB_ICONWARNING: u32 = 0x30;
pub const MB_ICONERROR: u32 = 0x10;

#[repr(C)]
#[allow(non_snake_case, clippy::upper_case_acronyms)]
struct SYSTEMTIME {
    wYear: u16,
    wMonth: u16,
    wDayOfWeek: u16,
    wDay: u16,
    wHour: u16,
    wMinute: u16,
    wSecond: u16,
    wMilliseconds: u16,
}

#[link(name = "user32")]
extern "system" {
    fn MessageBoxW(hwnd: isize, text: *const u16, caption: *const u16, utype: u32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateMutexW(
        attributes: *const core::ffi::c_void,
        initial_owner: i32,
        name: *const u16,
    ) -> isize;
    fn GetLastError() -> u32;
    fn GetLocalTime(time: *mut SYSTEMTIME);
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Visar en meddelanderuta (blockerar tråden tills användaren klickar OK).
pub fn message_box(title: &str, text: &str, icon: u32) {
    let text_w = wide(text);
    let title_w = wide(title);
    unsafe {
        MessageBoxW(0, text_w.as_ptr(), title_w.as_ptr(), icon);
    }
}

/// Skapar en namngiven mutex som lever lika länge som processen.
/// Returnerar false om en annan instans redan äger den.
pub fn ensure_single_instance() -> bool {
    const ERROR_ALREADY_EXISTS: u32 = 183;
    let name = wide("T-Whisper-single-instance-mutex");
    unsafe {
        let handle = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        if handle == 0 {
            // Kunde inte skapa mutexen alls - släpp igenom hellre än att vägra starta.
            return true;
        }
        // Handtaget lämnas öppet avsiktligt; OS:et städar när processen dör.
        GetLastError() != ERROR_ALREADY_EXISTS
    }
}

fn now_string() -> String {
    let mut t = SYSTEMTIME {
        wYear: 0,
        wMonth: 0,
        wDayOfWeek: 0,
        wDay: 0,
        wHour: 0,
        wMinute: 0,
        wSecond: 0,
        wMilliseconds: 0,
    };
    unsafe { GetLocalTime(&mut t) };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    )
}

/// Loggar till stderr och till %APPDATA%\T-Whisper\log.txt med tidsstämpel.
/// Enkel rotation: växer filen över ~1 MB flyttas den till log.old.txt.
pub fn log(msg: &str) {
    eprintln!("{msg}");
    let dir = crate::config::config_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("log.txt");
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > 1_000_000 {
            let old = dir.join("log.old.txt");
            let _ = std::fs::remove_file(&old);
            let _ = std::fs::rename(&path, &old);
        }
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(f, "[{}] {}", now_string(), msg);
    }
}
