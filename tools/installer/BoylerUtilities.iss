; Boyler Utilities - the installer (Order 032). Inno Setup 6.7. Build with tools/installer/build.sh (it passes the defines).
; - per user, never admin: %LOCALAPPDATA%\Programs\Boyler Utilities, Start menu shortcut, "Installed apps" entry under HKCU;
; - tasks (both ticked): Start with Windows (the app's own Run value, see app/src/pages/settings/autostart.rs) and
;   Everything for Search (Order 049: Search's OWN copy of voidtools' Everything + our manual service, set up by the app's
;   --install-everything step - the ONLY step that may ask for admin; nothing of it shows outside the app);
; - install / uninstall close the running app first (`--quit`); uninstall asks to keep the settings (default keep), never
;   removes a user's own Everything (Search's own copy + service go). Uninstall /KEEPSETTINGS=no deletes them without asking (silent proof runs).
; - uninstall first offers "Undo my Windows changes too?" (default Yes) when the app's change log isn't empty (Order 036:
;   the app's --undo-windows, no window); /UNDOWINDOWS=no|yes answers without asking.

#ifndef AppVersion
  #error Build with tools/installer/build.sh
#endif

#define AppName "Boyler Utilities"
#define AppExe "Boyler Utilities.exe"
#define AppGuid "{9C7C5E8C-178D-40CC-9BC5-54986753A645}"

[Setup]
AppId={{#AppGuid}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
VersionInfoVersion={#AppVersion}
VersionInfoProductName={#AppName}
VersionInfoDescription={#AppName} Setup
PrivilegesRequired=lowest
DefaultDirName={autopf}\{#AppName}
DisableDirPage=yes
DisableProgramGroupPage=yes
DisableWelcomePage=yes
ShowLanguageDialog=no
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
SetupIconFile={#SrcRoot}\app\assets\pane_dark.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
OutputDir={#OutDir}
OutputBaseFilename={#AppName} Setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; the running app is closed by [Code] (--quit), not by the Restart Manager dialog
CloseApplications=no
; "Start with Windows" starts ticked on a first install; on an update it shows what the app has now ([Code] InitializeWizard)
UsePreviousTasks=no

[Tasks]
Name: "autostart"; Description: "Start with Windows"
Name: "everything"; Description: "Install Everything for Search (free, voidtools)"; Check: not EverythingFound

[Files]
Source: "{#AppExeSrc}"; DestDir: "{app}"; DestName: "{#AppExe}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"

[Registry]
; the same value the app's Settings > Start with Windows writes (name "Boyler Utilities", the quoted exe path)
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "{#AppName}"; \
  ValueData: """{app}\{#AppExe}"""; Tasks: autostart; Check: WantAutostart

[Run]
; Order 041 (the owner Oct 8: "it didn't start the app by itself ... it should open up when its installed"): the last page's
; ticked box starts the app when Setup finishes; its first start shows the bubble above the tray icon (app/src/welcome.rs).
; Not in a silent run (proof installs, prove.ps1) - those never start the app.
Filename: "{app}\{#AppExe}"; Description: "Start {#AppName}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; what the self-updater may leave next to the exe (crates/updater: <exe>.update-part / -new / -old / -result)
Type: files; Name: "{app}\{#AppExe}.update-*"
Type: dirifempty; Name: "{app}"

[Code]
const
  MUTEX = 'Local\BoylerUtilities';
  RUN_KEY = 'Software\Microsoft\Windows\CurrentVersion\Run';
  UNINST_KEY = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{#AppGuid}_is1';

{ ---------------------------------------------------------------- Everything (crates/search/src/real/ours.rs) }

{ Order 049: Search's OWN Everything (our copy in Program Files + our manual service), never one the user installed. }
function EverythingFound: Boolean;
begin
  Result := FileExists(ExpandConstant('{commonpf64}\Boyler Utilities\Everything\Everything.exe'));
end;

function WantEverything: Boolean;
begin
  Result := WizardIsTaskSelected('everything') and not EverythingFound;
end;

#ifdef ProofNoMsi
{ proof builds only (tools/installer/prove.ps1): nothing is set up }
procedure InstallEverything;
begin
  Log('PROOF: Everything set-up not run (proof build)');
end;
#else
{ The app itself does it (no window): downloads voidtools' portable Everything, checks it, and its own admin helper (the
  ONE admin prompt) puts it in Program Files with our manual service - and tidies up the Everything v1.0.0 installed. }
procedure InstallEverything;
var
  Code: Integer;
begin
  if not Exec(ExpandConstant('{app}\{#AppExe}'), '--install-everything', '', SW_HIDE, ewWaitUntilTerminated, Code) then
    Log('Everything: the set-up did not start (' + SysErrorMessage(Code) + ')')
  else if Code = 0 then
    Log('Everything set up')
  else if Code = 1602 then
    Log('Everything: the admin prompt was answered No')
  else begin
    Log('Everything: the set-up stopped (code ' + IntToStr(Code) + ')');
    SuppressibleMsgBox('Everything for Search could not be set up (code ' + IntToStr(Code) + ').' + #13#10#13#10 +
      'The Search tab can set it up later.', mbInformation, MB_OK, IDOK);
  end;
end;
#endif

{ ---------------------------------------------------------------- closing the running app }

{ Hands --quit to the running copy (the app's own single-instance message) and waits up to 10 s for it to end. }
function CloseApp(const Exe: String): Boolean;
var
  Code, I: Integer;
begin
  Result := not CheckForMutexes(MUTEX);
  if Result then
    Exit;
  if FileExists(Exe) then
    Exec(Exe, '--quit', '', SW_HIDE, ewWaitUntilTerminated, Code);
  for I := 1 to 100 do begin
    if not CheckForMutexes(MUTEX) then begin
      Result := True;
      Exit;
    end;
    Sleep(100);
  end;
end;

{ ---------------------------------------------------------------- Start with Windows }

{ The Run value starts the exe in Dir. }
function AutostartIn(const Dir: String): Boolean;
var
  V: String;
begin
  Result := (Dir <> '') and RegQueryStringValue(HKCU, RUN_KEY, '{#AppName}', V)
    and (Pos(Lowercase(AddBackslash(Dir) + '{#AppExe}'), Lowercase(V)) > 0);
end;

{ The Run value goes only when it starts THIS install's exe (the app's Settings may have pointed it elsewhere). }
procedure RemoveAutostart;
begin
  if AutostartIn(ExpandConstant('{app}')) then
    RegDeleteValue(HKCU, RUN_KEY, '{#AppName}');
end;

{ An update over an install whose Start with Windows is off now (switched off in the app's Settings): running a newer
  Setup must not switch it back on. The tasks page shows it unticked; a silent update keeps it off unless its command line
  asks for it (/TASKS or /MERGETASKS naming "autostart"). }
var
  AutostartWasOff, TasksPageSeen: Boolean;

function InitializeSetup: Boolean;
var
  Loc, Day: String;
begin
  AutostartWasOff := RegQueryStringValue(HKCU, UNINST_KEY, 'InstallLocation', Loc) and not AutostartIn(Loc);
  { Order 049: keep the day of the FIRST install before this install rewrites the entry's date (it tells v1.0.0's
    Everything from a user's own - crates/search/src/real/ours.rs v100_rule) }
  if not RegValueExists(HKCU, 'Software\BoylerUtilities', 'FirstInstallDate') then
    if RegQueryStringValue(HKCU, UNINST_KEY, 'InstallDate', Day) then
      RegWriteStringValue(HKCU, 'Software\BoylerUtilities', 'FirstInstallDate', Day);
  Result := True;
end;

function NamesAutostart(const List: String): Boolean;
var
  L: String;
begin
  L := ',' + Lowercase(List) + ',';
  StringChangeEx(L, ' ', '', True);
  Result := (Pos(',autostart,', L) > 0) or (Pos(',*autostart,', L) > 0);
end;

function WantAutostart: Boolean;
begin
  Result := not AutostartWasOff or TasksPageSeen
    or NamesAutostart(ExpandConstant('{param:TASKS|}')) or NamesAutostart(ExpandConstant('{param:MERGETASKS|}'));
end;

procedure CurPageChanged(CurPageID: Integer);
begin
  if (CurPageID = wpSelectTasks) and not TasksPageSeen and not WizardSilent then begin
    if AutostartWasOff then
      WizardSelectTasks('!autostart');
    TasksPageSeen := True;
  end;
end;

{ ---------------------------------------------------------------- setup events }

function NextButtonClick(CurPageID: Integer): Boolean;
begin
  Result := True;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';
  if CheckForMutexes(MUTEX) then begin
    { the new exe hands --quit to whichever copy runs }
    ExtractTemporaryFile('{#AppExe}');
    if not CloseApp(ExpandConstant('{tmp}\{#AppExe}')) then
      Result := '{#AppName} is still running. Close it (tray icon > Quit) and run Setup again.';
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep <> ssPostInstall then
    Exit;
  { unticked = off (an update over an install may have had it on) }
  if not WizardIsTaskSelected('autostart') or not WantAutostart then
    RemoveAutostart;
  if WantEverything then
    InstallEverything;
end;

{ ---------------------------------------------------------------- uninstall }

{ Only a full path (a drive "X:\..." or a share "\\..."): an empty or odd variable must never make a relative folder. }
procedure AddDataDir(var Dirs: TArrayOfString; const Base, Name: String);
var
  N: Integer;
begin
  if not ((Length(Base) >= 3) and (Copy(Base, 2, 2) = ':\')) and not ((Length(Base) >= 3) and (Copy(Base, 1, 2) = '\\')) then begin
    Log('Settings folder skipped (no full path in the variable: "' + Base + '")');
    Exit;
  end;
  N := GetArrayLength(Dirs);
  SetArrayLength(Dirs, N + 1);
  Dirs[N] := AddBackslash(Base) + Name;
end;

function SettingsDirs: TArrayOfString;
begin
  { the folders the app itself uses (it reads the APPDATA / LOCALAPPDATA variables): settings, mouse + controller
    backups, Activity history, Screenshots index, our Everything instance's index }
  SetArrayLength(Result, 0);
  AddDataDir(Result, GetEnv('APPDATA'), '{#AppName}');
  AddDataDir(Result, GetEnv('LOCALAPPDATA'), 'BoylerUtilities');
end;

function KeepSettings: Boolean;
var
  P: String;
begin
  P := Lowercase(ExpandConstant('{param:KEEPSETTINGS|}'));
  if (P = 'no') or (P = '0') then
    Result := False
  else if (P = 'yes') or (P = '1') then
    Result := True
  else
    Result := SuppressibleMsgBox('Keep your {#AppName} settings?' + #13#10#13#10 +
      'Yes - keep them (they come back if you install it again).' + #13#10 +
      'No - delete them too.', mbConfirmation, MB_YESNO or MB_DEFBUTTON1, IDYES) = IDYES;
end;

{ Order 036: the app's change log (every change it made to Windows, with the value before). Not empty -> "Undo my Windows
  changes too?" (default Yes); Yes -> the app itself puts them back with no window (--undo-windows), then the uninstall goes
  on. The app's exit codes (app/src/main.rs undo_windows): --undo-windows-count = how many; --undo-windows = how many failed;
  251 = the app is still running. /UNDOWINDOWS=yes|no answers without asking (a silent run with neither = Yes, the default). }
const
  UNDO_APP_RUNNING = 251;

function WantUndo(const N: Integer): Boolean;
var
  P, What: String;
begin
  P := Lowercase(ExpandConstant('{param:UNDOWINDOWS|}'));
  if (P = 'no') or (P = '0') then
    Result := False
  else if (P = 'yes') or (P = '1') then
    Result := True
  else begin
    if N = 1 then
      What := '1 setting'
    else
      What := IntToStr(N) + ' settings';
    Result := SuppressibleMsgBox('Undo my Windows changes too?' + #13#10#13#10 +
      'Yes - ' + What + ' this app changed in Windows go back to how your PC was.' + #13#10 +
      'No - they stay as they are.', mbConfirmation, MB_YESNO or MB_DEFBUTTON1, IDYES) = IDYES;
  end;
end;

procedure UndoWindowsChanges;
var
  Exe: String;
  N, Code: Integer;
begin
  Exe := ExpandConstant('{app}\{#AppExe}');
  if not FileExists(Exe) then
    Exit;
  if not Exec(Exe, '--undo-windows-count', '', SW_HIDE, ewWaitUntilTerminated, N) then begin
    Log('Undo: the app could not count its changes');
    Exit;
  end;
  Log('Undo: ' + IntToStr(N) + ' change(s) in the log');
  if (N <= 0) or (N >= UNDO_APP_RUNNING) then
    Exit;
  if not WantUndo(N) then begin
    Log('Undo: declined');
    Exit;
  end;
  if not Exec(Exe, '--undo-windows', '', SW_HIDE, ewWaitUntilTerminated, Code) then
    Log('Undo: the app could not be started')
  else if Code = 0 then
    Log('Undo: every change put back')
  else if (Code > 0) and (Code < UNDO_APP_RUNNING) then begin
    Log('Undo: ' + IntToStr(Code) + ' change(s) could not be put back');
    SuppressibleMsgBox(IntToStr(Code) + ' of your Windows changes could not be put back. They stay as they are.',
      mbInformation, MB_OK, IDOK);
  end else
    Log('Undo: did not finish (exit code ' + IntToStr(Code) + ')');
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Dirs: TArrayOfString;
  I: Integer;
begin
  case CurUninstallStep of
    usAppMutexCheck:
      while not CloseApp(ExpandConstant('{app}\{#AppExe}')) do
        if SuppressibleMsgBox('{#AppName} is still running. Close it (tray icon > Quit), then click Retry.',
          mbError, MB_RETRYCANCEL, IDCANCEL) = IDCANCEL then
          Abort;
    usUninstall:
      begin
        { first, while the exe and the settings (its change log) are still there }
        UndoWindowsChanges;
        RemoveAutostart;
        { Order 049: Search's own Everything (copy + service) goes with the app - one admin prompt, only when it is there }
        if EverythingFound then
          if not Exec(ExpandConstant('{app}\{#AppExe}'), '--uninstall-everything', '', SW_HIDE, ewWaitUntilTerminated, I) then
            Log('Everything: removing it did not start')
          else
            Log('Everything removed (code ' + IntToStr(I) + ')');
        Dirs := SettingsDirs;
        for I := 0 to GetArrayLength(Dirs) - 1 do
          Log('Settings folder: ' + Dirs[I]);
        if not KeepSettings then
          for I := 0 to GetArrayLength(Dirs) - 1 do
            if DirExists(Dirs[I]) then
              if DelTree(Dirs[I], True, True, True) then
                Log('Deleted ' + Dirs[I])
              else
                Log('Could not delete all of ' + Dirs[I]);
      end;
  end;
end;
