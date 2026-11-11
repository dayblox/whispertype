#ifndef AppVersion
  #define AppVersion "0.1.1"
#endif

[Setup]
AppId={{B2615475-8302-4834-AE3E-092AA419C04C}
AppName=Whispertype
AppVersion={#AppVersion}
AppPublisher=Dayblox
AppPublisherURL=https://github.com/dayblox/whisper
DefaultDirName={localappdata}\Whispertype
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
LicenseFile=..\LICENSE
CloseApplications=yes
RestartApplications=no

[Tasks]
Name: "autostart"; Description: "Start Whispertype when I sign in"

[Files]
Source: "..\target\release\whispertype.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\THIRD_PARTY_NOTICES.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Whispertype"; Filename: "{app}\whispertype.exe"; Parameters: "run"
Name: "{group}\Uninstall Whispertype"; Filename: "{uninstallexe}"

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Whispertype"; ValueData: """{app}\whispertype.exe"" run"; Tasks: autostart; Flags: uninsdeletevalue

[Run]
Filename: "{app}\whispertype.exe"; Parameters: "run"; Description: "Start Whispertype"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{app}\whispertype.exe"; Parameters: "quit"; RunOnceId: "StopWhispertype"; Flags: runhidden skipifdoesntexist

[Code]
var
  KeyPage: TInputQueryWizardPage;

procedure InitializeWizard;
begin
  KeyPage := CreateInputQueryPage(wpSelectTasks, 'Groq API key',
    'Optional: configure transcription now',
    'Enter your Groq key from console.groq.com/keys. Leave blank to keep an existing key or configure it later. Keys and settings are preserved when uninstalling.');
  KeyPage.Add('API key:', True);
end;

function NextButtonClick(CurPageID: Integer): Boolean;
var
  Key: String;
  I: Integer;
begin
  Result := True;
  if CurPageID <> KeyPage.ID then Exit;
  Key := Trim(KeyPage.Values[0]);
  if Key = '' then Exit;
  Result := (Length(Key) >= 20) and (Copy(Key, 1, 4) = 'gsk_');
  for I := 1 to Length(Key) do
    if Pos(Key[I], 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_') = 0 then Result := False;
  if not Result then MsgBox('Expected a Groq API key beginning with gsk_.', mbError, MB_OK);
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  ConfigDir: String;
begin
  if (CurStep = ssPostInstall) and (Trim(KeyPage.Values[0]) <> '') then begin
    ConfigDir := ExpandConstant('{userappdata}\whispertype');
    if not ForceDirectories(ConfigDir) then RaiseException('Could not create the configuration directory.');
    if not SaveStringToFile(ConfigDir + '\groq.key', AnsiString(Trim(KeyPage.Values[0])), False) then
      RaiseException('Could not save the Groq API key.');
  end;
end;
