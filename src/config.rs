use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Push-to-talk-tangent, t.ex. "F9" eller "ctrl+shift+KeyD"
    pub hotkey: String,
    /// KB-Whisper-modell: tiny | base | small | medium | large
    pub model: String,
    /// Språkkod för transkribering
    pub language: String,
    /// Lägg till ett blanksteg efter inskriven text
    pub append_space: bool,
    /// Skriv in texten via urklipp + Ctrl+V (robust) i stället för teckenvis
    pub paste: bool,
    /// Skicka Shift+Enter efter varje diktering (ny rad i t.ex. Notepad)
    pub shift_enter: bool,
    /// Ljudfeedback: dovt klick vid inspelningsstart, pling vid släpp
    pub sounds: bool,
    /// Volym för feedbackljuden, 0.0–1.0
    pub sound_volume: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: "F9".into(),
            model: "small".into(),
            language: "sv".into(),
            append_space: true,
            paste: true,
            shift_enter: false,
            sounds: true,
            sound_volume: 0.5,
        }
    }
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .expect("kunde inte hitta konfigurationskatalogen")
        .join("T-Whisper")
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = config_dir().join("config.toml");
        if path.exists() {
            let raw = std::fs::read_to_string(&path)?;
            let cfg: Self = toml::from_str(&raw)?;
            // Uppgradera äldre, okommenterade filer till dokumenterat format.
            if !raw.starts_with("# ==") {
                cfg.save()?;
            }
            Ok(cfg)
        } else {
            let cfg = Self::default();
            cfg.save()?;
            Ok(cfg)
        }
    }

    /// Sparar konfigurationen som en kommenterad, självdokumenterande fil.
    /// Kommentarer (rader med #) läses aldrig av programmet.
    pub fn save(&self) -> Result<()> {
        std::fs::create_dir_all(config_dir())?;
        std::fs::write(config_dir().join("config.toml"), self.render())?;
        Ok(())
    }

    fn render(&self) -> String {
        format!(
            r#"# ==============================================================
#  T-Whisper – konfiguration
# ==============================================================
#  Den här filen läses när programmet startar. Rader som börjar
#  med # är kommentarer och påverkar ingenting.
#
#  Öppna filen via systemfältsikonen -> "Öppna konfigurationsfil".
#  Starta om T-Whisper efter ändringar här. Inspelningsknapp och
#  ljudvolym kan även ändras direkt i systemfältsmenyn.

# --------------------------------------------------------------
#  Push-to-talk-tangent: håll för att spela in, släpp för att
#  skriva texten. Exempel: "F9", "F12", "ctrl+shift+KeyD",
#  "alt+Space". Tangenten reserveras globalt medan appen kör.
hotkey = "{hotkey}"

# --------------------------------------------------------------
#  KB-Whisper-modell. Byter du modell laddas den ner automatiskt
#  från Hugging Face vid nästa start och sparas i
#  %APPDATA%\T-Whisper\models. Större modell = bättre kvalitet,
#  men långsammare och mer minne:
#
#    Modell    Nedladdning   Beskrivning
#    tiny      ~30 MB        snabbast, enklast – korta kommandon
#    base      ~60 MB        snabb, enkel diktering
#    small     ~170 MB       rekommenderad balans (standard)
#    medium    ~540 MB       bättre kvalitet, långsammare på CPU
#    large     ~1.1 GB       bäst kvalitet, kräver kraftfull GPU
model = "{model}"

#  Talspråk (ISO-kod): "sv" = svenska, "en" = engelska, osv.
language = "{language}"

# --------------------------------------------------------------
#  true = lägg till ett blanksteg efter varje inskriven mening,
#  praktiskt när man dikterar flera meningar i rad.
append_space = {append_space}

#  true  = texten skrivs in via urklipp + Ctrl+V (rekommenderas;
#          ditt gamla urklipp återställs efteråt)
#  false = texten skrivs tecken för tecken (långsammare, kan tappa
#          tecken i vissa program, men funkar där Ctrl+V inte gör det)
paste = {paste}

#  true = skicka Shift+Enter efter varje diktering, så att varje
#  mening hamnar på en ny rad (praktiskt i t.ex. Anteckningar).
#  Kan även slås av/på i systemfältsmenyn.
shift_enter = {shift_enter}

# --------------------------------------------------------------
#  Ljudfeedback: dovt klick när inspelningen startar, pling när
#  knappen släpps.
sounds = {sounds}

#  Volym för feedbackljuden, 0.0 (tyst) till 1.0 (max).
#  Kan även ändras i systemfältsmenyn under "Ljudvolym".
sound_volume = {sound_volume}
"#,
            hotkey = self.hotkey,
            model = self.model,
            language = self.language,
            append_space = self.append_space,
            paste = self.paste,
            shift_enter = self.shift_enter,
            sound_volume = self.sound_volume,
            sounds = self.sounds,
        )
    }
}
