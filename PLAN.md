# T-Whisper — Push-to-talk-transkribering för Windows 11 (svenska)

## Kontext

Projektet (X:\T-Whisper, GitHub: Disma-git/T-Whisper, tomt repo) ska bli en snabb och lätt
Windows 11-app för tal-till-text på svenska med **push-to-talk**: håll en global snabbtangent,
prata, släpp — texten skrivs in vid markören i vilket program som helst. Allt körs lokalt,
ingen molntjänst.

## Beslut (bekräftade av användaren 2026-06-10)

| Område | Val |
|---|---|
| Hårdvara | NVIDIA-GPU finns → GPU-acceleration, CPU-fallback |
| Output | Text skrivs vid markören (diktering) |
| Standardmodell | **KBLab/kb-whisper-small** (~500 MB, bättre svenska än OpenAI large-v3) |
| Språk | **Rust** |

## Research-underlag

- **KB-Whisper** (Kungliga biblioteket): Whisper finjusterad på 50 000 h svenskt tal,
  ~47 % färre fel än OpenAI Whisper på svenska. Finns i tiny/base/small/medium/large
  och i **GGML-format för whisper.cpp** direkt på Hugging Face:
  https://huggingface.co/KBLab/kb-whisper-small (filer `ggml-model-q5.bin` m.fl.)
- **Motor: whisper.cpp** via Rust-bindningen **whisper-rs** — modellen hålls laddad i minnet
  (ingen laddningskostnad per yttrande), byggs med **CUDA-feature** för NVIDIA-GPU och
  faller tillbaka till CPU. Lättvikt: en enda exe, ingen Python/CUDA-runtime-install krävs
  utöver NVIDIA-drivrutin.
- Referensprojekt med samma arkitektur: **Handy** (https://github.com/cjpais/Handy, Rust +
  whisper.cpp, öppen källkod) och GigaWhisper (Tauri/Rust).
- faster-whisper är snabbare för batch på GPU men kräver Python-stack — fel avvägning för
  en lätt alltid-på-app.

## Arkitektur

Ren Rust-binär (ingen webview/Electron) som ligger i systemfältet:

```
[Global hotkey] → [Mikrofoninspelning 16 kHz mono] → [whisper.cpp (kb-whisper-small, GPU)]
      → [Textinmatning vid markören] + [tray-ikon med status/inställningar]
```

### Kärnkomponenter (crates)

| Komponent | Crate | Roll |
|---|---|---|
| Whisper-inferens | `whisper-rs` (feature `cuda`) | Binder whisper.cpp; modell laddad persistent |
| Ljudinspelning | `cpal` | WASAPI-mikrofon, resampla till 16 kHz mono f32 |
| Global hotkey | `global-hotkey` | Push-to-talk: keydown = spela in, keyup = transkribera |
| Textinmatning | `enigo` | Skriver text vid markören (SendInput, unicode — klarar åäö) |
| Tray-ikon | `tray-icon` | Status (lyssnar/transkriberar), meny: inställningar, avsluta |
| Modellnedladdning | `reqwest` | Laddar ner GGML-modellen från Hugging Face vid första start |
| Konfig | `serde` + TOML | Hotkey, modellval, språk, i %APPDATA%\T-Whisper |

### Flöde push-to-talk

1. Appen startar, laddar `ggml kb-whisper-small (q5)` i VRAM (laddas ner vid första start till `%APPDATA%\T-Whisper\models`).
2. Användaren håller hotkey (standard: t.ex. `Ctrl+Win`): inspelning startar, tray-ikonen indikerar.
3. Släpp: ljudbufferten skickas till whisper med `language="sv"`; på GPU tar small-modellen typiskt < 1 s för ett normalt yttrande.
4. Resultatet trimmas (whisper-artefakter som `[MUSIK]`, inledande blanksteg) och skrivs vid markören via SendInput.

## Implementationssteg

1. **Init**: `cargo init`, .gitignore (target/, models/), README, första commit + push.
2. **Inferens-PoC**: whisper-rs + nedladdning av kb-whisper-small GGML; transkribera en WAV-fil på svenska. Verifiera CUDA-bygget (kräver CUDA Toolkit + MSVC vid byggtillfället; slutanvändare behöver bara NVIDIA-drivrutin).
3. **Ljudinspelning**: cpal-inspelning till ringbuffert, resampling till 16 kHz mono.
4. **Push-to-talk-loop**: global-hotkey keydown/keyup kopplat till inspelning → transkribering.
5. **Textinmatning**: enigo skriver resultatet vid markören; verifiera åäö i Notepad/VS Code.
6. **Tray + konfig**: tray-icon med statusikoner, TOML-konfig, modellval (small/medium/large), valbar hotkey.
7. **Polering**: CPU-fallback om CUDA saknas, autostart (valfritt), release-bygge `--release` + LTO, GitHub Actions-bygge.

## Verifiering

- Enhetsnivå: transkribera en känd svensk test-WAV och jämför mot förväntad text.
- End-to-end: starta appen, håll hotkey, säg en svensk mening med åäö och egennamn, släpp — texten ska dyka upp vid markören i Notepad inom ~1–2 s.
- Mät: tid från keyup till text (mål < 1,5 s på GPU), RAM/VRAM-användning (mål < 1 GB VRAM med small q5), CPU i vila (~0 %).

## Risker / noteringar

- CUDA-bygge av whisper.cpp på Windows kräver CUDA Toolkit installerat på byggmaskinen; alternativ är Vulkan-featuren (fungerar på alla GPU:er, något långsammare).
- Vissa appar med förhöjda rättigheter tar inte emot SendInput — känd begränsning, dokumenteras.
- Antivirus kan flagga globala hotkeys + SendInput; signering/undantag kan behövas senare.
