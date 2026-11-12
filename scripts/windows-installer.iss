#ifndef AppVersion
  #define AppVersion "0.1.11"
#endif

[Setup]
AppId={{B2615475-8302-4834-AE3E-092AA419C04C}
AppName=Whispertype
AppVersion={#AppVersion}
AppPublisher=Dayblox
AppPublisherURL=https://github.com/dayblox/whispertype
DefaultDirName={localappdata}\Whispertype
DisableDirPage=no
DefaultGroupName=Whispertype
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir=..\dist
OutputBaseFilename=whispertype-windows-x64-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\whispertype.exe
SetupIconFile=..\assets\whispertype.ico
LicenseFile=..\LICENSE
; Stop and verify our exact installed EXE in PrepareToInstall, including helpers.
CloseApplications=no
RestartApplications=no

[Tasks]
Name: "autostart"; Description: "Start Whispertype when I sign in"

[Files]
Source: "..\target\release\whispertype.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\THIRD_PARTY_NOTICES.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Whispertype"; Filename: "{app}\whispertype.exe"; Parameters: "run"; IconFilename: "{app}\whispertype.exe"; IconIndex: 0
Name: "{group}\Uninstall Whispertype"; Filename: "{uninstallexe}"

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Whispertype"; ValueData: """{app}\whispertype.exe"" run"; Tasks: autostart; Flags: uninsdeletevalue

[Run]
Filename: "{app}\whispertype.exe"; Parameters: "run"; Description: "Start Whispertype"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{app}\whispertype.exe"; Parameters: "_stop-for-upgrade ""{app}\whispertype.exe"""; RunOnceId: "StopWhispertype"; Flags: runhidden skipifdoesntexist

[Code]
var
  KeyPage: TInputQueryWizardPage;
  ReplaceKey: TNewCheckBox;
  ConfigDir: String;
  ExistingKey: Boolean;

function ValidKey(Key: String): Boolean;
var
  I: Integer;
begin
  Result := (Length(Key) >= 20) and (Copy(Key, 1, 4) = 'gsk_');
  for I := 1 to Length(Key) do
    if Pos(Key[I], 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_') = 0 then Result := False;
end;

procedure ReplaceKeyClick(Sender: TObject);
begin
  KeyPage.Edits[0].Enabled := ReplaceKey.Checked;
  KeyPage.Edits[0].Visible := ReplaceKey.Checked;
  KeyPage.PromptLabels[0].Visible := ReplaceKey.Checked;
  if not ReplaceKey.Checked then KeyPage.Values[0] := '';
end;

procedure InitializeWizard;
var
  SavedKey: AnsiString;
begin
  ConfigDir := GetEnv('WHISPERTYPE_HOME');
  if ConfigDir = '' then ConfigDir := ExpandConstant('{userappdata}\whispertype');
  ExistingKey := False;
  if LoadStringFromFile(AddBackslash(ConfigDir) + 'groq.key', SavedKey) then
    ExistingKey := ValidKey(Trim(String(SavedKey)));
  ExistingKey := ExistingKey or ValidKey(Trim(GetEnv('GROQ_API_KEY')));
  KeyPage := CreateInputQueryPage(wpSelectTasks, 'Groq API key',
    'Optional: configure transcription now',
    'Enter your Groq key from console.groq.com/keys. Leave blank to keep an existing key or configure it later. Keys and settings are preserved when uninstalling.');
  KeyPage.Add('API key:', True);
  if ExistingKey then begin
    KeyPage.Description := 'Already configured - no key entry needed';
    KeyPage.SubCaptionLabel.Caption := 'Your existing Groq API key was found and will be kept. You can continue without entering anything.';
    ReplaceKey := TNewCheckBox.Create(KeyPage);
    ReplaceKey.Parent := KeyPage.Surface;
    ReplaceKey.Caption := 'Replace my existing key';
    ReplaceKey.Top := KeyPage.PromptLabels[0].Top;
    ReplaceKey.Width := KeyPage.SurfaceWidth;
    ReplaceKey.OnClick := @ReplaceKeyClick;
    KeyPage.PromptLabels[0].Top := ReplaceKey.Top + ReplaceKey.Height + ScaleY(16);
    KeyPage.Edits[0].Top := KeyPage.PromptLabels[0].Top + KeyPage.PromptLabels[0].Height + ScaleY(6);
    ReplaceKeyClick(ReplaceKey);
    Log('Existing credential detected; replacement entry disabled.');
  end;
end;

function NextButtonClick(CurPageID: Integer): Boolean;
var
  Key: String;
begin
  Result := True;
  if CurPageID <> KeyPage.ID then Exit;
  Key := Trim(KeyPage.Values[0]);
  if Key = '' then Exit;
  Result := ValidKey(Key);
  if not Result then MsgBox('Expected a Groq API key beginning with gsk_.', mbError, MB_OK);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  ExitCode: Integer;
begin
  Result := '';
  if not FileExists(ExpandConstant('{app}\whispertype.exe')) then Exit;
  ExtractTemporaryFile('whispertype.exe');
  if not Exec(ExpandConstant('{tmp}\whispertype.exe'),
      ExpandConstant('_stop-for-upgrade "{app}\whispertype.exe"'), '', SW_HIDE,
      ewWaitUntilTerminated, ExitCode) then
    Result := 'Could not stop the installed Whispertype. Close it and retry Setup.'
  else if ExitCode <> 0 then
    Result := 'A Whispertype process is still running. Close Whispertype and retry Setup. No files have been replaced.';
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and (Trim(KeyPage.Values[0]) <> '') then begin
    if not ForceDirectories(ConfigDir) then RaiseException('Could not create the configuration directory.');
    if not SaveStringToFile(AddBackslash(ConfigDir) + 'groq.key', AnsiString(Trim(KeyPage.Values[0])), False) then
      RaiseException('Could not save the Groq API key.');
  end;
end;
