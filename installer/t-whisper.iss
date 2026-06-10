; ==============================================================
;  T-Whisper - Inno Setup-skript
; ==============================================================
;  Bygg installern så här (PowerShell):
;
;    1. cargo build --release --features cuda
;    2. Skapa en staging-mapp med:
;         t-whisper.exe
;         cudart64_13.dll, cublas64_13.dll, cublasLt64_13.dll   (CUDA bin\x64)
;         msvcp140.dll, vcruntime140.dll, vcruntime140_1.dll    (VC++ redist x64)
;    3. & "C:\Program Files (x86)\Inno Setup 6\ISCC.exe" `
;         /DAppVersion=<version> /DStageDir=<staging-mapp> t-whisper.iss
;
;  Resultatet blir Output\T-Whisper-Setup-<version>.exe

#ifndef AppVersion
  #define AppVersion "0.2.0"
#endif
#ifndef StageDir
  #define StageDir "stage"
#endif

[Setup]
AppId={{7F4B96D1-3A52-4E0C-9B7D-T0WHISPER001}
AppName=T-Whisper
AppVersion={#AppVersion}
AppPublisher=Disma-git
AppPublisherURL=https://github.com/Disma-git/T-Whisper
AppSupportURL=https://github.com/Disma-git/T-Whisper/issues
AppUpdatesURL=https://github.com/Disma-git/T-Whisper/releases
DefaultDirName={autopf}\T-Whisper
DisableProgramGroupPage=yes
; Per-användare-installation: ingen UAC-prompt, hamnar i
; %LOCALAPPDATA%\Programs\T-Whisper
PrivilegesRequired=lowest
OutputBaseFilename=T-Whisper-Setup-{#AppVersion}
Compression=lzma2/max
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayIcon={app}\t-whisper.exe
WizardStyle=modern

[Tasks]
Name: "autostart"; Description: "Starta T-Whisper automatiskt när Windows startar"
Name: "desktopicon"; Description: "Skapa en genväg på skrivbordet"; Flags: unchecked

[Files]
Source: "{#StageDir}\t-whisper.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\*.dll"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\T-Whisper"; Filename: "{app}\t-whisper.exe"
Name: "{autodesktop}\T-Whisper"; Filename: "{app}\t-whisper.exe"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; \
  ValueType: string; ValueName: "T-Whisper"; ValueData: """{app}\t-whisper.exe"""; \
  Flags: uninsdeletevalue; Tasks: autostart

[Run]
Filename: "{app}\t-whisper.exe"; Description: "Starta T-Whisper nu"; \
  Flags: nowait postinstall skipifsilent

[UninstallRun]
; Stäng appen om den kör innan filerna tas bort.
Filename: "taskkill"; Parameters: "/f /im t-whisper.exe"; Flags: runhidden; \
  RunOnceId: "StoppaTWhisper"
