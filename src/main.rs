#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod config;
mod model;
mod numbers;
mod sound;
mod transcribe;
mod update;
mod vad;
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
    /// Byt mikrofon; tom sträng = Windows standardenhet.
    SetMicrophone(String),
    /// Slå på/av kontinuerligt läge (auto-lyssning med VAD).
    SetContinuous(bool),
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
    /// En nyare version (taggen) finns på GitHub.
    UpdateAvailable(String),
    /// Kontinuerligt läge kunde inte aktiveras — återställ menybocken.
    ContinuousOff,
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
    let model_desc = if cfg.engine == "nemotron" {
        "nemotron-3.5-asr-streaming-0.6b".to_string()
    } else {
        format!("kb-whisper-{}", cfg.model)
    };
    log(&format!(
        "T-Whisper v{VERSION} startar — modell: {model_desc}, hotkey: {}",
        cfg.hotkey
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
    // Kontinuerligt läge: på/av-bock + egen aktiveringsknapp (F1–F12).
    let cont_submenu = Submenu::new("Kontinuerligt läge", true);
    let continuous_item =
        CheckMenuItem::new("Aktiverat (auto-lyssning)", true, cfg.continuous, None);
    cont_submenu.append(&continuous_item)?;
    let vad_key_submenu = Submenu::new("Aktiveringsknapp", true);
    let mut vad_key_items: Vec<(CheckMenuItem, String)> = Vec::new();
    for name in HOTKEY_CHOICES {
        let item = CheckMenuItem::new(name, true, name == cfg.continuous_hotkey, None);
        vad_key_submenu.append(&item)?;
        vad_key_items.push((item, name.to_string()));
    }
    cont_submenu.append(&vad_key_submenu)?;
    menu.append(&cont_submenu)?;
    // Mikrofonval: "Systemstandard" + enheterna som fanns vid appstart.
    let mic_submenu = Submenu::new("Mikrofon", true);
    let mut mic_items: Vec<(CheckMenuItem, String)> = Vec::new();
    {
        let default_item =
            CheckMenuItem::new("Systemstandard", true, cfg.microphone.is_empty(), None);
        mic_submenu.append(&default_item)?;
        mic_items.push((default_item, String::new()));
        for name in audio::input_device_names() {
            let item = CheckMenuItem::new(&name, true, cfg.microphone == name, None);
            mic_submenu.append(&item)?;
            mic_items.push((item, name));
        }
    }
    menu.append(&mic_submenu)?;
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
    let update_item = MenuItem::new("Sök efter uppdateringar", true, None);
    menu.append(&update_item)?;
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
    // Tangent som togglar kontinuerligt läge. Misslyckad registrering är
    // inte fatal — läget kan fortfarande styras via menyn.
    let mut vad_hotkey: HotKey = cfg.continuous_hotkey.parse().map_err(|e| {
        anyhow::anyhow!(
            "ogiltig continuous_hotkey '{}': {e:?}",
            cfg.continuous_hotkey
        )
    })?;
    if let Err(e) = hotkey_manager.register(vad_hotkey) {
        log(&format!(
            "kunde inte registrera {}: {e}",
            cfg.continuous_hotkey
        ));
    }

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

    // Tyst uppdateringskoll en stund efter start (kan stängas av i konfigen).
    if cfg.update_check {
        let proxy = event_loop.create_proxy();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(10));
            match update::fetch_latest_tag() {
                Ok(tag) if update::is_newer(&tag, VERSION) => {
                    log(&format!("ny version finns på GitHub: {tag}"));
                    let _ = proxy.send_event(UserEvent::UpdateAvailable(tag));
                }
                Ok(tag) => log(&format!(
                    "uppdateringskoll: {tag} är senaste — du kör v{VERSION}"
                )),
                Err(e) => log(&format!("uppdateringskollen misslyckades: {e}")),
            }
        });
    }
    let update_proxy = event_loop.create_proxy();

    log(&format!(
        "Redo. Håll {} och prata; släpp för att skriva texten. \
         {} togglar kontinuerligt läge.",
        cfg.hotkey, cfg.continuous_hotkey
    ));

    let mut cur_state = AppState::Loading;
    let mut cur_step = 0usize;
    let mut available_update: Option<String> = None;

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
            UserEvent::UpdateAvailable(tag) => {
                update_item.set_text(format!("Hämta {tag} — ny version!"));
                let _ = tray.set_tooltip(Some(format!(
                    "T-Whisper v{VERSION} — ny version ({tag}) finns på GitHub"
                )));
                available_update = Some(tag);
            }
            UserEvent::Hotkey(e) => {
                if e.id == vad_hotkey.id() {
                    if e.state == HotKeyState::Pressed {
                        cfg.continuous = !cfg.continuous;
                        continuous_item.set_checked(cfg.continuous);
                        let _ = cmd_tx.send(Cmd::SetContinuous(cfg.continuous));
                        if let Err(e) = cfg.save() {
                            log(&format!("kunde inte spara konfigurationen: {e}"));
                        }
                        play_toggle_sound(&cfg, &sound_volume, cfg.continuous);
                    }
                } else if e.id == hotkey.id() && !cfg.continuous {
                    // PTT-knappen ignoreras medan kontinuerligt läge är på.
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
            UserEvent::ContinuousOff => {
                cfg.continuous = false;
                continuous_item.set_checked(false);
                if let Err(e) = cfg.save() {
                    log(&format!("kunde inte spara konfigurationen: {e}"));
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
                } else if e.id == continuous_item.id() {
                    cfg.continuous = !cfg.continuous;
                    continuous_item.set_checked(cfg.continuous);
                    let _ = cmd_tx.send(Cmd::SetContinuous(cfg.continuous));
                    if let Err(e) = cfg.save() {
                        log(&format!("kunde inte spara konfigurationen: {e}"));
                    }
                    play_toggle_sound(&cfg, &sound_volume, cfg.continuous);
                } else if e.id == update_item.id() {
                    if available_update.is_some() {
                        // Ny version känd: öppna nedladdningssidan direkt.
                        if let Err(err) = std::process::Command::new("explorer")
                            .arg(format!("{REPO_URL}/releases/latest"))
                            .current_dir(config::config_dir())
                            .spawn()
                        {
                            log(&format!("kunde inte öppna releases-sidan: {err}"));
                        }
                    } else {
                        // Manuell koll i egen tråd så att loopen inte blockeras.
                        let proxy = update_proxy.clone();
                        std::thread::spawn(move || match update::fetch_latest_tag() {
                            Ok(tag) if update::is_newer(&tag, VERSION) => {
                                let _ = proxy.send_event(UserEvent::UpdateAvailable(tag.clone()));
                                winutil::message_box(
                                    "T-Whisper – uppdatering",
                                    &format!(
                                        "Ny version finns: {tag} (du kör v{VERSION}).\n\n\
                                         Välj \"Hämta {tag}\" i menyn för att öppna \
                                         nedladdningssidan."
                                    ),
                                    winutil::MB_ICONINFORMATION,
                                );
                            }
                            Ok(_) => winutil::message_box(
                                "T-Whisper – uppdatering",
                                &format!("Du kör senaste versionen (v{VERSION})."),
                                winutil::MB_ICONINFORMATION,
                            ),
                            Err(e) => {
                                log(&format!("uppdateringskollen misslyckades: {e}"));
                                winutil::message_box(
                                    "T-Whisper – uppdatering",
                                    &format!("Kunde inte nå GitHub:\n{e}"),
                                    winutil::MB_ICONWARNING,
                                );
                            }
                        });
                    }
                } else if let Some((_, name)) =
                    mic_items.iter().find(|(item, _)| e.id == *item.id())
                {
                    cfg.microphone = name.clone();
                    let _ = cmd_tx.send(Cmd::SetMicrophone(name.clone()));
                    if let Err(e) = cfg.save() {
                        log(&format!("kunde inte spara konfigurationen: {e}"));
                    }
                    log(&format!(
                        "Mikrofon vald: {}",
                        if name.is_empty() {
                            "systemstandard"
                        } else {
                            name
                        }
                    ));
                } else if e.id == about_item.id() {
                    show_about(cfg.engine == "nemotron");
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
                    if *name == cfg.continuous_hotkey {
                        warn_key_taken(name);
                    } else {
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
                                        log(&format!(
                                            "Inspelningsknapp ändrad till {}",
                                            cfg.hotkey
                                        ));
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
                    }
                } else if let Some((_, name)) =
                    vad_key_items.iter().find(|(item, _)| e.id == *item.id())
                {
                    if *name == cfg.hotkey {
                        warn_key_taken(name);
                    } else {
                        match name.parse::<HotKey>() {
                            Ok(new_hotkey) => {
                                let _ = hotkey_manager.unregister(vad_hotkey);
                                match hotkey_manager.register(new_hotkey) {
                                    Ok(()) => {
                                        vad_hotkey = new_hotkey;
                                        cfg.continuous_hotkey = name.clone();
                                        if let Err(e) = cfg.save() {
                                            log(&format!("kunde inte spara konfigurationen: {e}"));
                                        }
                                        log(&format!(
                                            "Aktiveringsknapp för kontinuerligt läge ändrad till {}",
                                            cfg.continuous_hotkey
                                        ));
                                    }
                                    Err(e) => {
                                        log(&format!("kunde inte registrera {name}: {e}"));
                                        let _ = hotkey_manager.register(vad_hotkey);
                                    }
                                }
                            }
                            Err(e) => log(&format!("ogiltig tangent {name}: {e:?}")),
                        }
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
                for (item, name) in &vad_key_items {
                    item.set_checked(*name == cfg.continuous_hotkey);
                }
                for (item, vol) in &vol_items {
                    item.set_checked((cfg.sound_volume - vol).abs() < 0.05);
                }
                for (item, name) in &mic_items {
                    item.set_checked(*name == cfg.microphone);
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
    let nemotron = cfg.engine == "nemotron";
    let ensured = if nemotron {
        model::ensure_nemotron_model()
    } else {
        model::ensure_model(&cfg.model)
    };
    let model_path = match ensured {
        Ok(p) => p,
        Err(e) => {
            let name = if nemotron {
                "nemotron-3.5-asr-streaming-0.6b".to_string()
            } else {
                format!("kb-whisper-{}", cfg.model)
            };
            fail(
                "T-Whisper – modellfel",
                &format!(
                    "Kunde inte hämta modellen {name}:\n{e:#}\n\n\
                     Kontrollera internetanslutningen och starta om T-Whisper."
                ),
            );
            return;
        }
    };
    let mut mic_name = cfg.microphone.clone();
    let mut recorder = match audio::Recorder::new(mic_level.clone(), wanted_mic(&mic_name)) {
        Ok(r) => r,
        Err(e) => {
            fail(
                "T-Whisper – mikrofonfel",
                &format!("Kunde inte öppna mikrofonen:\n{e:#}"),
            );
            return;
        }
    };
    log(if nemotron {
        "Laddar nemotron-modellen…"
    } else {
        "Laddar whisper-modellen…"
    });
    let t0 = std::time::Instant::now();
    let loaded = if nemotron {
        transcribe::Transcriber::new_nemotron(&model_path, &cfg.language)
    } else {
        transcribe::Transcriber::new_whisper(
            model_path.to_str().expect("ogiltig modellsökväg"),
            &cfg.language,
        )
    };
    let mut transcriber = match loaded {
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

    // Förhistorik som behålls före upptäckt talstart (~1 s vid 16 kHz),
    // så att första stavelsen inte klipps bort.
    const PRE_ROLL_SAMPLES: usize = 16_000;
    // Tvångsklipp yttranden längre än så här (whisper arbetar i
    // 30-sekundersfönster).
    const MAX_UTTERANCE_MS: u32 = 30_000;

    let mut recording = false;
    let mut continuous_on = false;
    let mut vad: Option<vad::Vad> = None;
    let mut gate = vad::SpeechGate::new(cfg.vad_silence_ms.max(200), MAX_UTTERANCE_MS);
    let mut buf: Vec<f32> = Vec::new();

    // Läget kan vara på redan från konfigen.
    if cfg.continuous {
        continuous_on =
            activate_continuous(&cfg, &mut vad, &mut recorder, &mut gate, &mut buf, &proxy);
    }

    loop {
        // I kontinuerligt läge pollas mikrofonen var 150:e ms; annars
        // blockerar tråden tills ett kommando kommer (0 % CPU i vila).
        let cmd = if continuous_on {
            match rx.recv_timeout(std::time::Duration::from_millis(150)) {
                Ok(c) => Some(c),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => None,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match rx.recv() {
                Ok(c) => Some(c),
                Err(_) => break,
            }
        };

        let Some(cmd) = cmd else {
            // Tick: töm mikrofonbufferten och låt VAD-grinden avgöra.
            if recorder.failed() {
                log("mikrofonströmmen har fallit — bygger om mot aktuell enhet");
                match audio::Recorder::new(mic_level.clone(), wanted_mic(&mic_name)) {
                    Ok(r) => {
                        recorder = r;
                        recorder.start();
                        buf.clear();
                        gate.reset();
                    }
                    Err(e) => log(&format!("kunde inte öppna mikrofonen: {e}")),
                }
                continue;
            }
            let chunk = recorder.drain();
            let chunk_ms = if chunk.is_empty() {
                150
            } else {
                (chunk.len() / 16) as u32
            };
            // Silero behöver minst ett analysfönster (512 sampel).
            let speech =
                chunk.len() >= 512 && vad.as_mut().map(|v| v.is_speech(&chunk)).unwrap_or(false);
            buf.extend_from_slice(&chunk);
            match gate.push(speech, chunk_ms) {
                vad::GateEvent::None => {
                    // Under tystnad: behåll bara den senaste förhistoriken.
                    if !gate.speaking() && buf.len() > PRE_ROLL_SAMPLES {
                        let cut = buf.len() - PRE_ROLL_SAMPLES;
                        buf.drain(..cut);
                    }
                }
                vad::GateEvent::SpeechStarted => {
                    let _ = proxy.send_event(UserEvent::State(AppState::Recording));
                }
                vad::GateEvent::UtteranceEnded | vad::GateEvent::ForcedCut => {
                    let _ = proxy.send_event(UserEvent::State(AppState::Working));
                    let samples = std::mem::take(&mut buf);
                    handle_utterance(
                        &samples,
                        &mut transcriber,
                        &mut enigo,
                        &cfg,
                        &shift_enter,
                        &digits,
                    );
                    // Tillbaka till röd: läget lyssnar fortfarande.
                    let _ = proxy.send_event(UserEvent::State(AppState::Recording));
                }
            }
            continue;
        };

        match cmd {
            Cmd::SetContinuous(true) if !continuous_on => {
                // Avbryt ev. pågående PTT-inspelning först.
                if recording {
                    recording = false;
                    let _ = recorder.stop();
                }
                continuous_on =
                    activate_continuous(&cfg, &mut vad, &mut recorder, &mut gate, &mut buf, &proxy);
            }
            Cmd::SetContinuous(false) if continuous_on => {
                continuous_on = false;
                let _ = recorder.stop();
                buf.clear();
                gate.reset();
                let _ = proxy.send_event(UserEvent::State(AppState::Idle));
                log("kontinuerligt läge av");
            }
            Cmd::StartRecording if !recording && !continuous_on => {
                // Föll mikrofonströmmen (t.ex. headset avstängt) byggs
                // inspelaren om mot nuvarande standardenhet.
                if recorder.failed() {
                    log("mikrofonströmmen har fallit — bygger om mot aktuell enhet");
                    match audio::Recorder::new(mic_level.clone(), wanted_mic(&mic_name)) {
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
                handle_utterance(
                    &samples,
                    &mut transcriber,
                    &mut enigo,
                    &cfg,
                    &shift_enter,
                    &digits,
                );
                let _ = proxy.send_event(UserEvent::State(AppState::Idle));
            }
            Cmd::SetMicrophone(name) => {
                // Avbryt ev. pågående inspelning innan bytet.
                if recording {
                    recording = false;
                    let _ = recorder.stop();
                    let _ = proxy.send_event(UserEvent::State(AppState::Idle));
                }
                mic_name = name;
                match audio::Recorder::new(mic_level.clone(), wanted_mic(&mic_name)) {
                    Ok(r) => {
                        recorder = r;
                        // I kontinuerligt läge ska nya enheten lyssna direkt.
                        if continuous_on {
                            recorder.start();
                            buf.clear();
                            gate.reset();
                            let _ = proxy.send_event(UserEvent::State(AppState::Recording));
                        }
                    }
                    Err(e) => log(&format!("kunde inte öppna mikrofonen: {e}")),
                }
            }
            _ => {}
        }
    }
}

/// Laddar VAD-modellen (vid behov) och startar lyssningen. Returnerar
/// true om läget kunde aktiveras; annars loggas felet, en varning visas
/// och menybocken återställs via [`UserEvent::ContinuousOff`].
fn activate_continuous(
    cfg: &config::Config,
    vad: &mut Option<vad::Vad>,
    recorder: &mut audio::Recorder,
    gate: &mut vad::SpeechGate,
    buf: &mut Vec<f32>,
    proxy: &EventLoopProxy<UserEvent>,
) -> bool {
    if vad.is_none() {
        let loaded = model::ensure_vad_model().and_then(|p| {
            vad::Vad::new(
                p.to_str().context("ogiltig sökväg till VAD-modellen")?,
                cfg.vad_threshold,
            )
        });
        match loaded {
            Ok(v) => *vad = Some(v),
            Err(e) => {
                log(&format!("kunde inte aktivera kontinuerligt läge: {e:#}"));
                let _ = proxy.send_event(UserEvent::ContinuousOff);
                std::thread::spawn(move || {
                    winutil::message_box(
                        "T-Whisper – kontinuerligt läge",
                        &format!(
                            "Kunde inte aktivera kontinuerligt läge:\n{e:#}\n\n\
                             Kontrollera internetanslutningen och försök igen."
                        ),
                        winutil::MB_ICONWARNING,
                    );
                });
                return false;
            }
        }
    }
    gate.reset();
    buf.clear();
    recorder.start();
    log("kontinuerligt läge på — lyssnar");
    // Röd ring hela tiden läget är på, så att det syns att appen lyssnar.
    let _ = proxy.send_event(UserEvent::State(AppState::Recording));
    true
}

/// Transkriberar ett färdigt yttrande och skriver in texten vid markören.
/// Gemensam för push-to-talk och kontinuerligt läge. Yttranden kortare
/// än ~0,25 s ignoreras.
fn handle_utterance(
    samples: &[f32],
    transcriber: &mut transcribe::Transcriber,
    enigo: &mut Enigo,
    cfg: &config::Config,
    shift_enter: &AtomicBool,
    digits: &AtomicBool,
) {
    if samples.len() < 4_000 {
        return;
    }
    let t = std::time::Instant::now();
    match transcriber.transcribe(samples, digits.load(Ordering::Relaxed)) {
        Ok(text) if !text.is_empty() => {
            log(&format!("[{:.2} s] {text}", t.elapsed().as_secs_f32()));
            let out = if cfg.append_space {
                format!("{text} ")
            } else {
                text
            };
            if let Err(e) = insert_text(enigo, &out, cfg.paste) {
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

/// Klick (på) eller pling (av) när kontinuerligt läge togglas.
fn play_toggle_sound(cfg: &config::Config, sound_volume: &AtomicU32, on: bool) {
    let vol = f32::from_bits(sound_volume.load(Ordering::Relaxed));
    if cfg.sounds && vol > 0.0 {
        if on {
            sound::play(300.0, 60, START_SOUND_GAIN * vol);
        } else {
            sound::play(880.0, 140, STOP_SOUND_GAIN * vol);
        }
    }
}

/// Varnar när användaren försöker ge två funktioner samma tangent.
/// Egen tråd så att meddelanderutan inte blockerar event-loopen.
fn warn_key_taken(key: &str) {
    let key = key.to_string();
    std::thread::spawn(move || {
        winutil::message_box(
            "T-Whisper",
            &format!(
                "{key} används redan av den andra funktionen.\n\
                 Välj en annan tangent."
            ),
            winutil::MB_ICONWARNING,
        );
    });
}

/// Visar en Om-ruta med version, projektinfo och GitHub-länk.
/// Körs i egen tråd så att meddelanderutan inte blockerar event-loopen.
fn show_about(nemotron: bool) {
    std::thread::spawn(move || {
        let (engine, model) = if nemotron {
            let engine = if cfg!(feature = "cuda") {
                "ONNX Runtime (NVIDIA CUDA)"
            } else if cfg!(feature = "directml") {
                "ONNX Runtime (DirectML)"
            } else {
                "ONNX Runtime (CPU)"
            };
            (engine, "Nemotron 3.5 ASR (NVIDIA)")
        } else {
            let engine = if cfg!(feature = "cuda") {
                "whisper.cpp (NVIDIA CUDA)"
            } else if cfg!(feature = "vulkan") {
                "whisper.cpp (Vulkan)"
            } else {
                "whisper.cpp (CPU)"
            };
            (engine, "KB-Whisper (Kungliga biblioteket)")
        };
        let text = format!(
            "T-Whisper v{VERSION}\n\
             \n\
             Push-to-talk-diktering på svenska för Windows 11.\n\
             Håll inspelningsknappen, prata, släpp — texten skrivs\n\
             in vid markören. Allt körs lokalt, inget skickas till molnet.\n\
             \n\
             Motor: {engine}\n\
             Modell: {model}\n\
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

/// Tom mikrofonsträng i konfigen betyder Windows standardenhet.
fn wanted_mic(name: &str) -> Option<&str> {
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
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
