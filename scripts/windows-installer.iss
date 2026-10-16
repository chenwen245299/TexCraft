; Inno Setup script for the Windows installer, TexCraft-<version>-windows-x64-setup.exe.
; release.yml compiles it after copying the built files into TexCraft\ at the repository root:
;
;   iscc /DAppVersion=0.2.0 /DFileVersion=0.2.0 /O<output dir> scripts\windows-installer.iss
;
; Installs for the current user only, in %LOCALAPPDATA%\Programs\TexCraft, with no
; administrator prompt: TexCraft's updater (crates/update) replaces its own files in place, which
; it could not do under Program Files. Adds a Start menu entry, an optional desktop shortcut and
; an uninstaller (Settings > Apps). Never change AppId: it is how a newer installer finds the
; installed copy and upgrades it instead of installing a second one.

#ifndef AppVersion
  #error Pass the version: iscc /DAppVersion=1.2.3 scripts\windows-installer.iss
#endif
#ifndef Stage
  #define Stage "..\TexCraft"
#endif

[Setup]
AppId={{4987B422-4A03-42AC-9FD2-97D7CA1274A3}
AppName=TexCraft
AppVersion={#AppVersion}
AppVerName=TexCraft {#AppVersion}
AppPublisher=TexCraft
AppPublisherURL=https://github.com/chenwen245299/TexCraft
AppSupportURL=https://github.com/chenwen245299/TexCraft/issues
AppUpdatesURL=https://github.com/chenwen245299/TexCraft/releases
#ifdef FileVersion
VersionInfoVersion={#FileVersion}
#endif
DefaultDirName={autopf}\TexCraft
DisableDirPage=yes
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputBaseFilename=TexCraft-{#AppVersion}-windows-x64-setup
SetupIconFile=..\assets\icon\texcraft.ico
UninstallDisplayIcon={app}\texcraft.exe
UninstallDisplayName=TexCraft
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
CloseApplications=yes

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#Stage}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs

[Icons]
Name: "{autoprograms}\TexCraft"; Filename: "{app}\texcraft.exe"
Name: "{autodesktop}\TexCraft"; Filename: "{app}\texcraft.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\texcraft.exe"; Description: "{cm:LaunchProgram,TexCraft}"; Flags: nowait postinstall skipifsilent
