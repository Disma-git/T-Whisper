# T-Whisper

Snabb, lätt push-to-talk-diktering för Windows 11 på **svenska**. Håll en tangent (standard **F9**), prata, släpp — texten skrivs in vid markören i vilket program du än står i. Allt körs lokalt på din dator, inget skickas till molnet.

Bygger på [whisper.cpp](https://github.com/ggml-org/whisper.cpp) och Kungliga bibliotekets [KB-Whisper](https://huggingface.co/KBLab/kb-whisper-small) — en Whisper-modell tränad på 50 000 timmar svenskt tal med ~47 % färre fel än OpenAI:s Whisper på svenska. Skrivet i Rust; med CUDA-bygget transkriberas ett normalt yttrande på ~0,2 s (RTX 5080).

## Funktioner

- **Push-to-talk**: håll F9 (konfigurerbart), släpp för att transkribera
- **Texten skrivs vid markören** via urklipp + Ctrl+V (gamla urklippet återställs)
- **Helt lokalt** — modellen laddas ner en gång från Hugging Face, sedan ingen nätverkstrafik
- **Systemfältsikon med mic-nivåmätare**: grön stapel följer mikrofonens ljudnivå i realtid; ringen visar status (blå = redo, röd = spelar in, gul = transkriberar)
- **Ljudfeedback**: dovt klick när inspelningen startar, pling när knappen släpps — volym ställbar
- **Kontrollpanel i systemfältsmenyn**:
  - *Inspelningsknapp* — byt till valfri F1–F12 direkt, sparas automatiskt
  - *Ljudvolym* — Av / Låg / Mellan / Hög, med provljud vid val
  - *Öppna konfigurationsfil* — för avancerade inställningar
- Modellen hålls laddad i (GPU-)minnet → ingen uppstartsfördröjning per yttrande

## Bygga

### Förutsättningar

| Verktyg | Installation |
|---|---|
| Rust (MSVC) | `winget install Rustlang.Rustup` |
| Visual Studio 2022 Build Tools (C++) | `winget install Microsoft.VisualStudio.2022.BuildTools` med workload *VCTools* |
| CMake | `winget install Kitware.CMake` |
| LLVM (libclang för bindgen) | `winget install LLVM.LLVM`, sätt `LIBCLANG_PATH=C:\Program Files\LLVM\bin` |
| CUDA Toolkit (endast GPU-bygge) | `winget install Nvidia.CUDA` |

### Kompilera

```powershell
cargo build --release                     # CPU
cargo build --release --features cuda     # NVIDIA-GPU (rekommenderas)
```

Binären hamnar i `target/release/t-whisper.exe`. För CUDA-bygget måste miljövariablerna `CUDA_PATH` och `CUDA_PATH_V13_3` (sätts av CUDA-installern) finnas i byggmiljön; slutanvändaren behöver bara en NVIDIA-drivrutin. Utan GPU faller appen tillbaka till CPU, men då tar varje yttrande flera sekunder (whisper paddar allt ljud till 30 s-fönster).

Tips: ligger projektet på en nätverksenhet, sätt `CARGO_TARGET_DIR` till en lokal katalog för snabbare byggen.

## Konfiguration

Skapas vid första start i `%APPDATA%\T-Whisper\config.toml`:

```toml
hotkey = "F9"          # t.ex. "F9" eller "ctrl+shift+KeyD"
model = "small"        # tiny | base | small | medium | large
language = "sv"
append_space = true    # blanksteg efter varje inskriven mening
paste = true           # urklipp+Ctrl+V (false = teckenvis inskrivning)
sounds = true          # ljudfeedback vid start/stopp
sound_volume = 0.5     # volym för feedbackljuden, 0.0–1.0
```

Modellen (GGML q5, ~170 MB för small) laddas ner automatiskt till `%APPDATA%\T-Whisper\models\` vid första start. Byt till `medium` eller `large` i konfigen för högre kvalitet — de hämtas också automatiskt.

## Kända begränsningar

- Program som körs med administratörsrättigheter tar inte emot simulerad tangentbordsinmatning om T-Whisper inte också körs förhöjt.
- Hotkey-tangenten "sväljs" globalt medan appen körs (F9 når inte andra program).
- Enstaka program hanterar inte Ctrl+V-inklistring; sätt då `paste = false` för teckenvis inskrivning.
