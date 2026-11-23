; TuneSync's Windows installer (Inno Setup 6.7; built on Linux by tools/make-installer.sh).
;
; Installs TuneSync_Control.dll into eMotion LV1's Modules folder (LV1 loads every
; Modules\*Control.dll), creates %ProgramData%\TuneSync for the settings and the log,
; and, when the task is ticked, adds TuneSync to LV1's control surfaces (LV1's
; preferences, edited while LV1 is closed, backed up first). It needs administrator
; rights: the Modules folder is under Program Files (x86).
;
; Everything refuses while eMotion LV1 runs: LV1 holds every *Control.dll open and
; rewrites its preferences when it closes. The setup never closes LV1 itself.
;
; ISCC defines: AppVersion (0.1.0), Dist (folder with TuneSync_Control.dll, Windows path),
; OutDir, Build (commit and time). Sign=1 adds SignTool=tssign (make-installer.sh --sign).

#ifndef AppVersion
  #error AppVersion is required (/DAppVersion=0.1.0)
#endif
#ifndef Dist
  #error Dist is required (/DDist=Z:\path\to\dist)
#endif
#ifndef OutDir
  #define OutDir "."
#endif
#ifndef Build
  #define Build "dev"
#endif

[Setup]
AppId={{3C8E2B71-5D4A-4F0E-9B26-7A1D0C9E4F58}
AppName=TuneSync
AppVersion={#AppVersion}
AppVerName=TuneSync {#AppVersion}
AppPublisher=Orion Technologies
AppPublisherURL=https://github.com/flaviusemil/tunesync-lv1
AppSupportURL=https://github.com/flaviusemil/tunesync-lv1/issues
VersionInfoVersion={#AppVersion}
; Every build its own bytes (Smart App Control judges a file by its hash).
VersionInfoTextVersion={#AppVersion} {#Build}
VersionInfoProductTextVersion={#AppVersion} {#Build}
VersionInfoProductName=TuneSync
VersionInfoDescription=TuneSync {#AppVersion} setup
PrivilegesRequired=admin
DefaultDirName={commonpf64}\TuneSync
DisableDirPage=yes
DisableProgramGroupPage=yes
DisableReadyPage=no
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
WizardStyle=modern
UninstallDisplayName=TuneSync
SetupIconFile=..\assets\logo\tunesync.ico
UninstallDisplayIcon={app}\tunesync.ico
OutputDir={#OutDir}
OutputBaseFilename=TuneSync-{#AppVersion}-setup
Compression=lzma2/max
SolidCompression=yes
; Never let the Restart Manager close LV1: the setup asks the user to.
CloseApplications=no
RestartApplications=no
#ifdef Sign
SignTool=tssign
SignedUninstaller=yes
#endif

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "addsurface"; Description: "Add TuneSync to eMotion LV1's control surfaces (else: LV1 Setup > Control Surfaces)"; Flags: checkedonce

[Dirs]
; Users-modify: a folder an elevated setup creates is otherwise read-only to LV1's normal token.
Name: "{commonappdata}\TuneSync"; Permissions: users-modify

[Files]
Source: "{#Dist}\TuneSync_Control.dll"; DestDir: "{code:ModulesDir}"; Flags: ignoreversion
; The picture LV1 shows on TuneSync's tile (Setup > System Inventory > CONTROLS): the DLL
; sends this path when the file is beside it, and Qt takes the @2x on a HiDPI screen.
Source: "..\assets\lv1-surface\tunesync-surface.png"; DestDir: "{code:ModulesDir}"; Flags: ignoreversion
Source: "..\assets\lv1-surface\tunesync-surface@2x.png"; DestDir: "{code:ModulesDir}"; Flags: ignoreversion
Source: "{#Dist}\BUILD.txt"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "lv1-prefs.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\assets\logo\tunesync.ico"; DestDir: "{app}"; Flags: ignoreversion

[Run]
; As the user who started the setup (LV1's preferences are per user).
Filename: "powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\lv1-prefs.ps1"" -Add"; Flags: runhidden runasoriginaluser waituntilterminated; Tasks: addsurface; StatusMsg: "Adding TuneSync to LV1's control surfaces..."

[UninstallRun]
Filename: "powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\lv1-prefs.ps1"" -Remove"; Flags: runhidden waituntilterminated; RunOnceId: "RemoveSurface"

[UninstallDelete]
Type: files; Name: "{code:ModulesDir}\TuneSync_Control.dll"
Type: files; Name: "{code:ModulesDir}\tunesync-surface.png"
Type: files; Name: "{code:ModulesDir}\tunesync-surface@2x.png"

[Code]
function ModulesDir(Param: String): String;
begin
  Result := ExpandConstant('{commonpf32}') + '\Waves\eMotion LV1\Modules';
end;

// True while eMotion LV1 runs (tasklist, so no WMI or extra DLL is needed).
function LV1Running(): Boolean;
var
  r: TExecOutput;
  code: Integer;
  i: Integer;
begin
  Result := False;
  if ExecAndCaptureOutput(ExpandConstant('{sys}\tasklist.exe'), '/FI "IMAGENAME eq eMotion LV1.exe" /NH', '', SW_HIDE, ewWaitUntilTerminated, code, r) then
    for i := 0 to GetArrayLength(r.StdOut) - 1 do
      if Pos('eMotion LV1.exe', r.StdOut[i]) > 0 then
        Result := True;
end;

function WaitForLV1Closed(const What: String): Boolean;
begin
  Result := True;
  while LV1Running() do begin
    if SuppressibleMsgBox('eMotion LV1 is open. Close it, then click Retry to ' + What + '.' + #13#10 + #13#10 +
                          '(LV1 keeps its control-surface modules open while it runs.)',
                          mbInformation, MB_RETRYCANCEL, IDCANCEL) <> IDRETRY then begin
      Result := False;
      exit;
    end;
  end;
end;

function InitializeSetup(): Boolean;
begin
  Result := False;
  if not DirExists(ModulesDir('')) then begin
    SuppressibleMsgBox('eMotion LV1 was not found (no ' + ModulesDir('') + ').' + #13#10 + #13#10 +
                       'Install eMotion LV1 first, then run this setup again.', mbError, MB_OK, IDOK);
    exit;
  end;
  Result := WaitForLV1Closed('install TuneSync');
end;

function InitializeUninstall(): Boolean;
begin
  Result := WaitForLV1Closed('uninstall TuneSync');
end;
