#ifndef StageDir
  #define StageDir "..\.build-cache\online-stage"
#endif
#ifndef PayloadPathLength
  #error Build with packaging/build_online.py to supply the payload path inventory.
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
OutputBaseFilename=Manim-Studio-1.0.0-Setup-x64
SetupIconFile=studio.ico
UninstallDisplayIcon={app}\Manim Studio.exe
WizardStyle=modern
WizardSizePercent=110
Compression=lzma2/max
SolidCompression=yes
ArchiveExtraction=full
ExtraDiskSpaceRequired=3000000000
CloseApplications=yes
RestartApplications=no
DisableProgramGroupPage=yes
UninstallDisplayName=Manim Studio

[Messages]
WelcomeLabel2=This installs Manim Studio for your Windows account.%n%nSetup downloads and prepares the rendering tools automatically. Keep your internet connection on until setup finishes. After installation, standard animations and equations work offline.%n%nNo Python installation or commands are required.

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked

[InstallDelete]
#include "prune-previous.iss"
Type: files; Name: "{app}\app\studio_ui\fonts\geist-latin.woff2"
Type: files; Name: "{app}\tools\LICENSE"
Type: files; Name: "{app}\tools\README.txt"
Type: files; Name: "{app}\tools\avdevice-63.dll"
Type: files; Name: "{app}\tools\swscale-10.dll"

[Files]
Source: "{#StageDir}\core\*"; DestDir: "{app}"; Flags: ignoreversion overwritereadonly recursesubdirs createallsubdirs
Source: "{#StageDir}\bootstrap.py"; Flags: dontcopy
Source: "{#StageDir}\downloads.json"; Flags: dontcopy
Source: "{#StageDir}\requirements-online.txt"; Flags: dontcopy
Source: "{#StageDir}\tex-commands.json"; Flags: dontcopy
Source: "{#StageDir}\tex-native-lock.json"; Flags: dontcopy
Source: "{#StageDir}\wheels\*"; DestDir: "{tmp}\wheels"; Flags: dontcopy
Source: "{#StageDir}\vc\*"; DestDir: "{tmp}\prepared\runtime"; Flags: dontcopy
Source: "{tmp}\prepared\*"; DestDir: "{app}"; ExternalSize: 1770000000; Excludes: "*.pyc"; Flags: external ignoreversion overwritereadonly recursesubdirs createallsubdirs

[Icons]
Name: "{group}\Manim Studio"; Filename: "{app}\Manim Studio.exe"
Name: "{autodesktop}\Manim Studio"; Filename: "{app}\Manim Studio.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\Manim Studio.exe"; Description: "Open Manim Studio"; Flags: nowait postinstall skipifsilent

[Code]
#include "installer-paths.iss"

var
  DownloadPage: TDownloadWizardPage;
  PreparePage: TOutputProgressWizardPage;
  Prepared: Boolean;
  ToolFailureText: String;

procedure InitializeWizard;
begin
  DownloadPage := CreateDownloadPage('Downloading render tools', 'Setup will do everything automatically. Please stay connected.', nil);
  DownloadPage.ShowBaseNameInsteadOfUrl := True;
  PreparePage := CreateOutputProgressPage('Preparing render tools', 'Setting up a private runtime for Manim Studio');
end;

procedure ToolOutput(const S: String; const Error, FirstLine: Boolean);
var
  Rest, Caption: String;
  Position, Percent: Integer;
begin
  Log(S);
  if Pos('ERROR|', S) = 1 then begin
    ToolFailureText := Copy(S, 7, Length(S));
  end else if Pos('STAGE|', S) = 1 then begin
    Rest := Copy(S, 7, Length(S));
    Position := Pos('|', Rest);
    Percent := StrToIntDef(Copy(Rest, 1, Position - 1), 0);
    Caption := Copy(Rest, Position + 1, Length(Rest));
    PreparePage.SetText(Caption, 'Please wait. No commands or additional installers are needed.');
    PreparePage.SetProgress(Percent, 100);
  end else if Trim(S) <> '' then
    PreparePage.SetText(PreparePage.Msg1Label.Caption, Copy(S, 1, 170));
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Code: Integer;
  LogPath: String;
begin
  Result := ValidateInstallerPath({#PayloadPathLength});
  if Result <> '' then exit;
  if Prepared then begin
    Result := ValidatePreparedPaths;
    exit;
  end;
  ToolFailureText := '';
  LogPath := ExpandConstant('{localappdata}\ManimStudio\setup.log');
  try
    DownloadPage.Clear;
    #include StageDir + "\downloads.iss"
    DownloadPage.Show;
    try
      DownloadPage.Download;
    finally
      DownloadPage.Hide;
    end;
    PreparePage.Show;
    try
      PreparePage.SetText('Preparing Python and audio tools', 'Checking and extracting verified downloads...');
      PreparePage.SetProgress(0, 100);
      { Only clear our explicitly named temporary staging folders on retry. }
      DelTree(ExpandConstant('{tmp}\prepared'), True, True, True);
      ExtractArchive(ExpandConstant('{tmp}\python.zip'), ExpandConstant('{tmp}\prepared\runtime'), '', True, nil);
      ExtractTemporaryFiles('*');
      ForceDirectories(ExtractFileDir(LogPath));
      if not ExecAndLogOutput(ExpandConstant('{tmp}\prepared\runtime\python.exe'),
        '-u "' + ExpandConstant('{tmp}\bootstrap.py') + '" --downloads "' + ExpandConstant('{tmp}') +
        '" --destination "' + ExpandConstant('{tmp}\prepared') + '" --log "' + LogPath + '"',
        ExpandConstant('{tmp}'), SW_HIDE, ewWaitUntilTerminated, Code, @ToolOutput) then
        RaiseException('Could not start the private Python runtime.');
      if Code <> 0 then begin
        if ToolFailureText <> '' then
          RaiseException(ToolFailureText)
        else
          RaiseException('A rendering tool could not be prepared. Check your internet connection and try again.');
      end;
      Prepared := True;
      Result := ValidatePreparedPaths;
    finally
      PreparePage.Hide;
    end;
  except
    Result := GetExceptionMessage + #13#10 + #13#10 +
      'Click Install to retry. Your existing app and saved work have not been changed.' + #13#10 +
      'Setup details: ' + LogPath;
    ForceDirectories(ExtractFileDir(LogPath));
    SaveStringToFile(LogPath, #13#10 + Result + #13#10, True);
  end;
end;
