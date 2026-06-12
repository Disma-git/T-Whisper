# T-Whisper v0.4.0

[![Senaste release](https://img.shields.io/github/v/release/Disma-git/T-Whisper?label=release&cacheSeconds=3600)](https://github.com/Disma-git/T-Whisper/releases/latest)
[![Licens: MIT](https://img.shields.io/badge/licens-MIT-blue.svg)](LICENSE)

Snabb, lätt push-to-talk-diktering för Windows 11 på **svenska**. Håll en tangent (standard **F9**), prata, släpp — texten skrivs in vid markören i vilket program du än står i. Allt körs lokalt på din dator, inget skickas till molnet.

Bygger på [whisper.cpp](https://github.com/ggml-org/whisper.cpp) och Kungliga bibliotekets [KB-Whisper](https://huggingface.co/KBLab/kb-whisper-small) — en Whisper-modell tränad på 50 000 timmar svenskt tal med ~47 % färre fel än OpenAI:s Whisper på svenska. Skrivet i Rust; med CUDA-bygget transkriberas ett normalt yttrande på ~0,2 s (RTX 5080).

## Funktioner

- **Push-to-talk**: håll F9 (konfigurerbart), släpp för att transkribera
- **Texten skrivs vid markören** via urklipp + Ctrl+V (gamla urklippet återställs)
- **Shift+Enter efter varje diktering** (valbart) — varje mening på ny rad, perfekt för flödesdiktering i Anteckningar med en enda tangent
- **Tal som siffror** (valbart) — "tjugofem" blir 25, "femtusen" blir 5000, även sammansatta tal som "etthundratjugotre" → 123
- **Helt lokalt** — modellen laddas ner en gång från Hugging Face, sedan ingen nätverkstrafik
- **Systemfältsikon med mic-nivåmätare**: grön stapel följer mikrofonens ljudnivå i realtid; ringen visar status (blå = redo, röd = spelar in, gul = transkriberar)
- **Ljudfeedback**: dovt klick när inspelningen startar, pling när knappen släpps — volym ställbar
- **Kontrollpanel i systemfältsmenyn**:
  - *Inspelningsknapp* — byt till valfri F1–F12 direkt, sparas automatiskt
  - *Mikrofon* — välj ljudenhet manuellt, eller följ Windows standard
  - *Ljudvolym* — Av / Låg / Mellan / Hög, med provljud vid val
  - *Shift+Enter efter varje diktering* — slå av/på
  - *Skriv tal som siffror* — slå av/på
  - *Öppna konfigurationsfil* — för avancerade inställningar
  - *Om T-Whisper* — version, projektinfo och GitHub-länk
  - *Sök efter uppdateringar* — manuell koll; appen kollar även tyst vid start och byter menytext när ny version finns
- Modellen hålls laddad i (GPU-)minnet → ingen uppstartsfördröjning per yttrande

## Installera (färdigt paket)

Ladda ner senaste **T-Whisper-Setup-x.y.z.exe** från [Releases](https://github.com/Disma-git/T-Whisper/releases) och kör den. Installern:

- kräver inga administratörsrättigheter (installeras per användare)
- innehåller alla beroenden (CUDA- och VC++-runtime-DLL:er)
- erbjuder autostart med Windows och skrivbordsgenväg
- avinstalleras via Inställningar → Appar

Windows SmartScreen kan varna eftersom installern inte är kodsignerad — klicka "Mer info" → "Kör ändå". Vid första start laddas modellen (~170 MB) ner automatiskt.

**Krav:** Windows 11 (x64). NVIDIA-GPU med aktuell drivrutin rekommenderas starkt — utan GPU körs transkriberingen på CPU och tar flera sekunder per yttrande.

## Bygga från källkod

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

Binären hamnar i `target/release/t-whisper.exe`. För CUDA-bygget måste miljövariablerna `CUDA_PATH` och `CUDA_PATH_V13_3` (sätts av CUDA-installern) finnas i byggmiljön. Kör du exe:n utanför installern: lägg CUDA-DLL:erna (`cudart64_13.dll`, `cublas64_13.dll`, `cublasLt64_13.dll`) bredvid exe:n, annars hittas de inte på system utan CUDA Toolkit i PATH.

Tips: ligger projektet på en nätverksenhet, sätt `CARGO_TARGET_DIR` till en lokal katalog för snabbare byggen.

### Bygga installern

Installern byggs från [installer/t-whisper.iss](installer/t-whisper.iss) med [Inno Setup](https://jrsoftware.org/isinfo.php); fullständiga instruktioner finns i skriptets huvud.

## Konfiguration

Skapas vid första start i `%APPDATA%\T-Whisper\config.toml` — filen är självdokumenterande med förklarande kommentarer för varje inställning, inklusive en tabell över tillgängliga modeller:

```toml
hotkey = "F9"          # t.ex. "F9" eller "ctrl+shift+KeyD"
model = "small"        # tiny | base | small | medium | large (se tabell i filen)
language = "sv"
microphone = ""        # enhetens namn; tom = Windows standardenhet
append_space = true    # blanksteg efter varje inskriven mening
paste = true           # urklipp+Ctrl+V (false = teckenvis inskrivning)
shift_enter = false    # ny rad efter varje diktering
digits = false         # skriv tal som siffror ("tjugofem" -> 25)
sounds = true          # ljudfeedback vid start/stopp
sound_volume = 0.5     # volym för feedbackljuden, 0.0–1.0
update_check = true    # kolla efter ny version på GitHub vid start
```

Modellen (GGML q5, ~170 MB för small) laddas ner automatiskt till `%APPDATA%\T-Whisper\models\`. Byt till `medium` eller `large` i konfigen för högre kvalitet — de hämtas också automatiskt.

## Versionshistorik

| Version | Datum | Nyheter |
|---|---|---|
| **0.4.0** | 2026-06-12 | Mikrofonval i systemfältsmenyn (valet sparas; strömmen byggs om direkt); uppdateringskoll mot GitHub Releases — tyst vid start plus menyposten "Sök efter uppdateringar" |
| **0.3.0** | 2026-06-10 | Stort optimerings- och robusthetspaket: whisper-state återanvänds + flash attention (snabbare svar); händelsestyrd event-loop (~0 % CPU i vila); lås-fri ljudbuffert; 16 kHz väljs direkt när mikrofonen stödjer det; single-instance-skydd; felmeddelanden i dialogrutor + loggfil (%APPDATA%\T-Whisper\log.txt); mikrofonströmmen byggs om automatiskt vid enhetsbyte; trasig konfig stoppar inte starten; trayikonen visas direkt vid start; app-ikon i Utforskaren; LICENSE/NOTICE; CI på GitHub Actions |
| **0.2.2** | 2026-06-10 | Valbart läge "Skriv tal som siffror": svenska talord konverteras till siffror (även sammansatta som "tjugofemtusen" → 25 000) och modellen styrs mot sifferskrivning |
| **0.2.1** | 2026-06-10 | Valbar Shift+Enter efter varje diktering (flödesdiktering); självdokumenterande konfigfil med modelltabell; Inno Setup-installer med alla beroenden buntade |
| **0.2.0** | 2026-06-10 | Textinmatning via urklipp+Ctrl+V (tappar inte längre tecken); mic-nivåmätare i systemfältsikonen; ljudfeedback med ställbar volym; kontrollpanelsmeny (inspelningsknapp F1–F12, ljudvolym); Om-ruta med version och GitHub-länk; robustare öppning av konfigfilen |
| **0.1.0** | 2026-06-10 | Första versionen: push-to-talk med F9, KB-Whisper small på GPU (CUDA) med CPU-fallback, svensk transkribering, systemfältsikon, automatisk modellnedladdning |

## Felsökning

Appen loggar till `%APPDATA%\T-Whisper\log.txt` (roteras vid ~1 MB). Allvarliga fel vid start visas dessutom i en dialogruta. Startar du appen två gånger berättar den andra instansen att T-Whisper redan kör.

## Kända begränsningar

- Program som körs med administratörsrättigheter tar inte emot simulerad tangentbordsinmatning om T-Whisper inte också körs förhöjt.
- Hotkey-tangenten "sväljs" globalt medan appen körs (F9 når inte andra program).
- Enstaka program hanterar inte Ctrl+V-inklistring; sätt då `paste = false` för teckenvis inskrivning.
- Installern är inte kodsignerad, vilket ger en SmartScreen-varning vid första körning.
