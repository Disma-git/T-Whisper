#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod config;
mod model;
mod numbers;
mod sound;
mod transcribe;
mod winutil;

use anyhow::{Context, Result};
use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use global_hotkey::{hotkey::HotKey, GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use tao::event::Event;
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::TrayIconBuilder;
use winutil::log;

const HOTKEY_CHOICES: [&str; 12] = [
    "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12",
];

const VOLUME_CHOICES: [(&str, f32); 4] = [("Av", 0.0), ("Låg", 0.2), ("Mellan", 0.5), ("Hög", 1.0)];

const VERSION: &str = env!("CARGO_PKG_VERSION");
const REPO_URL: &str = "https://github.com/Disma-git/T-Whisper";

// Grundnivåer för feedbackljuden; skalas med konfigurerad volym.
const START_SOUND_GAIN: f32 = 0.5;
const STOP_SOUND_GAIN: f32 = 0.36;

/// Antal steg i nivåmätaren (utöver 0). Ikoner förrenderas per steg.
const LEVEL_STEPS: usize = 8;

enum Cmd {
    StartRecording,
    StopAndTranscribe,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum AppState {
    Loading,
    Idle,
    Recording,
    Working,
}

enum UserEvent {
    State(AppState),
    /// Kvantiserat mätarsteg 0..=LEVEL_STEPS.
    Level(usize),
    Hotkey(GlobalHotKeyEvent),
    Menu(MenuEvent),
}

fn main() {
    // Bara en instans åt gången: två instanser slåss annars om hotkey
    // och mikrofon, och tvåan dör tyst utan konsol.
    if !winutil::ensure_single_instance() {
        winutil::message_box(
            "T-Whisper",
            "T-Whisper kör redan — titta i systemfältet.",
            winutil::MB_ICONINFORMATION,
        );
        return;
    }
    if let Err(e) = run() {
        log(&format!("FATALT FEL: {e:#}"));
        winutil::message_box(
            "T-Whisper – fel vid start",
            &format!(
                "{e:#}\n\nMer information finns i:\n{}",
                config::config_dir().join("log.txt").display()
            ),
            winutil::MB_ICONERROR,
        );
    }
}

fn run() -> Result<()> {
    // Tysta whisper.cpp/ggml:s pratiga stderr-dumpar.
    whisper_rs::install_logging_hooks();

    let mut cfg = config::Config::load().context("kunde inte läsa konfigurationen")?;
    log(&format!(
        "T-Whisper v{VERSION} startar — modell: kb-whisper-{}, hotkey: {}",
        cfg.model, cfg.hotkey
    ));

    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<Cmd>();
    let mic_level: Arc<AtomicU32> = Arc::default();
    // Delad volym (f32 som bitar) så att menyändringar slår igenom direkt.
    let sound_volume = Arc::new(AtomicU32::new(cfg.sound_volume.clamp(0.0, 1.0).to_bits()));
    // Delad flagga: skicka Shift+Enter efter varje diktering.
    let shift_enter = Arc::new(AtomicBool::new(cfg.shift_enter));
    // Delad flagga: skriv tal som siffror.
    let digits = Arc::new(AtomicBool::new(cfg.digits));

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();

    // Arbetstråden äger modellnedladdning, mikrofon, whisper och
    // tangentbordsutmatning — trayikonen visas direkt medan den jobbar.
    {
        let cfg = cfg.clone();
        let proxy = event_loop.create_proxy();
        let mic_level = mic_level.clone();
        let sound_volume = sound_volume.clone();
        let shift_enter = shift_enter.clone();
        let digits = digits.clone();
        std::thread::spawn(move || {
            controller(
                cfg,
                cmd_rx,
                proxy,
                mic_level,
                sound_volume,
                shift_enter,
                digits,
            )
        });
    }

    // Nivåmätar-ticker: läser av (och nollställer) toppnivån 10 ggr/s men
    // skickar bara händelser när det kvantiserade steget faktiskt ändras —
    // vid tystnad väcks alltså inte event-loopen alls.
    {
        let proxy = event_loop.create_proxy();
        let mic_level = mic_level.clone();
        std::thread::spawn(move || {
            let mut shown = 0f32;
            let mut last_step = usize::MAX;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(100));
                let peak = f32::from_bits(mic_level.swap(0, Ordering::Relaxed));
                let target = (peak * 6.0).min(1.0);
                shown = if target > shown { target } else { shown * 0.5 };
                if shown < 0.02 {
                    shown = 0.0;
                }
                let step = (shown * LEVEL_STEPS as f32).round() as usize;
                if step != last_step {
                    last_step = step;
                    if proxy.send_event(UserEvent::Level(step)).is_err() {
                        return;
                    }
                }
            }
        });
    }

    // Förrenderade ikoner: [status][mätarsteg].
    let icons = build_icon_set();

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
    let shift_enter_item = CheckMenuItem::new(
        "Shift+Enter efter varje diktering",
        true,
        cfg.shift_enter,
        None,
    );
    menu.append(&shift_enter_item)?;
    let digits_item = CheckMenuItem::new("Skriv tal som siffror", true, cfg.digits, None);
    menu.append(&digits_item)?;
    let open_cfg_item = MenuItem::new("Öppna konfigurationsfil", true, None);
    menu.append(&open_cfg_item)?;
    menu.append(&PredefinedMenuItem::separator())?;
    let about_item = MenuItem::new(format!("Om T-Whisper… (v{VERSION})"), true, None);
    menu.append(&about_item)?;
    let github_item = MenuItem::new("Öppna GitHub-sidan", true, None);
    menu.append(&github_item)?;
    menu.append(&PredefinedMenuItem::separator())?;
    let quit_item = MenuItem::new("Avsluta", true, None);
    menu.append(&quit_item)?;

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(format!("T-Whisper v{VERSION} — startar…"))
        .with_icon(icons[state_index(AppState::Loading)][0].clone())
        .build()?;

    // Global push-to-talk-tangent.
    let mut hotkey: HotKey = cfg
        .hotkey
        .parse()
        .map_err(|e| anyhow::anyhow!("ogiltig hotkey '{}': {e:?}", cfg.hotkey))?;
    let hotkey_manager = GlobalHotKeyManager::new()?;
    hotkey_manager.register(hotkey)?;

    // Händelsestyrt i stället för polling: hotkey- och menyhändelser
    // skickas in i event-loopen via proxyn, så loopen kan sova (Wait)
    // med ~0 % CPU i vila.
    {
        let proxy = event_loop.create_proxy();
        GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
            let _ = proxy.send_event(UserEvent::Hotkey(e));
        }));
        let proxy = event_loop.create_proxy();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let _ = proxy.send_event(UserEvent::Menu(e));
        }));
    }

    log(&format!(
        "Redo. Håll {} och prata; släpp för att skriva texten.",
        cfg.hotkey
    ));

    let mut cur_state = AppState::Loading;
    let mut cur_step = 0usize;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        let Event::UserEvent(ev) = event else {
            return;
        };
        match ev {
            UserEvent::State(s) => {
                let first_ready = cur_state == AppState::Loading && s != AppState::Loading;
                cur_state = s;
                let _ = tray.set_icon(Some(icons[state_index(cur_state)][cur_step].clone()));
                if first_ready {
                    let _ = tray.set_tooltip(Some(format!(
                        "T-Whisper v{VERSION} — håll {} och prata",
                        cfg.hotkey
                    )));
                }
            }
            UserEvent::Level(step) => {
                if step != cur_step {
                    cur_step = step.min(LEVEL_STEPS);
                    let _ = tray.set_icon(Some(icons[state_index(cur_state)][cur_step].clone()));
                }
            }
            UserEvent::Hotkey(e) => {
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
            UserEvent::Menu(e) => {
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
                        log(&format!("kunde inte öppna {}: {err}", cfg_path.display()));
                    }
                } else if e.id == shift_enter_item.id() {
                    cfg.shift_enter = !cfg.shift_enter;
                    shift_enter.store(cfg.shift_enter, Ordering::Relaxed);
                    shift_enter_item.set_checked(cfg.shift_enter);
                    if let Err(e) = cfg.save() {
                        log(&format!("kunde inte spara konfigurationen: {e}"));
                    }
                } else if e.id == digits_item.id() {
                    cfg.digits = !cfg.digits;
                    digits.store(cfg.digits, Ordering::Relaxed);
                    digits_item.set_checked(cfg.digits);
                    if let Err(e) = cfg.save() {
                        log(&format!("kunde inte spara konfigurationen: {e}"));
                    }
                } else if e.id == about_item.id() {
                    show_about();
                } else if e.id == github_item.id() {
                    if let Err(err) = std::process::Command::new("explorer")
                        .arg(REPO_URL)
                        .current_dir(config::config_dir())
                        .spawn()
                    {
                        log(&format!("kunde inte öppna {REPO_URL}: {err}"));
                    }
                } else if let Some((_, name)) =
                    key_items.iter().find(|(item, _)| e.id == *item.id())
                {
                    match name.parse::<HotKey>() {
                        Ok(new_hotkey) => {
                            let _ = hotkey_manager.unregister(hotkey);
                            match hotkey_manager.register(new_hotkey) {
                                Ok(()) => {
                                    hotkey = new_hotkey;
                                    cfg.hotkey = name.clone();
                                    if let Err(e) = cfg.save() {
                                        log(&format!("kunde inte spara konfigurationen: {e}"));
                                    }
                                    let _ = tray.set_tooltip(Some(format!(
                                        "T-Whisper v{VERSION} — håll {} och prata",
                                        cfg.hotkey
                                    )));
                                    log(&format!("Inspelningsknapp ändrad till {}", cfg.hotkey));
                                }
                                Err(e) => {
                                    log(&format!("kunde inte registrera {name}: {e}"));
                                    // Återta den gamla tangenten så appen inte blir döv.
                                    let _ = hotkey_manager.register(hotkey);
                                }
                            }
                        }
                        Err(e) => log(&format!("ogiltig tangent {name}: {e:?}")),
                    }
                } else if let Some((_, vol)) = vol_items.iter().find(|(item, _)| e.id == *item.id())
                {
                    sound_volume.store(vol.to_bits(), Ordering::Relaxed);
                    cfg.sound_volume = *vol;
                    if let Err(e) = cfg.save() {
                        log(&format!("kunde inte spara konfigurationen: {e}"));
                    }
                    // Spela ett prov så användaren hör nya nivån direkt.
                    if cfg.sounds && *vol > 0.0 {
                        sound::play(880.0, 140, STOP_SOUND_GAIN * vol);
                    }
                }
                // Synka bockarna i menyn med aktuell konfiguration.
                for (item, name) in &key_items {
                    item.set_checked(*name == cfg.hotkey);
                }
                for (item, vol) in &vol_items {
                    item.set_checked((cfg.sound_volume - vol).abs() < 0.05);
                }
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn controller(
    cfg: config::Config,
    rx: Receiver<Cmd>,
    proxy: EventLoopProxy<UserEvent>,
    mic_level: Arc<AtomicU32>,
    sound_volume: Arc<AtomicU32>,
    shift_enter: Arc<AtomicBool>,
    digits: Arc<AtomicBool>,
) {
    let fail = |title: &str, err: &str| {
        log(&format!("FEL: {err}"));
        winutil::message_box(title, err, winutil::MB_ICONERROR);
    };

    let _ = proxy.send_event(UserEvent::State(AppState::Loading));

    // Modellnedladdning och -laddning sker här så att trayikonen
    // hinner visas direkt vid start.
    let model_path = match model::ensure_model(&cfg.model) {
        Ok(p) => p,
        Err(e) => {
            fail(
                "T-Whisper – modellfel",
                &format!(
                    "Kunde inte hämta modellen kb-whisper-{}:\n{e:#}\n\n\
                     Kontrollera internetanslutningen och starta om T-Whisper.",
                    cfg.model
                ),
            );
            return;
        }
    };
    let mut recorder = match audio::Recorder::new(mic_level.clone()) {
        Ok(r) => r,
        Err(e) => {
            fail(
                "T-Whisper – mikrofonfel",
                &format!("Kunde inte öppna mikrofonen:\n{e:#}"),
            );
            return;
        }
    };
    log("Laddar whisper-modellen…");
    let t0 = std::time::Instant::now();
    let mut transcriber = match transcribe::Transcriber::new(
        model_path.to_str().expect("ogiltig modellsökväg"),
        &cfg.language,
    ) {
        Ok(t) => t,
        Err(e) => {
            fail(
                "T-Whisper – modellfel",
                &format!("Kunde inte ladda modellen:\n{e:#}"),
            );
            return;
        }
    };
    log(&format!(
        "Modellen laddad på {:.1} s.",
        t0.elapsed().as_secs_f32()
    ));
    let mut enigo = match Enigo::new(&Settings::default()) {
        Ok(e) => e,
        Err(e) => {
            fail(
                "T-Whisper – inmatningsfel",
                &format!("Kunde inte initiera tangentbordsutmatningen:\n{e}"),
            );
            return;
        }
    };

    let _ = proxy.send_event(UserEvent::State(AppState::Idle));

    let mut recording = false;
    for cmd in rx {
        match cmd {
            Cmd::StartRecording if !recording => {
                // Föll mikrofonströmmen (t.ex. headset avstängt) byggs
                // inspelaren om mot nuvarande standardenhet.
                if recorder.failed() {
                    log("mikrofonströmmen har fallit — bygger om mot aktuell enhet");
                    match audio::Recorder::new(mic_level.clone()) {
                        Ok(r) => recorder = r,
                        Err(e) => {
                            log(&format!("kunde inte öppna mikrofonen: {e}"));
                            continue;
                        }
                    }
                }
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
                    match transcriber.transcribe(&samples, digits.load(Ordering::Relaxed)) {
                        Ok(text) if !text.is_empty() => {
                            log(&format!("[{:.2} s] {text}", t.elapsed().as_secs_f32()));
                            let out = if cfg.append_space {
                                format!("{text} ")
                            } else {
                                text
                            };
                            if let Err(e) = insert_text(&mut enigo, &out, cfg.paste) {
                                log(&format!("kunde inte skriva texten: {e}"));
                            } else if shift_enter.load(Ordering::Relaxed) {
                                // Ny rad efter dikteringen (mjuk radbrytning).
                                let _ = enigo.key(Key::Shift, Direction::Press);
                                let _ = enigo.key(Key::Return, Direction::Click);
                                let _ = enigo.key(Key::Shift, Direction::Release);
                            }
                        }
                        Ok(_) => log("(inget tal uppfattades)"),
                        Err(e) => log(&format!("transkriberingsfel: {e}")),
                    }
                }
                let _ = proxy.send_event(UserEvent::State(AppState::Idle));
            }
            _ => {}
        }
    }
}

/// Visar en Om-ruta med version, projektinfo och GitHub-länk.
/// Körs i egen tråd så att meddelanderutan inte blockerar event-loopen.
fn show_about() {
    std::thread::spawn(|| {
        let engine = if cfg!(feature = "cuda") {
            "whisper.cpp (NVIDIA CUDA)"
        } else if cfg!(feature = "vulkan") {
            "whisper.cpp (Vulkan)"
        } else {
            "whisper.cpp (CPU)"
        };
        let text = format!(
            "T-Whisper v{VERSION}\n\
             \n\
             Push-to-talk-diktering på svenska för Windows 11.\n\
             Håll inspelningsknappen, prata, släpp — texten skrivs\n\
             in vid markören. Allt körs lokalt, inget skickas till molnet.\n\
             \n\
             Motor: {engine}\n\
             Modell: KB-Whisper (Kungliga biblioteket)\n\
             \n\
             Projekt och källkod:\n\
             {REPO_URL}"
        );
        winutil::message_box("Om T-Whisper", &text, winutil::MB_ICONINFORMATION);
    });
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

fn state_index(state: AppState) -> usize {
    match state {
        AppState::Loading => 0,
        AppState::Idle => 1,
        AppState::Recording => 2,
        AppState::Working => 3,
    }
}

/// Förrenderar alla ikoner (4 statusar × nivåsteg) en gång vid start
/// så att mätaruppdateringar bara klonar ett handtag.
fn build_icon_set() -> Vec<Vec<tray_icon::Icon>> {
    [
        AppState::Loading,
        AppState::Idle,
        AppState::Recording,
        AppState::Working,
    ]
    .iter()
    .map(|state| {
        (0..=LEVEL_STEPS)
            .map(|step| make_icon(*state, step as f32 / LEVEL_STEPS as f32))
            .collect()
    })
    .collect()
}

/// Ikon: statusfärgad ring (grå = laddar, blå = redo, röd = inspelning,
/// gul = transkriberar) med en grön nivåstapel i mitten som följer
/// mikrofonens ljudnivå.
fn make_icon(state: AppState, level: f32) -> tray_icon::Icon {
    const S: i32 = 32;
    let (rr, rg, rb) = match state {
        AppState::Loading => (130u8, 130u8, 140u8),
        AppState::Idle => (90, 120, 200),
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
