#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod config;
mod model;
mod sound;
mod transcribe;

use anyhow::{Context, Result};
use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use global_hotkey::{hotkey::HotKey, GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use tao::event::Event;
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::TrayIconBuilder;

const HOTKEY_CHOICES: [&str; 12] = [
    "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12",
];

const VOLUME_CHOICES: [(&str, f32); 4] =
    [("Av", 0.0), ("Låg", 0.2), ("Mellan", 0.5), ("Hög", 1.0)];

// Grundnivåer för feedbackljuden; skalas med konfigurerad volym.
const START_SOUND_GAIN: f32 = 0.5;
const STOP_SOUND_GAIN: f32 = 0.36;

enum Cmd {
    StartRecording,
    StopAndTranscribe,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum AppState {
    Idle,
    Recording,
    Working,
}

enum UserEvent {
    State(AppState),
    Level(f32),
}

fn main() -> Result<()> {
    let mut cfg = config::Config::load().context("kunde inte läsa konfigurationen")?;
    eprintln!(
        "T-Whisper startar — modell: kb-whisper-{}, hotkey: {}",
        cfg.model, cfg.hotkey
    );
    let model_path = model::ensure_model(&cfg.model)?;

    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<Cmd>();
    let mic_level: Arc<AtomicU32> = Arc::default();
    // Delad volym (f32 som bitar) så att menyändringar slår igenom direkt.
    let sound_volume = Arc::new(AtomicU32::new(
        cfg.sound_volume.clamp(0.0, 1.0).to_bits(),
    ));

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();

    // Arbetstråden äger mikrofon, whisper-modell och tangentbordsutmatning.
    {
        let cfg = cfg.clone();
        let proxy = event_loop.create_proxy();
        let mic_level = mic_level.clone();
        let sound_volume = sound_volume.clone();
        std::thread::spawn(move || {
            controller(model_path, cfg, cmd_rx, proxy, mic_level, sound_volume)
        });
    }

    // Nivåmätar-ticker: läser av (och nollställer) mikrofonens toppnivå 10 ggr/s.
    {
        let proxy = event_loop.create_proxy();
        let mic_level = mic_level.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(100));
            let peak = f32::from_bits(mic_level.swap(0, Ordering::Relaxed));
            if proxy.send_event(UserEvent::Level(peak)).is_err() {
                return;
            }
        });
    }

    // Systemfältsikon med kontrollpanelsmeny.
    let menu = Menu::new();
    let hotkey_submenu = Submenu::new("Inspelningsknapp", true);
    let mut key_items: Vec<(CheckMenuItem, String)> = Vec::new();
    for name in HOTKEY_CHOICES {
        let item = CheckMenuItem::new(name, true, name == cfg.hotkey, None);
        hotkey_submenu.append(&item)?;
        key_items.push((item, name.to_string()));
    }
    menu.append(&hotkey_submenu)?;
    let volume_submenu = Submenu::new("Ljudvolym", true);
    let mut vol_items: Vec<(CheckMenuItem, f32)> = Vec::new();
    for (name, vol) in VOLUME_CHOICES {
        let checked = (cfg.sound_volume - vol).abs() < 0.05;
        let item = CheckMenuItem::new(name, true, checked, None);
        volume_submenu.append(&item)?;
        vol_items.push((item, vol));
    }
    menu.append(&volume_submenu)?;
    let open_cfg_item = MenuItem::new("Öppna konfigurationsfil", true, None);
    menu.append(&open_cfg_item)?;
    menu.append(&PredefinedMenuItem::separator())?;
    let quit_item = MenuItem::new("Avsluta", true, None);
    menu.append(&quit_item)?;

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(format!("T-Whisper — håll {} och prata", cfg.hotkey))
        .with_icon(make_icon(AppState::Idle, 0.0))
        .build()?;

    // Global push-to-talk-tangent.
    let mut hotkey: HotKey = cfg
        .hotkey
        .parse()
        .map_err(|e| anyhow::anyhow!("ogiltig hotkey '{}': {e:?}", cfg.hotkey))?;
    let hotkey_manager = GlobalHotKeyManager::new()?;
    hotkey_manager.register(hotkey)?;

    let hotkey_rx = GlobalHotKeyEvent::receiver();
    let menu_rx = MenuEvent::receiver();

    eprintln!(
        "Redo. Håll {} och prata; släpp för att skriva texten.",
        cfg.hotkey
    );

    let mut cur_state = AppState::Idle;
    let mut cur_level = 0f32;

    event_loop.run(move |event, _, control_flow| {
        // Kort poll-intervall: global-hotkeys kanal väcker inte tao-loopen av
        // sig själv, så vi tittar med jämna mellanrum (försumbar CPU-kostnad).
        *control_flow = ControlFlow::WaitUntil(
            std::time::Instant::now() + std::time::Duration::from_millis(30),
        );

        if let Event::UserEvent(ev) = &event {
            match ev {
                UserEvent::State(s) => {
                    cur_state = *s;
                    let _ = tray.set_icon(Some(make_icon(cur_state, cur_level)));
                }
                UserEvent::Level(peak) => {
                    // Perceptuell skalning + mjuk avklingning av mätaren.
                    let target = (peak * 6.0).min(1.0);
                    let next = if target > cur_level {
                        target
                    } else {
                        cur_level * 0.6
                    };
                    if (next - cur_level).abs() > 0.02 || (next == 0.0 && cur_level > 0.0) {
                        cur_level = next;
                        let _ = tray.set_icon(Some(make_icon(cur_state, cur_level)));
                    }
                }
            }
        }

        while let Ok(e) = hotkey_rx.try_recv() {
            if e.id == hotkey.id() {
                match e.state {
                    HotKeyState::Pressed => {
                        let _ = cmd_tx.send(Cmd::StartRecording);
                    }
                    HotKeyState::Released => {
                        let _ = cmd_tx.send(Cmd::StopAndTranscribe);
                    }
                }
            }
        }

        while let Ok(e) = menu_rx.try_recv() {
            if e.id == quit_item.id() {
                *control_flow = ControlFlow::Exit;
            } else if e.id == open_cfg_item.id() {
                // Absolut sökväg + lokal arbetskatalog: ärvd cwd kan ligga på
                // en nätverksenhet som paketerade Anteckningar inte hanterar.
                let cfg_path = config::config_dir().join("config.toml");
                let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
                let notepad = std::path::Path::new(&windir)
                    .join("System32")
                    .join("notepad.exe");
                if let Err(err) = std::process::Command::new(notepad)
                    .arg(&cfg_path)
                    .current_dir(config::config_dir())
                    .spawn()
                {
                    eprintln!("kunde inte öppna {}: {err}", cfg_path.display());
                }
            } else if let Some((_, name)) = key_items.iter().find(|(item, _)| e.id == *item.id())
            {
                match name.parse::<HotKey>() {
                    Ok(new_hotkey) => {
                        let _ = hotkey_manager.unregister(hotkey);
                        match hotkey_manager.register(new_hotkey) {
                            Ok(()) => {
                                hotkey = new_hotkey;
                                cfg.hotkey = name.clone();
                                if let Err(e) = cfg.save() {
                                    eprintln!("kunde inte spara konfigurationen: {e}");
                                }
                                let _ = tray.set_tooltip(Some(format!(
                                    "T-Whisper — håll {} och prata",
                                    cfg.hotkey
                                )));
                                eprintln!("Inspelningsknapp ändrad till {}", cfg.hotkey);
                            }
                            Err(e) => {
                                eprintln!("kunde inte registrera {name}: {e}");
                                // Återta den gamla tangenten så appen inte blir döv.
                                let _ = hotkey_manager.register(hotkey);
                            }
                        }
                    }
                    Err(e) => eprintln!("ogiltig tangent {name}: {e:?}"),
                }
            } else if let Some((_, vol)) = vol_items.iter().find(|(item, _)| e.id == *item.id()) {
                sound_volume.store(vol.to_bits(), Ordering::Relaxed);
                cfg.sound_volume = *vol;
                if let Err(e) = cfg.save() {
                    eprintln!("kunde inte spara konfigurationen: {e}");
                }
                // Spela ett prov så användaren hör nya nivån direkt.
                if cfg.sounds && *vol > 0.0 {
                    sound::play(880.0, 140, STOP_SOUND_GAIN * vol);
                }
                eprintln!("Ljudvolym ändrad till {vol}");
            }
            // Synka bockarna i menyn med aktuell konfiguration.
            for (item, name) in &key_items {
                item.set_checked(*name == cfg.hotkey);
            }
            for (item, vol) in &vol_items {
                item.set_checked((cfg.sound_volume - vol).abs() < 0.05);
            }
        }
    });
}

fn controller(
    model_path: PathBuf,
    cfg: config::Config,
    rx: Receiver<Cmd>,
    proxy: EventLoopProxy<UserEvent>,
    mic_level: Arc<AtomicU32>,
    sound_volume: Arc<AtomicU32>,
) {
    let recorder = match audio::Recorder::new(mic_level) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("FEL: kunde inte öppna mikrofonen: {e}");
            return;
        }
    };
    eprintln!("Laddar whisper-modellen…");
    let t0 = std::time::Instant::now();
    let transcriber = match transcribe::Transcriber::new(
        model_path.to_str().expect("ogiltig modellsökväg"),
        &cfg.language,
    ) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("FEL: kunde inte ladda modellen: {e}");
            return;
        }
    };
    eprintln!("Modellen laddad på {:.1} s.", t0.elapsed().as_secs_f32());
    let mut enigo = match Enigo::new(&Settings::default()) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("FEL: kunde inte initiera tangentbordsutmatning: {e}");
            return;
        }
    };

    let mut recording = false;
    for cmd in rx {
        match cmd {
            Cmd::StartRecording if !recording => {
                recording = true;
                recorder.start();
                let vol = f32::from_bits(sound_volume.load(Ordering::Relaxed));
                if cfg.sounds && vol > 0.0 {
                    sound::play(300.0, 60, START_SOUND_GAIN * vol); // dovt klick
                }
                let _ = proxy.send_event(UserEvent::State(AppState::Recording));
            }
            Cmd::StopAndTranscribe if recording => {
                recording = false;
                let vol = f32::from_bits(sound_volume.load(Ordering::Relaxed));
                if cfg.sounds && vol > 0.0 {
                    sound::play(880.0, 140, STOP_SOUND_GAIN * vol); // pling
                }
                let _ = proxy.send_event(UserEvent::State(AppState::Working));
                let samples = recorder.stop();
                // Ignorera tryck kortare än ~0,25 s.
                if samples.len() >= 4_000 {
                    let t = std::time::Instant::now();
                    match transcriber.transcribe(&samples) {
                        Ok(text) if !text.is_empty() => {
                            eprintln!("[{:.2} s] {text}", t.elapsed().as_secs_f32());
                            let out = if cfg.append_space {
                                format!("{text} ")
                            } else {
                                text
                            };
                            if let Err(e) = insert_text(&mut enigo, &out, cfg.paste) {
                                eprintln!("kunde inte skriva texten: {e}");
                            }
                        }
                        Ok(_) => eprintln!("(inget tal uppfattades)"),
                        Err(e) => eprintln!("transkriberingsfel: {e}"),
                    }
                }
                let _ = proxy.send_event(UserEvent::State(AppState::Idle));
            }
            _ => {}
        }
    }
}

/// Skriver in text vid markören. Urklipp + Ctrl+V är standard eftersom
/// teckenvis SendInput tappar tecken i många program; användarens
/// gamla urklipp återställs efteråt.
fn insert_text(enigo: &mut Enigo, text: &str, paste: bool) -> Result<()> {
    if !paste {
        enigo.text(text)?;
        return Ok(());
    }
    let mut clipboard = arboard::Clipboard::new()?;
    let previous = clipboard.get_text().ok();
    clipboard.set_text(text.to_string())?;
    std::thread::sleep(std::time::Duration::from_millis(50));
    enigo.key(Key::Control, Direction::Press)?;
    enigo.key(Key::Unicode('v'), Direction::Click)?;
    enigo.key(Key::Control, Direction::Release)?;
    // Ge mottagarprogrammet tid att läsa urklippet innan det återställs.
    std::thread::sleep(std::time::Duration::from_millis(300));
    if let Some(prev) = previous {
        let _ = clipboard.set_text(prev);
    }
    Ok(())
}

/// Ikon: statusfärgad ring (blå = redo, röd = inspelning, gul = transkriberar)
/// med en grön nivåstapel i mitten som följer mikrofonens ljudnivå.
fn make_icon(state: AppState, level: f32) -> tray_icon::Icon {
    const S: i32 = 32;
    let (rr, rg, rb) = match state {
        AppState::Idle => (90u8, 120u8, 200u8),
        AppState::Recording => (220, 60, 60),
        AppState::Working => (230, 170, 50),
    };
    let mut rgba = vec![0u8; (S * S * 4) as usize];
    let c = (S as f32 - 1.0) / 2.0;
    let outer = c - 0.5;
    let inner = outer - 3.0;

    // Nivåstapel: 10 px bred, växer nedifrån och upp inom den inre cirkeln.
    let bar_half_w = 5;
    let bar_bottom = 24;
    let bar_top_full = 8;
    let bar_top = bar_bottom - ((bar_bottom - bar_top_full) as f32 * level.clamp(0.0, 1.0)) as i32;

    for y in 0..S {
        for x in 0..S {
            let dx = x as f32 - c;
            let dy = y as f32 - c;
            let d = (dx * dx + dy * dy).sqrt();
            let i = ((y * S + x) * 4) as usize;
            if d <= outer && d > inner {
                // statusring
                rgba[i] = rr;
                rgba[i + 1] = rg;
                rgba[i + 2] = rb;
                rgba[i + 3] = 255;
            } else if d <= inner {
                let in_bar = (x - S / 2).abs() <= bar_half_w && y >= bar_top && y <= bar_bottom;
                if in_bar {
                    // grön nivåstapel
                    rgba[i] = 70;
                    rgba[i + 1] = 200;
                    rgba[i + 2] = 90;
                    rgba[i + 3] = 255;
                } else {
                    // mörk botten
                    rgba[i] = 38;
                    rgba[i + 1] = 38;
                    rgba[i + 2] = 44;
                    rgba[i + 3] = 255;
                }
            }
        }
    }
    tray_icon::Icon::from_rgba(rgba, S as u32, S as u32).expect("ikon")
}
