; MDLite installer, for Inno Setup 6.4 or later.
;
; Build it with installer\build-installer.ps1, which compiles the program first
; and passes the version in. By hand:
;   ISCC /DAppVersion=0.1.0 installer\mdlite.iss
;
; One user by default, nobody's administrator rights needed: MDLite goes under
; %LOCALAPPDATA%\Programs. The first page of the wizard offers all users, which
; installs under Program Files and asks Windows for permission.

#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif
#ifndef SourceDir
  #define SourceDir "..\dist"
#endif
#ifndef OutputDir
  #define OutputDir "..\dist\installer"
#endif
#define AppName "MDLite"
#define AppExe "mdlite.exe"
#define ProgId "MDLite.Markdown"

[Setup]
AppId={{6893FA32-7C9A-4FA6-9700-929922862FFE}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=Hamza
DefaultDirName={autopf}\{#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; Windows 10 version 1703, which per-monitor DPI awareness needs.
MinVersion=10.0.15063
WizardStyle=modern
SetupIconFile=..\assets\mdlite.ico
UninstallDisplayIcon={app}\{#AppExe}
LicenseFile=..\LICENSE
OutputDir={#OutputDir}
OutputBaseFilename=MDLite-Setup-{#AppVersion}
Compression=lzma2/ultra64
SolidCompression=yes
; Ask a running MDLite to close so an upgrade can replace it, instead of
; failing on a locked file or asking for a restart.
CloseApplications=yes
RestartApplications=no
; Tell Explorer the file associations changed.
ChangesAssociations=yes
VersionInfoVersion={#AppVersion}.0
VersionInfoDescription={#AppName} Setup

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "assoc"; Description: "Make {#AppName} the default reader for Markdown files (.md, .markdown, .mdown, .mkd)"; GroupDescription: "File types:"
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked

[Files]
Source: "{#SourceDir}\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
; The palettes a new install starts from. The program copies this beside the
; user's own settings on first run; it is never the live file, so Program Files
; does not need to be writable.
Source: "..\mdlite.ini"; DestDir: "{app}"; DestName: "mdlite.default.ini"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "..\examples\*.md"; DestDir: "{app}\examples"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autoprograms}\Uninstall {#AppName}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Registry]
; The same keys the Settings button writes (src\association.rs), so the two
; agree whichever ran last. Keys are listed before their values: Windows removes
; them in reverse, so a key is only deleted once it is empty.
;
; MDLite as a program Windows can offer for Markdown. This does not claim anything.
Root: HKA; Subkey: "Software\Classes\{#ProgId}"; ValueType: string; ValueName: ""; ValueData: "Markdown document"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\{#ProgId}\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"",0"
Root: HKA; Subkey: "Software\Classes\{#ProgId}\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "{#AppName}"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}\SupportedTypes"; ValueType: string; ValueName: ".md"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}\SupportedTypes"; ValueType: string; ValueName: ".markdown"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}\SupportedTypes"; ValueType: string; ValueName: ".mdown"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}\SupportedTypes"; ValueType: string; ValueName: ".mkd"; ValueData: ""
; What Windows lists under Settings > Default apps.
Root: HKA; Subkey: "Software\{#AppName}"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\{#AppName}\Capabilities"; ValueType: string; ValueName: "ApplicationName"; ValueData: "{#AppName}"
Root: HKA; Subkey: "Software\{#AppName}\Capabilities"; ValueType: string; ValueName: "ApplicationDescription"; ValueData: "Lightweight Markdown reader"
Root: HKA; Subkey: "Software\{#AppName}\Capabilities\FileAssociations"; ValueType: string; ValueName: ".md"; ValueData: "{#ProgId}"
Root: HKA; Subkey: "Software\{#AppName}\Capabilities\FileAssociations"; ValueType: string; ValueName: ".markdown"; ValueData: "{#ProgId}"
Root: HKA; Subkey: "Software\{#AppName}\Capabilities\FileAssociations"; ValueType: string; ValueName: ".mdown"; ValueData: "{#ProgId}"
Root: HKA; Subkey: "Software\{#AppName}\Capabilities\FileAssociations"; ValueType: string; ValueName: ".mkd"; ValueData: "{#ProgId}"
Root: HKA; Subkey: "Software\RegisteredApplications"; ValueType: string; ValueName: "{#AppName}"; ValueData: "Software\{#AppName}\Capabilities"; Flags: uninsdeletevalue
; "Open with" lists MDLite for each extension.
Root: HKA; Subkey: "Software\Classes\.md"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.md\OpenWithProgids"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.md\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.markdown"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.markdown\OpenWithProgids"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.markdown\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.mdown"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.mdown\OpenWithProgids"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.mdown\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.mkd"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.mkd\OpenWithProgids"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.mkd\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue
; The default reader, when the box is ticked. Only an extension that opens with
; nothing yet is taken: what the person already chose is theirs to change, in
; Windows Settings, which the last page of the wizard offers to open.
Root: HKA; Subkey: "Software\Classes\.md"; ValueType: string; ValueName: ""; ValueData: "{#ProgId}"; Flags: uninsdeletevalue; Tasks: assoc; Check: CanClaim('.md')
Root: HKA; Subkey: "Software\Classes\.markdown"; ValueType: string; ValueName: ""; ValueData: "{#ProgId}"; Flags: uninsdeletevalue; Tasks: assoc; Check: CanClaim('.markdown')
Root: HKA; Subkey: "Software\Classes\.mdown"; ValueType: string; ValueName: ""; ValueData: "{#ProgId}"; Flags: uninsdeletevalue; Tasks: assoc; Check: CanClaim('.mdown')
Root: HKA; Subkey: "Software\Classes\.mkd"; ValueType: string; ValueName: ""; ValueData: "{#ProgId}"; Flags: uninsdeletevalue; Tasks: assoc; Check: CanClaim('.mkd')

[Run]
Filename: "{app}\{#AppExe}"; Parameters: """{app}\examples\showcase.md"""; Description: "Open the {#AppName} showcase"; Flags: nowait postinstall skipifsilent
; Windows alone decides the default once a person has chosen one, so this opens
; the page where that choice is made, with MDLite already listed.
Filename: "ms-settings:defaultapps?registeredAppUser={#AppName}"; Description: "Choose {#AppName} in Windows Default apps"; Flags: shellexec postinstall skipifsilent; Tasks: assoc

[UninstallRun]
; An open program cannot be deleted: ask it to close first.
Filename: "{app}\{#AppExe}"; Parameters: "--close"; Flags: runhidden; RunOnceId: "CloseRunningCopy"
; Forget what the Settings button may have registered since, for this user.
Filename: "{app}\{#AppExe}"; Parameters: "--unregister"; Flags: runhidden; RunOnceId: "ForgetMarkdownRegistration"

[Code]
{ An upgrade replaces a program that may be open. Closing it here, by asking it
  to quit, is gentler than leaving a locked file for the end of setup. }
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Existing: String;
  Code: Integer;
begin
  Result := '';
  Existing := ExpandConstant('{app}\{#AppExe}');
  if FileExists(Existing) then
    Exec(Existing, '--close', '', SW_HIDE, ewWaitUntilTerminated, Code);
end;

{ True when nothing opens this extension yet, or when MDLite already does. }
function CanClaim(const Extension: String): Boolean;
var
  Current: String;
begin
  Result := (not RegQueryStringValue(HKEY_CLASSES_ROOT, Extension, '', Current))
    or (Current = '') or (Current = '{#ProgId}');
end;

{ Settings are the reader's, not the program's: keep them unless asked. Silent
  uninstalls keep them too, unless given /purge=1. }
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Folder: String;
begin
  if CurUninstallStep <> usPostUninstall then
    Exit;
  Folder := ExpandConstant('{userappdata}\{#AppName}');
  if not DirExists(Folder) then
    Exit;
  if (ExpandConstant('{param:purge|0}') = '1') or
     ((not UninstallSilent) and
      (MsgBox('Also delete your {#AppName} settings (palettes and preferences)?' + #13#10#13#10 + Folder,
        mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES)) then
    DelTree(Folder, True, True, True);
end;
