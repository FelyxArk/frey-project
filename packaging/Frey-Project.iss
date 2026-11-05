; Instalador do Frey Project. Compilado por Build-Installer.ps1, que define as constantes abaixo.
#ifndef StageRoot
  #error StageRoot is required
#endif
#ifndef OutputRoot
  #error OutputRoot is required
#endif
#ifndef AppVersionText
  #error AppVersionText is required
#endif

[Setup]
AppId={{6E0A1F3C-0B5D-4E2A-9F67-3C1D7A5B8E42}
AppName=Frey Project
AppVersion={#AppVersionText}
AppVerName=Frey Project {#AppVersionText}
AppPublisher=Frey
DefaultDirName={localappdata}\Programs\Frey Project
DefaultGroupName=Frey Project
DisableDirPage=yes
DisableProgramGroupPage=yes
DisableWelcomePage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
CloseApplications=no
RestartApplications=no
RestartIfNeededByRun=no
UninstallDisplayName=Frey Project
UninstallDisplayIcon={app}\frey-project.exe
OutputDir={#OutputRoot}
OutputBaseFilename=Frey-Project-Setup-{#AppVersionText}
Compression=lzma2
SolidCompression=yes
LZMANumBlockThreads=4
SetupLogging=yes
SetupIconFile=branding\frey-setup.ico
WizardStyle=modern dark windows11 hidebevels
WizardBackColor=#191b1f
WizardImageFile=
WizardSmallImageFile=branding\frey-header.png
WizardSmallImageBackColor=#191b1f
ShowLanguageDialog=auto

[Languages]
Name: "ptBR"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "es"; MessagesFile: "compiler:Languages\Spanish.isl"
Name: "fr"; MessagesFile: "compiler:Languages\French.isl"
Name: "ja"; MessagesFile: "compiler:Languages\Japanese.isl"
Name: "ru"; MessagesFile: "compiler:Languages\Russian.isl"
Name: "ar"; MessagesFile: "compiler:Languages\Arabic.isl"

[CustomMessages]
ptBR.AutoStart=Iniciar com o Windows (na bandeja; o atalho Ctrl+Alt+P só funciona com o app aberto)
en.AutoStart=Start with Windows (in the tray; the Ctrl+Alt+P shortcut only works while the app is running)
es.AutoStart=Iniciar con Windows (en la bandeja; el atajo Ctrl+Alt+P solo funciona con la app abierta)
fr.AutoStart=Démarrer avec Windows (dans la zone de notification ; le raccourci Ctrl+Alt+P ne fonctionne que si l'application est ouverte)
ja.AutoStart=Windows と同時に起動 (タスクトレイ。Ctrl+Alt+P はアプリの起動中のみ使えます)
ru.AutoStart=Запускать вместе с Windows (в трее; сочетание Ctrl+Alt+P работает, только когда приложение запущено)
ar.AutoStart=التشغيل مع Windows (في علبة النظام؛ يعمل الاختصار Ctrl+Alt+P فقط عندما يكون التطبيق مفتوحًا)
ptBR.Webview=Instalando o Microsoft WebView2...
en.Webview=Installing Microsoft WebView2...
es.Webview=Instalando Microsoft WebView2...
fr.Webview=Installation de Microsoft WebView2...
ja.Webview=Microsoft WebView2 をインストールしています...
ru.Webview=Установка Microsoft WebView2...
ar.Webview=جارٍ تثبيت Microsoft WebView2...

[Tasks]
Name: "autostart"; Description: "{cm:AutoStart}"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"

[Files]
Source: "{#StageRoot}\frey-project.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageRoot}\bin\*"; DestDir: "{app}\bin"; Flags: ignoreversion
Source: "{#StageRoot}\licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion
Source: "{#StageRoot}\NOTICES.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageRoot}\webview2\MicrosoftEdgeWebView2RuntimeInstallerX64.exe"; DestDir: "{tmp}"; Flags: deleteafterinstall; Check: NeedsWebView2

[Icons]
Name: "{autoprograms}\Frey Project"; Filename: "{app}\frey-project.exe"
Name: "{autodesktop}\Frey Project"; Filename: "{app}\frey-project.exe"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Frey Project"; ValueData: """{app}\frey-project.exe"" --hidden"; Flags: uninsdeletevalue; Tasks: autostart

[Run]
Filename: "{tmp}\MicrosoftEdgeWebView2RuntimeInstallerX64.exe"; Parameters: "/silent /install"; StatusMsg: "{cm:Webview}"; Check: NeedsWebView2; Flags: waituntilterminated
Filename: "{app}\frey-project.exe"; Description: "{cm:LaunchProgram,Frey Project}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "powershell.exe"; Parameters: "-NoProfile -WindowStyle Hidden -Command ""Get-Process frey-project -ErrorAction SilentlyContinue | Where-Object {{ $_.Path -like '{app}\*' } | Stop-Process -Force"""; RunOnceId: "StopApp"; Flags: runhidden waituntilterminated

[Code]
const
  WebViewKey = 'SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}';

function HasWebViewVersion(Root: Integer): Boolean;
var
  Version: String;
begin
  Result := RegQueryStringValue(Root, WebViewKey, 'pv', Version) and (Version <> '') and (Version <> '0.0.0.0');
end;

function NeedsWebView2: Boolean;
begin
  Result := not (HasWebViewVersion(HKLM32) or HasWebViewVersion(HKLM64) or HasWebViewVersion(HKCU));
end;

// Atualizacao: o X so esconde o painel, entao o app pode estar rodando e travar o .exe.
// So encerra a copia desta pasta; downloads parciais sao retomados depois.
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  ResultCode: Integer;
begin
  Result := '';
  Exec('powershell.exe', '-NoProfile -WindowStyle Hidden -Command "Get-Process frey-project -ErrorAction SilentlyContinue | Where-Object { $_.Path -like ''' + ExpandConstant('{app}') + '\*'' } | Stop-Process -Force"', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
end;
