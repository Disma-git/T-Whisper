# Tredjepartskomponenter

T-Whisper (MIT-licens) bygger på och distribuerar följande komponenter:

| Komponent | Licens | Källa |
|---|---|---|
| **KB-Whisper** (taligenkänningsmodell) | Apache License 2.0 | KBLab, Kungliga biblioteket — https://huggingface.co/KBLab |
| **whisper.cpp** (inferensmotor, via whisper-rs) | MIT | https://github.com/ggml-org/whisper.cpp |
| **Whisper** (modellarkitektur) | MIT | OpenAI — https://github.com/openai/whisper |
| **NVIDIA CUDA-runtime** (cudart, cuBLAS — medföljer installern) | NVIDIA EULA (omdistribuerbara runtime-bibliotek) | https://docs.nvidia.com/cuda/eula/ |
| **Microsoft Visual C++ Runtime** (medföljer installern) | Microsoft Visual Studio-distributionsvillkor | https://visualstudio.microsoft.com |
| Rust-crates (cpal, tao, tray-icon, global-hotkey, enigo, arboard, rtrb, reqwest, serde, toml, anyhow, dirs, whisper-rs) | MIT och/eller Apache 2.0 | https://crates.io |

KB-Whisper är tränad av KBLab vid Kungliga biblioteket på cirka 50 000 timmar
svenskt tal. Modellen laddas ner av användaren från Hugging Face vid första
start och modifieras inte av T-Whisper.
