; Inno Setup script for the HP Scanjet 3770 Windows installer.
;
; Built by scripts/package.ps1 (locally and in the release workflow):
;   iscc /DAppVersion=0.1.0 /DSourceDir=..\..\target\x86_64-pc-windows-msvc\release /DOutputDir=..\..\dist packaging\windows\hp3770.iss
;
; The installer asks "Install for me only" (the default, no administrator rights: files in
; %LOCALAPPDATA%\Programs\HP3770, Start menu and uninstall entry in the user's profile and HKCU)
; or "Install for all users" (Program Files and HKLM, asks for administrator rights). From the
; command line, /CURRENTUSER or /ALLUSERS skip the question.
;
; No driver is installed: the app talks to the scanner through HP's own driver (usbscan.sys)
; when it is there, or through WinUSB.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceDir
  #define SourceDir "..\..\target\release"
#endif
#ifndef OutputDir
  #define OutputDir "..\..\dist"
#endif

[Setup]
; Never change AppId: it is how Windows recognises an installed copy to upgrade or remove it.
AppId={{6B0BD725-4FC5-40CE-A326-BBF50527870C}
AppName=HP Scanjet 3770
AppVersion={#AppVersion}
AppVerName=HP Scanjet 3770 {#AppVersion}
AppPublisher=Matteo Dal Molin
AppPublisherURL=https://github.com/MatteoDM/scanner
AppSupportURL=https://github.com/MatteoDM/scanner/issues
VersionInfoVersion={#AppVersion}
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog commandline
DefaultDirName={autopf}\HP3770
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
CloseApplications=yes
RestartApplications=no
UninstallDisplayIcon={app}\hp3770-app.ico
UninstallDisplayName=HP Scanjet 3770
SetupIconFile=..\icons\hp3770-app.ico
LicenseFile=..\..\LICENSE
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
OutputDir={#OutputDir}
OutputBaseFilename=hp3770-v{#AppVersion}-windows-x86_64-setup

[Languages]
Name: "italian"; MessagesFile: "compiler:Languages\Italian.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\hp3770-app.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\hp3770.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\icons\hp3770-app.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
; Written by `cargo about generate` (scripts/package.ps1) before ISCC runs.
Source: "..\..\target\package\THIRD_PARTY_LICENSES.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\HP Scanjet 3770"; Filename: "{app}\hp3770-app.exe"; IconFilename: "{app}\hp3770-app.ico"
Name: "{autodesktop}\HP Scanjet 3770"; Filename: "{app}\hp3770-app.exe"; IconFilename: "{app}\hp3770-app.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\hp3770-app.exe"; Description: "{cm:LaunchProgram,HP Scanjet 3770}"; Flags: nowait postinstall skipifsilent
