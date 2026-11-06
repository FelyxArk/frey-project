# Testa o link copiado no app real, com uma instancia de teste separada: pode rodar com o app instalado aberto.
# Usa o depurador do WebView, sem mouse nem teclado. Troca a area de transferencia durante o teste e restaura o texto no fim.
# Uso: powershell -NoProfile -STA -ExecutionPolicy Bypass -File Test-Clipboard.ps1 -OutDir pasta
param([Parameter(Mandatory)][string]$OutDir, [string]$Exe)
$ErrorActionPreference = 'Stop'
if ([Threading.Thread]::CurrentThread.ApartmentState -ne 'STA') { throw 'Rode com powershell -STA.' }
if (-not $Exe) { $Exe = Join-Path $env:LOCALAPPDATA 'frey-project-target\release\frey-project.exe' }
Add-Type -AssemblyName System.Windows.Forms
Remove-Item -LiteralPath $OutDir -Recurse -Force -ErrorAction SilentlyContinue
$dest = Join-Path $OutDir 'dest'; $cfg = Join-Path $OutDir 'config'; $web = Join-Path $OutDir 'webview'
New-Item -ItemType Directory -Force -Path $dest, $cfg, $web | Out-Null
[IO.File]::WriteAllText("$cfg\settings.json", ('{"idioma":"pt","so_baixar":true,"destino":' + (ConvertTo-Json $dest) + '}'), (New-Object Text.UTF8Encoding($false)))
[IO.File]::WriteAllText("$cfg\modelos.json", '{"abrirAoFinal":false,"modelos":[{"nome":"T","pastaRaiz":"{nome}","pastas":["a"],"pastaDestino":"a"}]}', (New-Object Text.UTF8Encoding($false)))
$results = New-Object System.Collections.ArrayList
function Check([string]$l, [bool]$p) { $line = '{0}  {1}' -f $(if ($p) { 'PASS' } else { 'FAIL' }), $l; [void]$results.Add($line); Write-Host $line }
$script:ws = $null; $script:proc = $null
function Eval([string]$expr) {
  $id = Get-Random
  $msg = ConvertTo-Json -Compress -Depth 6 @{ id = $id; method = 'Runtime.evaluate'; params = @{ expression = $expr; returnByValue = $true; awaitPromise = $true } }
  $seg = New-Object ArraySegment[byte] (,[Text.Encoding]::UTF8.GetBytes($msg))
  $script:ws.SendAsync($seg, 'Text', $true, [Threading.CancellationToken]::None).Wait()
  while ($true) {
    $buf = New-Object byte[] 1048576; $text = ''
    do { $r = $script:ws.ReceiveAsync((New-Object ArraySegment[byte] (,$buf)), [Threading.CancellationToken]::None); $r.Wait(); $text += [Text.Encoding]::UTF8.GetString($buf, 0, $r.Result.Count) } while (-not $r.Result.EndOfMessage)
    $o = $text | ConvertFrom-Json
    if ($o.id -eq $id) { return $o.result.result.value }
  }
}
function CopyText([string]$text) { foreach ($i in 1..10) { try { [Windows.Forms.Clipboard]::SetText($text); break } catch { Start-Sleep -Milliseconds 100 } }; Start-Sleep -Milliseconds 1300 }
function OpenPanel { [void](Eval 'window.__TAURI__.event.emit("panel-shown").then(()=>1)'); Start-Sleep -Milliseconds 700 }
function Rows { [int](Eval 'document.querySelectorAll("#rows .row").length') }
function FirstUrl { Eval '(document.querySelector("#rows .row__link")||{}).textContent||""' }
function Launch {
  $script:proc = Start-Process $Exe -ArgumentList '--hidden' -PassThru
  $tg = $null
  foreach ($i in 1..80) { Start-Sleep -Milliseconds 250; try { $tg = @(Invoke-RestMethod 'http://127.0.0.1:9339/json' -TimeoutSec 2 | Where-Object { $_.type -eq 'page' })[0]; if ($tg) { break } } catch {} }
  if (-not $tg) { throw 'Depurador do WebView nao respondeu.' }
  $script:ws = New-Object Net.WebSockets.ClientWebSocket
  $script:ws.ConnectAsync([Uri]$tg.webSocketDebuggerUrl, [Threading.CancellationToken]::None).Wait()
  Start-Sleep -Seconds 1
}
function Quit { if ($script:ws) { $script:ws.Dispose(); $script:ws = $null }; if ($script:proc -and -not $script:proc.HasExited) { Stop-Process -Id $script:proc.Id -Force; [void]$script:proc.WaitForExit(5000) }; Start-Sleep -Milliseconds 800 }

$saved = $null; $hadText = $false
try { if ([Windows.Forms.Clipboard]::ContainsText()) { $saved = [Windows.Forms.Clipboard]::GetText(); $hadText = $true } } catch {}
$env:FREY_PROJECT_CONFIG_DIR = $cfg
$env:FREY_PROJECT_TEST_INSTANCE = '1'
$env:WEBVIEW2_USER_DATA_FOLDER = $web
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9339 --remote-allow-origins=*'
$oldLink = 'https://drive.google.com/file/d/IdAntesDeAbrirOFreyProject0001/view'
$driveLink = 'https://drive.google.com/file/d/IdCopiadoAgoraNoFreyProject0002/view'
$ytLink = 'https://www.youtube.com/watch?v=exemploFreyProject'
try {
  CopyText $oldLink
  Launch
  OpenPanel
  Check 'link copiado antes de o app abrir nao entra' ((Rows) -eq 0)
  OpenPanel
  Check 'abrir de novo tambem nao traz o link antigo' ((Rows) -eq 0)

  CopyText $driveLink
  Check 'copiar nao muda nada ate abrir o painel' ((Rows) -eq 0)
  OpenPanel
  Check 'link novo fica pronto ao abrir' ((Rows) -eq 1 -and (FirstUrl) -eq $driveLink)
  Check 'avisa que adicionou' ((Eval 'document.getElementById("status").textContent') -match 'copiado')
  Check 'nao comeca a baixar sozinho' ((Eval 'document.getElementById("go").disabled') -eq $false -and (Eval 'document.querySelector("#rows .row__state").textContent') -eq 'Aguardando')
  OpenPanel
  Check 'mesma copia nao entra duas vezes' ((Rows) -eq 1)

  [void](Eval 'document.querySelector("#rows .row button").click()'); Start-Sleep -Milliseconds 300
  OpenPanel
  Check 'link removido nao volta sem copiar de novo' ((Rows) -eq 0)

  CopyText 'um texto qualquer https://example.com/pagina'
  OpenPanel
  Check 'texto e outros sites sao ignorados' ((Rows) -eq 0)
  CopyText 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA'
  OpenPanel
  Check 'codigo solto, como uma senha, e ignorado' ((Rows) -eq 0)
  [void](Eval 'document.getElementById("links").value=""')

  CopyText $driveLink
  OpenPanel
  CopyText $ytLink
  OpenPanel
  Check 'pendente fica e o novo entra junto' ((Rows) -eq 2)
  Check 'novo YouTube ganha a escolha de qualidade' ((Eval 'document.querySelectorAll("#rows select.row__quality").length') -eq 1)
  CopyText $driveLink
  OpenPanel
  Check 'link igual ao que ja esta na lista nao duplica' ((Rows) -eq 2)

  # Na aba Converter o link leva de volta ao Projeto
  [void](Eval '[...document.querySelectorAll("#rows .row button")].forEach(b=>b.click())'); Start-Sleep -Milliseconds 300
  [void](Eval 'document.getElementById("tabConvert").click()'); Start-Sleep -Milliseconds 300
  CopyText $ytLink
  OpenPanel
  Check 'link novo volta para a aba Projeto' ((Eval 'document.getElementById("viewProject").hidden') -eq $false -and (Rows) -eq 1)

  # Linha concluida sai quando chega link novo
  if ($env:FREY_TEST_DRIVE_FILE) {
    [void](Eval '[...document.querySelectorAll("#rows .row button")].forEach(b=>b.click())'); Start-Sleep -Milliseconds 300
    CopyText $env:FREY_TEST_DRIVE_FILE
    OpenPanel
    [void](Eval 'document.getElementById("go").click()')
    Start-Sleep -Milliseconds 400
    foreach ($i in 1..120) { if ((Eval 'document.getElementById("go").disabled') -eq $false) { break }; Start-Sleep -Milliseconds 250 }
    Check 'download real terminou' ((Eval 'document.querySelector("#rows .row__state").textContent') -match 'Conclu')
    CopyText $ytLink
    OpenPanel
    Check 'concluido sai e o novo fica' ((Rows) -eq 1 -and (FirstUrl) -eq $ytLink)
    [void](Eval '[...document.querySelectorAll("#rows .row button")].forEach(b=>b.click())')
  }

  # Desligar a opcao
  [void](Eval 'document.getElementById("tabSettings").click()'); Start-Sleep -Milliseconds 300
  Check 'opcao vem ligada' ((Eval 'document.getElementById("sDetectLink").checked') -eq $true)
  [void](Eval 'document.getElementById("sDetectLink").click()'); Start-Sleep -Milliseconds 500
  Check 'desligar fica salvo' ((Get-Content -LiteralPath "$cfg\settings.json" -Raw) -match '"ignorar_link_copiado":\s*true')
  CopyText $driveLink
  OpenPanel
  Check 'desligada, nao adiciona nada' ((Rows) -eq 0)
  Quit
  CopyText $driveLink
  Launch
  CopyText $ytLink
  OpenPanel
  Check 'desligada continua assim depois de reiniciar' ((Rows) -eq 0)
  [void](Eval 'document.getElementById("tabSettings").click()'); Start-Sleep -Milliseconds 200
  Check 'caixa aparece desmarcada depois de reiniciar' ((Eval 'document.getElementById("sDetectLink").checked') -eq $false)
} catch {
  Check "erro: $($_.Exception.Message)" $false
} finally {
  Quit
  foreach ($n in 'FREY_PROJECT_CONFIG_DIR', 'FREY_PROJECT_TEST_INSTANCE', 'WEBVIEW2_USER_DATA_FOLDER', 'WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS') { Remove-Item "Env:$n" -ErrorAction SilentlyContinue }
  foreach ($i in 1..10) { try { if ($hadText) { [Windows.Forms.Clipboard]::SetText($saved) } else { [Windows.Forms.Clipboard]::Clear() }; break } catch { Start-Sleep -Milliseconds 100 } }
}
"RESUMO PASS=$(@($results | Where-Object { $_ -like 'PASS*' }).Count) FAIL=$(@($results | Where-Object { $_ -like 'FAIL*' }).Count)"
if (@($results | Where-Object { $_ -like 'FAIL*' }).Count) { exit 1 }
