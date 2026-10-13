; Trinh cai dat EVDict (Inno Setup 6). Build: scripts\package.ps1
; Cai cho rieng nguoi dung hien tai (khong can quyen admin), vao %LOCALAPPDATA%\Programs\EVDict.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{8F3A2C1E-5B7D-4E9A-9C21-6A3F0E7B4D12}
AppName=EVDict
AppVersion={#AppVersion}
AppVerName=EVDict {#AppVersion}
AppPublisher=EVDict
DefaultDirName={localappdata}\Programs\EVDict
DisableDirPage=auto
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputBaseFilename=EVDict-Setup-{#AppVersion}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\evdict.exe
UninstallDisplayName=EVDict - English-Vietnamese dictionary
; Dong ban dang chay truoc khi ghi de (app xu ly WM_ENDSESSION de thoat gon).
CloseApplications=force
RestartApplications=no

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "autostart"; Description: "Start EVDict when Windows starts"; GroupDescription: "Options:"

[Files]
Source: "..\dist\evdict.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\dist\evdict.dat"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\EVDict"; Filename: "{app}\evdict.exe"; Comment: "English-Vietnamese dictionary (Ctrl+Alt+D)"

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "EVDict"; ValueData: """{app}\evdict.exe"""; Tasks: autostart

[Run]
Filename: "{app}\evdict.exe"; Description: "Launch EVDict"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM evdict.exe"; Flags: runhidden; RunOnceId: "StopEVDict"

[Code]
// Go muc "khoi dong cung Windows" ke ca khi nguoi dung bat no tu menu cua app.
// Tu dien nguoi dung da them (%APPDATA%\EVDict) duoc giu lai.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
    RegDeleteValue(HKEY_CURRENT_USER, 'Software\Microsoft\Windows\CurrentVersion\Run', 'EVDict');
end;
