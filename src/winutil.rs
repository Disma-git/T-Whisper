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
    fn AttachConsole(process_id: u32) -> i32;
    fn GetStdHandle(std_handle: u32) -> isize;
    fn SetStdHandle(std_handle: u32, handle: isize) -> i32;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        security: *const core::ffi::c_void,
        creation: u32,
        flags: u32,
        template: isize,
    ) -> isize;
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

/// Kopplar processen till konsolen den startades från, om det fanns en.
///
/// Release-bygget är ett GUI-program (`windows_subsystem = "windows"`) och
/// får därför ingen konsol. Utan det här syns ingenting när man kör exe:n
/// från en terminal — konsolhandtag ärvs inte, till skillnad från fil- och
/// pipe-handtag. Startas appen från Startmenyn eller autostart finns ingen
/// föräldrakonsol, och då gör funktionen ingenting.
pub fn attach_parent_console() {
    const ATTACH_PARENT_PROCESS: u32 = 0xFFFF_FFFF;
    const STD_OUTPUT_HANDLE: u32 = 0xFFFF_FFF5; // -11
    const STD_ERROR_HANDLE: u32 = 0xFFFF_FFF4; // -12
    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const FILE_SHARE_READ_WRITE: u32 = 0x0000_0003;
    const OPEN_EXISTING: u32 = 3;
    const INVALID_HANDLE_VALUE: isize = -1;

    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return; // Ingen föräldrakonsol (Startmenyn, autostart, tjänst).
        }
        // Bara handtag som saknas ersätts — annars skulle en omdirigering
        // till fil eller pipe skrivas över och utdatan hamna fel.
        for std_handle in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            let existing = GetStdHandle(std_handle);
            if existing != 0 && existing != INVALID_HANDLE_VALUE {
                continue;
            }
            let name = wide("CONOUT$");
            let handle = CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                0,
            );
            if handle != INVALID_HANDLE_VALUE {
                SetStdHandle(std_handle, handle);
            }
        }
    }
    // Skalets prompt har redan skrivits ut; en tom rad skiljer den från
    // appens utmatning så att det inte ser ut som en enda rad.
    eprintln!();
}

/// Installerar en panic-hook som skriver panicen till log.txt och visar en
/// meddelanderuta. Utan den dör en panic tyst i release-läge (inget konsol-
/// fönster finns, så stderr går ingenstans) — även panics i bakgrundstrådar.
pub fn install_panic_hook() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static BOX_SHOWN: AtomicBool = AtomicBool::new(false);

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("<namnlös>");
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<okänd plats>".to_string());
        let backtrace = std::backtrace::Backtrace::force_capture();

        let payload = info.payload();
        let msg = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("<okänd orsak>");

        log(&format!(
            "PANIC i tråden '{name}' vid {location}: {msg}\nBacktrace:\n{backtrace}"
        ));

        // Bara en ruta även om flera trådar panikar — annars staplas de på varandra.
        if !BOX_SHOWN.swap(true, Ordering::SeqCst) {
            message_box(
                "T-Whisper – oväntat fel",
                &format!(
                    "Programmet stötte på ett internt fel och kan behöva startas om.\n\n\
                     Tråd: {name}\nPlats: {location}\n\n\
                     Fullständig information finns i:\n{}",
                    crate::config::config_dir().join("log.txt").display()
                ),
                MB_ICONERROR,
            );
        }

        default_hook(info);
    }));
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
