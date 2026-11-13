#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{29BC2D42-1EEB-4D88-A7EA-95A9D4B6FBCC}
AppName=Snowfall
AppVersion={#AppVersion}
AppPublisher=Tomjerri1
AppPublisherURL=https://github.com/Tomjerri1/Snowfall
AppSupportURL=https://github.com/Tomjerri1/Snowfall/issues
DefaultDirName={autopf}\Snowfall
DefaultGroupName=Snowfall
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
LicenseFile=..\LICENSE
SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\snowfall.exe
OutputBaseFilename=snowfall-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; Flags: unchecked
Name: "autostart"; Description: "Start Snowfall when Windows starts"; Flags: unchecked

[Files]
Source: "..\target\release\snowfall.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Snowfall"; Filename: "{app}\snowfall.exe"
Name: "{autodesktop}\Snowfall"; Filename: "{app}\snowfall.exe"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Snowfall"; ValueData: """{app}\snowfall.exe"""; Flags: uninsdeletevalue; Tasks: autostart

[Run]
Filename: "{app}\snowfall.exe"; Description: "Launch Snowfall"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM snowfall.exe"; Flags: runhidden; RunOnceId: "KillSnowfall"

[Code]
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  ResultCode: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM snowfall.exe', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Result := '';
end;
