#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod config;
mod model;
mod transcribe;

use anyhow::{Context, Result};
use enigo::{Enigo, Keyboard, Settings};
use global_hotkey::{
    hotkey::HotKey, GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use tao::event::Event;
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::TrayIconBuilder;

enum Cmd {
    StartRecording,
    StopAndTranscribe,
}

#[derive(Debug, Clone, Copy)]
enum AppState {
    Idle,
    Recording,
    Working,
}

fn main() -> Result<()> {
    let cfg = config::Config::load().context("kunde inte läsa konfigurationen")?;
    eprintln!(
        "T-Whisper startar — modell: kb-whisper-{}, hotkey: {}",
        cfg.model, cfg.hotkey
    );
    let model_path = model::ensure_model(&cfg.model)?;

    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<Cmd>();

    let event_loop = EventLoopBuilder::<AppState>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    // Arbetstråden äger mikrofon, whisper-modell och tangentbordsutmatning.
    {
        let cfg = cfg.clone();
        std::thread::spawn(move || controller(model_path, cfg, cmd_rx, proxy));
    }

    // Systemfältsikon med meny.
    let menu = Menu::new();
    let quit_item = MenuItem::new("Avsluta", true, None);
    menu.append(&quit_item)?;
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(format!("T-Whisper — håll {} och prata", cfg.hotkey))
        .with_icon(make_icon(90, 120, 200))
        .build()?;

    // Global push-to-talk-tangent.
    let hotkey: HotKey = cfg
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

    event_loop.run(move |event, _, control_flow| {
        // Kort poll-intervall: global-hotkeys kanal väcker inte tao-loopen av
        // sig själv, så vi tittar med jämna mellanrum (försumbar CPU-kostnad).
        *control_flow = ControlFlow::WaitUntil(
            std::time::Instant::now() + std::time::Duration::from_millis(30),
        );

        if let Event::UserEvent(state) = &event {
            let icon = match state {
                AppState::Idle => make_icon(90, 120, 200),
                AppState::Recording => make_icon(220, 60, 60),
                AppState::Working => make_icon(230, 170, 50),
            };
            let _ = tray.set_icon(Some(icon));
        }

        while let Ok(e) = hotkey_rx.try_recv() {
            eprintln!("hotkey-händelse: id={} state={:?}", e.id, e.state);
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
            }
        }
    });
}

fn controller(
    model_path: PathBuf,
    cfg: config::Config,
    rx: Receiver<Cmd>,
    proxy: EventLoopProxy<AppState>,
) {
    let recorder = match audio::Recorder::new() {
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
                let _ = proxy.send_event(AppState::Recording);
            }
            Cmd::StopAndTranscribe if recording => {
                recording = false;
                let _ = proxy.send_event(AppState::Working);
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
                            if let Err(e) = enigo.text(&out) {
                                eprintln!("kunde inte skriva texten: {e}");
                            }
                        }
                        Ok(_) => eprintln!("(inget tal uppfattades)"),
                        Err(e) => eprintln!("transkriberingsfel: {e}"),
                    }
                }
                let _ = proxy.send_event(AppState::Idle);
            }
            _ => {}
        }
    }
}

/// Enkel rund ikon i given färg, genererad i minnet (ingen asset-fil behövs).
fn make_icon(r: u8, g: u8, b: u8) -> tray_icon::Icon {
    const S: u32 = 32;
    let mut rgba = vec![0u8; (S * S * 4) as usize];
    let c = (S as f32 - 1.0) / 2.0;
    for y in 0..S {
        for x in 0..S {
            let dx = x as f32 - c;
            let dy = y as f32 - c;
            if (dx * dx + dy * dy).sqrt() <= c - 1.0 {
                let i = ((y * S + x) * 4) as usize;
                rgba[i] = r;
                rgba[i + 1] = g;
                rgba[i + 2] = b;
                rgba[i + 3] = 255;
            }
        }
    }
    tray_icon::Icon::from_rgba(rgba, S, S).expect("ikon")
}
