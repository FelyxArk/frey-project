# Testa o que so existe na janela real: arrastar do Explorer, clique duplo fora, iniciar sem foco.
# Usa uma instancia de teste separada e MOVE O MOUSE DE VERDADE por cerca de 30 segundos.
# Antes de cada clique confere qual janela esta sob o cursor; se nao for a do teste, nao clica.
# Uso: powershell -NoProfile -STA -ExecutionPolicy Bypass -File Test-Panel.ps1 -OutDir pasta
param([Parameter(Mandatory)][string]$OutDir, [string]$Exe)
$ErrorActionPreference = 'Stop'
if ([Threading.Thread]::CurrentThread.ApartmentState -ne 'STA') { throw 'Rode com powershell -STA.' }
if (-not $Exe) { $Exe = Join-Path $env:LOCALAPPDATA 'frey-project-target\release\frey-project.exe' }
Add-Type -AssemblyName System.Windows.Forms, System.Drawing, UIAutomationClient, UIAutomationTypes
Add-Type @'
using System; using System.Text; using System.Runtime.InteropServices;
public static class P {
  [StructLayout(LayoutKind.Sequential)] public struct R { public int L, T, Rt, B; }
  [StructLayout(LayoutKind.Sequential)] public struct Pt { public int X, Y; }
  public delegate bool EP(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EP f, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out R r);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr a, int x, int y, int cx, int cy, uint f);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Pt p);
  [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr h, uint f);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out Pt p);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, UIntPtr e);
  [DllImport("user32.dll")] public static extern void keybd_event(byte k, byte s, uint f, UIntPtr e);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  public static IntPtr Panel(uint pid) { return Find(pid, "Frey Project", null); }
  public static IntPtr Dialog(uint pid) { return Find(pid, null, "#32770"); }
  static IntPtr Find(uint pid, string title, string cls) { IntPtr f = IntPtr.Zero; EnumWindows((h,l)=>{ uint p; GetWindowThreadProcessId(h,out p); if(p!=pid) return true; var s=new StringBuilder(64); GetWindowText(h,s,64); var c=new StringBuilder(64); GetClassName(h,c,64); if((title==null||s.ToString()==title)&&(cls==null||c.ToString()==cls)){f=h;return false;} return true;}, IntPtr.Zero); return f; }
  public static string Name(IntPtr h) { var c=new StringBuilder(64); GetClassName(h,c,64); var s=new StringBuilder(64); GetWindowText(h,s,64); return c+"/"+s; }
  public static IntPtr Top(int x, int y) { Pt p; p.X=x; p.Y=y; return GetAncestor(WindowFromPoint(p),2); }
}
'@
Remove-Item -LiteralPath $OutDir -Recurse -Force -ErrorAction SilentlyContinue
$cfg = Join-Path $OutDir 'config'; $web = Join-Path $OutDir 'webview'; $src = Join-Path $OutDir 'origem'; $folder = Join-Path $src 'pasta-solta'
New-Item -ItemType Directory -Force -Path $cfg, $web, $folder | Out-Null
$file = Join-Path $src 'arrastado.mp4'; [IO.File]::WriteAllBytes($file, (New-Object byte[] 1024))
$enc = New-Object Text.UTF8Encoding($false)
[IO.File]::WriteAllText("$cfg\settings.json", '{"idioma":"pt","so_baixar":true,"atalho":"ctrl+alt+shift+f11"}', $enc)
[IO.File]::WriteAllText("$cfg\modelos.json", '{"abrirAoFinal":false,"modelos":[{"nome":"T","pastaRaiz":"{nome}","pastas":["a"],"pastaDestino":"a"}]}', $enc)
$env:FREY_PROJECT_CONFIG_DIR = $cfg; $env:FREY_PROJECT_TEST_INSTANCE = '1'; $env:WEBVIEW2_USER_DATA_FOLDER = $web
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9342 --remote-allow-origins=*'
$script:fail = 0; $script:pass = 0
function Check([string]$l, [bool]$p) { if ($p) { $script:pass++ } else { $script:fail++ }; Write-Host ('{0}  {1}' -f $(if ($p) { 'PASS' } else { 'FAIL' }), $l) }
$script:ws = $null; $script:proc = $null; $script:h = [IntPtr]::Zero
function Eval([string]$expr) {
  $id = Get-Random
  $msg = ConvertTo-Json -Compress -Depth 6 @{ id = $id; method = 'Runtime.evaluate'; params = @{ expression = $expr; returnByValue = $true; awaitPromise = $false } }
  $seg = New-Object ArraySegment[byte] (,[Text.Encoding]::UTF8.GetBytes($msg))
  $script:ws.SendAsync($seg, 'Text', $true, [Threading.CancellationToken]::None).Wait()
  while ($true) {
    $buf = New-Object byte[] 1048576; $text = ''
    do { $r = $script:ws.ReceiveAsync((New-Object ArraySegment[byte] (,$buf)), [Threading.CancellationToken]::None); $r.Wait(); $text += [Text.Encoding]::UTF8.GetString($buf, 0, $r.Result.Count) } while (-not $r.Result.EndOfMessage)
    $o = $text | ConvertFrom-Json
    if ($o.id -eq $id) { return $o.result.result.value }
  }
}
function Launch([string[]]$launchArgs) {
  $script:proc = if ($launchArgs) { Start-Process $Exe -ArgumentList $launchArgs -PassThru } else { Start-Process $Exe -PassThru }
  $tg = $null
  foreach ($i in 1..80) { Start-Sleep -Milliseconds 250; try { $tg = @(Invoke-RestMethod 'http://127.0.0.1:9342/json' -TimeoutSec 2 | Where-Object { $_.type -eq 'page' })[0]; if ($tg) { break } } catch {} }
  if (-not $tg) { throw 'Depurador do WebView nao respondeu.' }
  $script:ws = New-Object Net.WebSockets.ClientWebSocket
  $script:ws.ConnectAsync([Uri]$tg.webSocketDebuggerUrl, [Threading.CancellationToken]::None).Wait()
  Start-Sleep -Seconds 1
  $script:h = [P]::Panel([uint32]$script:proc.Id)
}
function Quit { if ($script:ws) { $script:ws.Dispose(); $script:ws = $null }; if ($script:proc -and -not $script:proc.HasExited) { Stop-Process -Id $script:proc.Id -Force; [void]$script:proc.WaitForExit(5000) }; Start-Sleep -Milliseconds 800 }
function Rect($h) { $r = New-Object P+R; [void][P]::GetWindowRect($h, [ref]$r); $r }
# Mostra sem ativar, numa posicao que nao cobre a origem nem o alvo dos cliques.
function ShowPanel { [void][P]::SetWindowPos($script:h, [IntPtr]::new(-1), 520, 200, 0, 0, 0x0015); [void][P]::ShowWindow($script:h, 4); Start-Sleep -Milliseconds 500 }
# O atalho e so desta instancia e nao digita nada; passa pelo mesmo caminho do uso normal.
function Hotkey { foreach ($k in 0x11, 0x12, 0x10, 0x7A) { [P]::keybd_event($k, 0, 0, [UIntPtr]::Zero) }; foreach ($k in 0x7A, 0x10, 0x12, 0x11) { [P]::keybd_event($k, 0, 2, [UIntPtr]::Zero) }; Pump 700 }
function Pump([int]$ms) { foreach ($i in 1..[Math]::Max(1, [int]($ms / 25))) { [Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 25 } }
function Press { [P]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero); Pump 50; [P]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero) }
function ClickOn($expected, [int]$x, [int]$y) {
  foreach ($i in 1..40) { if ([P]::Top($x, $y) -eq $expected) { break }; Pump 50 }
  if ([P]::Top($x, $y) -ne $expected) { $pr = Rect $script:h; throw "Outra janela ($([P]::Name([P]::Top($x, $y))) hwnd $([P]::Top($x, $y))) sob o cursor em $x,$y. Painel de teste $($script:h) em $($pr.L),$($pr.T) a $($pr.Rt),$($pr.B). Nada foi clicado." }
  [void][P]::SetCursorPos($x, $y); Pump 60; Press
}
function DragTo($from, [int]$fx, [int]$fy, [string]$element) {
  $r = Rect $script:h
  $box = (Eval "var e=document.getElementById('$element').getBoundingClientRect();JSON.stringify({x:e.left+e.width/2,y:e.top+e.height/2})") | ConvertFrom-Json
  $tx = $r.L + 8 + [int]$box.x; $ty = $r.T + [int]$box.y
  foreach ($i in 1..60) { if ([P]::Top($fx, $fy) -eq $from) { break }; Pump 50 }
  if ([P]::Top($fx, $fy) -ne $from) { throw "Outra janela ($([P]::Name([P]::Top($fx, $fy)))) sobre a origem do arraste em $fx,$fy. Nada foi clicado." }
  [void][P]::SetCursorPos($fx, $fy); Pump 150
  [P]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero); Pump 200
  foreach ($i in 1..25) { [void][P]::SetCursorPos($fx + [int](($tx - $fx) * $i / 25), $fy + [int](($ty - $fy) * $i / 25)); Pump 25 }
  Pump 400
  $ok = [P]::Top($tx, $ty) -eq $script:h
  if (-not $ok) { [void][P]::SetCursorPos($fx, $fy); Pump 200 }
  [P]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero); Pump 800
  $ok
}

$old = New-Object P+Pt; [void][P]::GetCursorPos([ref]$old)
$shell = New-Object -ComObject Shell.Application
$explorer = $null; $form = $null
try {
  # 1. Iniciar com o Windows: nao aparece na tela nem pega o foco.
  $before = [P]::GetForegroundWindow()
  $script:proc = Start-Process $Exe -ArgumentList '--hidden' -PassThru
  $seen = $false; $took = $false; $wnd = [IntPtr]::Zero
  foreach ($i in 1..120) {
    if ($wnd -eq [IntPtr]::Zero) { $wnd = [P]::Panel([uint32]$script:proc.Id) }
    if ($wnd -ne [IntPtr]::Zero) {
      if ([P]::GetForegroundWindow() -eq $wnd) { $took = $true }
      if ([P]::IsWindowVisible($wnd)) { $r = Rect $wnd; if ($r.L -lt [Windows.Forms.SystemInformation]::VirtualScreen.Right - 16) { if (-not $seen) { Write-Host "  visivel em $($r.L),$($r.T) a $($r.Rt),$($r.B) na leitura $i" }; $seen = $true } }
    }
    Start-Sleep -Milliseconds 25
  }
  Check 'ao iniciar nao pega o foco' (-not $took)
  Check 'ao iniciar nao aparece na tela' (-not $seen)
  Check 'ao iniciar fica escondido' ($wnd -ne [IntPtr]::Zero -and -not [P]::IsWindowVisible($wnd))
  Stop-Process -Id $script:proc.Id -Force; [void]$script:proc.WaitForExit(5000); Start-Sleep -Seconds 1

  # 2. Arrastar do Explorer de verdade, com o app iniciado como no Windows.
  Launch @('--hidden')
  Start-Process explorer.exe -ArgumentList "`"$src`""
  $wanted = ([Uri]$src).AbsoluteUri; $window = $null
  foreach ($i in 1..40) { Start-Sleep -Milliseconds 250; $window = @($shell.Windows() | Where-Object { try { $_.LocationURL -ieq $wanted } catch { $false } })[0]; if ($window) { break } }
  if (-not $window) { throw 'Explorer do teste nao abriu.' }
  $explorer = $window; $eh = [IntPtr]::new([int64]$explorer.HWND)
  if (@($shell.Windows() | Where-Object { try { $_.HWND -eq $explorer.HWND } catch { $false } }).Count -ne 1) { throw 'O Explorer abriu como aba de uma janela sua. Teste parado.' }
  [void][P]::SetWindowPos($eh, [IntPtr]::new(-1), 10, 20, 490, 480, 0x0040); Start-Sleep -Seconds 2
  $root = [Windows.Automation.AutomationElement]::FromHandle($eh)
  $items = $root.FindAll([Windows.Automation.TreeScope]::Descendants, (New-Object Windows.Automation.PropertyCondition ([Windows.Automation.AutomationElement]::ControlTypeProperty), ([Windows.Automation.ControlType]::ListItem)))
  $spot = @{}
  foreach ($item in $items) { $b = $item.Current.BoundingRectangle; if ($item.Current.Name -like 'arrastado*') { $spot.file = $b } elseif ($item.Current.Name -eq 'pasta-solta') { $spot.folder = $b } }
  if (-not $spot.file -or -not $spot.folder) { throw 'Itens nao encontrados no Explorer.' }
  ShowPanel
  [void](Eval "document.getElementById('tabConvert').click();1"); Start-Sleep -Milliseconds 400
  $ok = DragTo $eh ([int]$spot.file.Left + 40) ([int]($spot.file.Top + $spot.file.Height / 2)) 'cAdd'
  Check 'cursor chegou na area tracejada' $ok
  Check 'arrastar do Explorer adiciona o video' ((Eval "document.querySelectorAll('#cRows .row').length") -eq 1)
  Check 'trocar para o Explorer nao esconde o painel' ([P]::IsWindowVisible($script:h))

  [void](Eval "document.getElementById('tabProject').click();1"); Start-Sleep -Milliseconds 400
  $ok = DragTo $eh ([int]$spot.folder.Left + 40) ([int]($spot.folder.Top + $spot.folder.Height / 2)) 'dest'
  Check 'arrastar pasta na aba Projeto vira o destino' ($ok -and (Eval "document.getElementById('dest').textContent") -eq $folder)
  [void]$explorer.Quit(); $explorer = $null; Start-Sleep -Milliseconds 500
  Quit
  Launch @()

  # 3. Clique duplo fora, numa janela do proprio teste.
  $form = New-Object Windows.Forms.Form; $form.Text = 'alvo'; $form.TopMost = $true; $form.StartPosition = 'Manual'; $form.ShowInTaskbar = $false
  $area = [Windows.Forms.Screen]::PrimaryScreen.WorkingArea; $form.Location = New-Object Drawing.Point(280, ($area.Bottom - 130)); $form.Size = New-Object Drawing.Size(200, 120); $form.Show(); Pump 400
  $fh = $form.Handle; $fx = 380; $fy = $area.Bottom - 70
  $gap = [Windows.Forms.SystemInformation]::DoubleClickTime
  ClickOn $fh $fx $fy; Pump ($gap + 300)
  Check 'um clique fora nao fecha' ([P]::IsWindowVisible($script:h))
  ClickOn $fh $fx $fy; Pump ($gap + 300); ClickOn $fh $fx $fy; Pump ($gap + 300)
  Check 'dois cliques lentos nao fecham' ([P]::IsWindowVisible($script:h))
  $r = Rect $script:h; $box = (Eval "var e=document.getElementById('destLabel').getBoundingClientRect();JSON.stringify({x:e.left+e.width/2,y:e.top+e.height/2})") | ConvertFrom-Json
  $ix = $r.L + 8 + [int]$box.x; $iy = $r.T + [int]$box.y
  ClickOn $script:h $ix $iy; Pump 60; ClickOn $script:h $ix $iy; Pump 500
  Check 'clique duplo dentro nao fecha' ([P]::IsWindowVisible($script:h))
  $before = Rect $script:h; $box = (Eval "var e=document.getElementById('hotkey').getBoundingClientRect();JSON.stringify({x:e.left-30,y:e.top+e.height/2})") | ConvertFrom-Json
  ClickOn $script:h ($before.L + 8 + [int]$box.x) ($before.T + [int]$box.y); Pump 60; ClickOn $script:h ($before.L + 8 + [int]$box.x) ($before.T + [int]$box.y); Pump 500
  $after = Rect $script:h
  Check 'clique duplo na barra do topo nao maximiza' (($after.Rt - $after.L) -eq ($before.Rt - $before.L) -and [P]::IsWindowVisible($script:h))
  [void][P]::SetWindowPos($script:h, [IntPtr]::new(-1), $before.L, $before.T, 0, 0, 0x0015)
  ClickOn $fh $fx $fy; Pump 60; ClickOn $fh $fx $fy; Pump 500
  Check 'clique duplo fora fecha' (-not [P]::IsWindowVisible($script:h))
  Hotkey
  Check 'atalho reabre depois de fechar' ([P]::IsWindowVisible($script:h))
  [void](Eval "document.getElementById('dest').click();1")
  $dialog = [IntPtr]::Zero; foreach ($i in 1..40) { Pump 100; $dialog = [P]::Dialog([uint32]$script:proc.Id); if ($dialog -ne [IntPtr]::Zero) { break } }
  Check 'caixa de pasta abriu' ($dialog -ne [IntPtr]::Zero)
  [void][P]::SetWindowPos($dialog, [IntPtr]::Zero, 620, 40, 0, 0, 0x0015); Pump 300
  ClickOn $fh $fx $fy; Pump 60; ClickOn $fh $fx $fy; Pump 500
  Check 'com a caixa aberta o clique duplo nao fecha' ([P]::IsWindowVisible($script:h))
  [void][P]::PostMessage($dialog, 0x10, [IntPtr]::Zero, [IntPtr]::Zero)
  foreach ($i in 1..60) { Pump 100; if ([P]::Dialog([uint32]$script:proc.Id) -eq [IntPtr]::Zero) { break } }
  Write-Host "  caixa fechada apos $($i * 100) ms; visivel=$([P]::IsWindowVisible($script:h)); frente=$([P]::Name([P]::GetForegroundWindow()))"; Pump 500
  ClickOn $fh $fx $fy; Pump 60; ClickOn $fh $fx $fy; Pump 500
  Check 'fechada a caixa, o clique duplo volta a fechar' (-not [P]::IsWindowVisible($script:h))
  Check 'fechar nao encerra o app' (-not $script:proc.HasExited)
  Check 'o pino nao existe mais' ((Eval "document.getElementById('pin')===null") -eq $true)
  Quit

  # 4. Abrir normalmente: aparece na tela visivel, com foco. Pega o teclado por 2 segundos.
  Launch @()
  $r = Rect $script:h; $screen = [Windows.Forms.SystemInformation]::VirtualScreen
  Check 'ao abrir aparece dentro da tela' ([P]::IsWindowVisible($script:h) -and $r.L -ge $screen.Left -and $r.T -ge $screen.Top -and $r.Rt -le $screen.Right -and $r.B -le $screen.Bottom)
  foreach ($i in 1..20) { if ([P]::GetForegroundWindow() -eq $script:h) { break }; Pump 100 }
  if ([P]::GetForegroundWindow() -ne $script:h) { Write-Host "  na frente: $([P]::Name([P]::GetForegroundWindow()))" }
  Check 'ao abrir fica com o foco' ([P]::GetForegroundWindow() -eq $script:h)
  Quit
} catch {
  Check "erro: $($_.Exception.Message)" $false
} finally {
  if ($form) { $form.Close(); $form.Dispose() }
  if ($explorer) { try { [void]$explorer.Quit() } catch {} }
  [void][P]::SetCursorPos($old.X, $old.Y)
  Quit
}
Write-Host "RESUMO PASS=$script:pass FAIL=$script:fail"
if ($script:fail) { exit 1 }
