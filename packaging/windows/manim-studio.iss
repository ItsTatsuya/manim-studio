; Inno Setup script for Manim Studio (per-user install, no admin rights needed).
; Named "Manim Studio 2" so it installs next to the 1.x app (same folder and shortcut
; names would overwrite its Start Menu entry).
; Build: script\bundle-windows.ps1 -Installer   (needs Inno Setup 6: winget install JRSoftware.InnoSetup)

#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif
#ifndef SourceDir
  #define SourceDir "..\..\target\release"
#endif

[Setup]
AppId={{6C3B2E4A-6E0E-4E3B-9C55-6A1E8D7F2B10}
AppName=Manim Studio 2
AppVersion={#AppVersion}
AppPublisher=Manim Studio
DefaultDirName={localappdata}\Programs\Manim Studio 2
DefaultGroupName=Manim Studio 2
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir=..\..\dist
OutputBaseFilename=ManimStudio-{#AppVersion}-setup
SetupIconFile=..\..\crates\studio-app\resources\icon.ico
UninstallDisplayIcon={app}\manim-studio.exe
Compression=lzma2/ultra
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

[Files]
Source: "{#SourceDir}\manim-studio.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\studio-engine.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Manim Studio 2"; Filename: "{app}\manim-studio.exe"
Name: "{userdesktop}\Manim Studio 2"; Filename: "{app}\manim-studio.exe"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Run]
Filename: "{app}\manim-studio.exe"; Description: "Open Manim Studio 2"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; Leftovers from in-place updates (previous version kept for rollback).
Type: files; Name: "{app}\*.old"
Type: files; Name: "{app}\*.new"
