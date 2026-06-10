# T-Whisper

Snabb, lätt push-to-talk-diktering för Windows 11 på **svenska**. Håll en tangent (standard **F9**), prata, släpp — texten skrivs in vid markören i vilket program du än står i. Allt körs lokalt på din dator, inget skickas till molnet.

Bygger på [whisper.cpp](https://github.com/ggml-org/whisper.cpp) och Kungliga bibliotekets [KB-Whisper](https://huggingface.co/KBLab/kb-whisper-small) — en Whisper-modell tränad på 50 000 timmar svenskt tal med ~47 % färre fel än OpenAI:s Whisper på svenska.

## Funktioner

- **Push-to-talk**: håll F9 (konfigurerbart), släpp för att transkribera
- **Texten skrivs vid markören** via SendInput — fungerar i de flesta program
- **Helt lokalt** — modellen laddas ner en gång från Hugging Face, sedan ingen nätverkstrafik
- **Systemfältsikon** med status: blå = redo, röd = spelar in, gul = transkriberar
- Modellen hålls laddad i minnet → ingen uppstartsfördröjning per yttrande

## Bygga

Kräver Rust (MSVC-toolchain) och CMake.

```powershell
cargo build --release                     # CPU
cargo build --release --features cuda     # NVIDIA-GPU (kräver CUDA Toolkit vid bygge)
```

Binären hamnar i `target/release/t-whisper.exe`.

## Konfiguration

Skapas vid första start i `%APPDATA%\T-Whisper\config.toml`:

```toml
hotkey = "F9"          # t.ex. "F9" eller "ctrl+shift+KeyD"
model = "small"        # tiny | base | small | medium | large
language = "sv"
append_space = true    # blanksteg efter varje inskriven mening
```

Modellen laddas ner automatiskt till `%APPDATA%\T-Whisper\models\` vid första start (~500 MB för small).

## Kända begränsningar

- Program som körs med administratörsrättigheter tar inte emot simulerad tangentbordsinmatning om T-Whisper inte också körs förhöjt.
- Hotkey-tangenten "sväljs" globalt medan appen körs (F9 når inte andra program).
