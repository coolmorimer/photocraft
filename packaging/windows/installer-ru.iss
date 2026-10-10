; Русская независимая сборка PhotoCraft. ISCC /DAppVersion=0.6.0-ru.1 installer-ru.iss
#ifndef AppVersion
  #define AppVersion "0.6.0-ru.1"
#endif
#ifndef BinDir
  #define BinDir "..\..\target\release"
#endif
[Setup]
AppId={{50C1CE35-F675-49B4-B0DF-5D1585D0732C}
AppName=PhotoCraft RU
AppVersion={#AppVersion}
AppPublisher=coolmorimer
AppPublisherURL=https://github.com/coolmorimer/photocraft
AppSupportURL=https://github.com/coolmorimer/photocraft/issues
AppUpdatesURL=https://github.com/coolmorimer/photocraft/releases
DefaultDirName={localappdata}\Programs\PhotoCraft RU
DefaultGroupName=PhotoCraft RU
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\..\dist\ru
OutputBaseFilename=PhotoCraft-RU-{#AppVersion}-windows-x64-setup
SetupIconFile=..\..\assets\app-icon\photocraft.ico
UninstallDisplayIcon={app}\photocraft.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
WizardResizable=yes
WizardSizePercent=115
CloseApplications=yes
LicenseFile=..\..\LICENSE-MIT
InfoBeforeFile=..\..\docs\ru\INSTALL.txt

[Languages]
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"

[Tasks]
Name: "desktopicon"; Description: "Создать ярлык на рабочем столе"; Flags: unchecked

[Files]
Source: "{#BinDir}\photocraft.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BinDir}\photocraft-cli.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE-MIT"; DestDir: "{app}"
Source: "..\..\LICENSE-APACHE"; DestDir: "{app}"
Source: "..\..\NOTICE"; DestDir: "{app}"
Source: "..\..\ATTRIBUTION.md"; DestDir: "{app}"
Source: "..\..\docs\ru\INSTALL.txt"; DestDir: "{app}"; DestName: "ПРОЧТИ-МЕНЯ.txt"

[Icons]
Name: "{autoprograms}\PhotoCraft RU"; Filename: "{app}\photocraft.exe"
Name: "{autodesktop}\PhotoCraft RU"; Filename: "{app}\photocraft.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\photocraft.exe"; Description: "Запустить PhotoCraft RU"; Flags: nowait postinstall skipifsilent
