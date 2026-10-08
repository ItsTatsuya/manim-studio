#ifndef BundleDir
  #define BundleDir "..\dist\Manim Studio Portable"
#endif
#ifndef PayloadPathLength
  #error Build with packaging/build.py to supply the payload path inventory.
#endif
[Setup]
AppId={{E55B93EB-6197-4FAA-BF08-9EC68F6ACBA4}
AppName=Manim Studio
AppVersion=1.0.0
AppPublisher=ItsTatsuya
DefaultDirName={localappdata}\Programs\Manim Studio
DefaultGroupName=Manim Studio
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.19041
OutputDir=..\dist
OutputBaseFilename=Manim-Studio-1.0.0-Offline-Setup-x64
SetupIconFile=studio.ico
UninstallDisplayIcon={app}\Manim Studio.exe
WizardStyle=modern
WizardSizePercent=110
Compression=lzma2/max
SolidCompression=yes
LZMANumBlockThreads=2
CloseApplications=yes
RestartApplications=no
DisableProgramGroupPage=yes
UninstallDisplayName=Manim Studio

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked

[InstallDelete]
; Remove only obsolete shipped files during an upgrade. UserData is untouched.
#include "prune-previous.iss"
Type: files; Name: "{app}\app\studio_ui\fonts\geist-latin.woff2"
Type: files; Name: "{app}\tools\LICENSE"
Type: files; Name: "{app}\tools\README.txt"
Type: files; Name: "{app}\tools\avdevice-63.dll"
Type: files; Name: "{app}\tools\swscale-10.dll"

[Files]
Source: "{#BundleDir}\*"; DestDir: "{app}"; Excludes: "portable.mode,UserData\*,*.pyc"; Flags: ignoreversion overwritereadonly recursesubdirs createallsubdirs

[Icons]
Name: "{group}\Manim Studio"; Filename: "{app}\Manim Studio.exe"
Name: "{autodesktop}\Manim Studio"; Filename: "{app}\Manim Studio.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\Manim Studio.exe"; Description: "Open Manim Studio"; Flags: nowait postinstall skipifsilent

[Code]
#include "installer-paths.iss"

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := ValidateInstallerPath({#PayloadPathLength});
end;
