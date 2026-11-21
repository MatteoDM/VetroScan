; Inno Setup script for the VetroScan Windows installer (until v0.2: "HP Scanjet 3770").
;
; Built by scripts/package.ps1 (locally and in the release workflow):
;   iscc /DAppVersion=0.1.0 /DSourceDir=..\..\target\x86_64-pc-windows-msvc\release /DOutputDir=..\..\dist packaging\windows\vetroscan.iss
;
; The installer asks "Install for me only" (the default, no administrator rights: files in
; %LOCALAPPDATA%\Programs\VetroScan, Start menu and uninstall entry in the user's profile and HKCU)
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
AppName=VetroScan
AppVersion={#AppVersion}
AppVerName=VetroScan {#AppVersion}
AppPublisher=Matteo Dal Molin
AppPublisherURL=https://github.com/MatteoDM/VetroScan
AppSupportURL=https://github.com/MatteoDM/VetroScan/issues
VersionInfoVersion={#AppVersion}
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog commandline
; An update keeps the folder of the installed copy (HP3770 before v0.3).
DefaultDirName={autopf}\VetroScan
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
CloseApplications=yes
RestartApplications=no
UninstallDisplayIcon={app}\vetroscan.ico
UninstallDisplayName=VetroScan
SetupIconFile=..\icons\vetroscan.ico
LicenseFile=..\..\LICENSE
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
OutputDir={#OutputDir}
OutputBaseFilename=vetroscan-v{#AppVersion}-windows-x86_64-setup

[Languages]
Name: "italian"; MessagesFile: "compiler:Languages\Italian.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\vetroscan.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\hp3770.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\icons\vetroscan.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
; Written by `cargo about generate` (scripts/package.ps1) before ISCC runs.
Source: "..\..\target\package\THIRD_PARTY_LICENSES.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion

[InstallDelete]
; What versions before v0.3 left, under the old name.
Type: files; Name: "{app}\hp3770-app.exe"
Type: files; Name: "{app}\hp3770-app.ico"
Type: files; Name: "{autoprograms}\HP Scanjet 3770.lnk"
Type: files; Name: "{autodesktop}\HP Scanjet 3770.lnk"

[Icons]
Name: "{autoprograms}\VetroScan"; Filename: "{app}\vetroscan.exe"; IconFilename: "{app}\vetroscan.ico"
Name: "{autodesktop}\VetroScan"; Filename: "{app}\vetroscan.exe"; IconFilename: "{app}\vetroscan.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\vetroscan.exe"; Description: "{cm:LaunchProgram,VetroScan}"; Flags: nowait postinstall skipifsilent
