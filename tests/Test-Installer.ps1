# Testa o instalador de verdade em pasta temporaria, sem ffmpeg, yt-dlp ou Deno no PATH, e desinstala no fim.
# Prova que o app instalado usa as copias embutidas. Usa o depurador do WebView: nao mexe em mouse nem teclado.
# Baixa o audio de um video aberto da Blender Foundation (cerca de 30 MB).
param(
  [Parameter(Mandatory)][string]$OutDir,
  [string]$Setup
)
$ErrorActionPreference = 'Stop'
if (-not $Setup) { $Setup = (Get-ChildItem (Join-Path $PSScriptRoot '..\dist\Frey-Project-Setup-*.exe') | Sort-Object LastWriteTime | Select-Object -Last 1).FullName }
if (Get-Process frey-project -ErrorAction SilentlyContinue) { throw 'O Frey Project esta aberto. Feche pela bandeja (Sair) antes de testar.' }
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
if ((Get-ItemProperty $runKey -ErrorAction SilentlyContinue).'Frey Project') { throw 'Ja existe inicio automatico do Frey Project; nao vou mexer.' }
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{6E0A1F3C-0B5D-4E2A-9F67-3C1D7A5B8E42}_is1'
if (Test-Path $uninstallKey) { throw 'O Frey Project ja esta instalado; este teste nao substitui uma instalacao real.' }
# O instalador usa o mesmo identificador e os mesmos atalhos: desinstalar aqui estragaria uma instalacao real.
foreach ($real in "$env:LOCALAPPDATA\Programs\Frey Project", "$([Environment]::GetFolderPath('Desktop'))\Frey Project.lnk", "$([Environment]::GetFolderPath('Programs'))\Frey Project.lnk") {
  if (Test-Path -LiteralPath $real) { throw "Instalacao real encontrada ($real). Rode este teste em um PC ou maquina virtual sem o Frey Project." }
}
if (Get-Process -Name 'Frey-Project-Setup*' -ErrorAction SilentlyContinue) { throw 'Ha um instalador do Frey Project em andamento.' }

Remove-Item -LiteralPath $OutDir -Recurse -Force -ErrorAction SilentlyContinue
$app = Join-Path $OutDir 'instalado'; $dest = Join-Path $OutDir 'dest'; $cfg = Join-Path $OutDir 'config'
New-Item -ItemType Directory -Force -Path $dest, $cfg | Out-Null
$results = New-Object System.Collections.ArrayList
function Check([string]$l, [bool]$p) { [void]$results.Add(('{0}  {1}' -f $(if ($p) { 'PASS' } else { 'FAIL' }), $l)) }
$script:ws = $null; $script:proc = $null
function Send([string]$method, $params) {
  $id = Get-Random
  $msg = ConvertTo-Json -Compress -Depth 6 @{ id = $id; method = $method; params = $params }
  $seg = New-Object ArraySegment[byte] (,[Text.Encoding]::UTF8.GetBytes($msg))
  $script:ws.SendAsync($seg, 'Text', $true, [Threading.CancellationToken]::None).Wait()
  while ($true) {
    $buf = New-Object byte[] 1048576; $text = ''
    do { $r = $script:ws.ReceiveAsync((New-Object ArraySegment[byte] (,$buf)), [Threading.CancellationToken]::None); $r.Wait(); $text += [Text.Encoding]::UTF8.GetString($buf, 0, $r.Result.Count) } while (-not $r.Result.EndOfMessage)
    $o = $text | ConvertFrom-Json
    if ($o.id -eq $id) { return $o.result }
  }
}
function Eval([string]$expr) { (Send 'Runtime.evaluate' @{ expression = $expr; returnByValue = $true; awaitPromise = $true }).result.value }

$setupInfo = Get-Item -LiteralPath $Setup
[void]$results.Add(('INFO  instalador: {0} ({1:N0} MB)' -f $setupInfo.Name, ($setupInfo.Length / 1MB)))
$sw = [Diagnostics.Stopwatch]::StartNew()
$p = Start-Process -FilePath $Setup -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/DIR=`"$app`"", '/TASKS="autostart"', '/LANG=ptBR', "/LOG=`"$OutDir\setup.log`"" -Wait -PassThru
[void]$results.Add("INFO  instalacao: $([int]$sw.Elapsed.TotalSeconds)s, codigo $($p.ExitCode)")
$uninstaller = Join-Path $app 'unins000.exe'
$savedPath = $env:Path
try {
  Check 'instalador terminou sem erro' ($p.ExitCode -eq 0)
  foreach ($file in 'frey-project.exe', 'bin\ffmpeg.exe', 'bin\ffprobe.exe', 'bin\yt-dlp.exe', 'bin\deno.exe', 'NOTICES.txt', 'licenses\FFmpeg-LICENSE.txt') { Check "instalou $file" (Test-Path -LiteralPath (Join-Path $app $file)) }
  Check 'WebView2 presente nao e reinstalado nem copiado' (-not ((Get-Content "$OutDir\setup.log" -Raw) -match 'WebView2RuntimeInstaller.*\.exe.*Filename|Executing.*WebView2'))
  $run = (Get-ItemProperty $runKey).'Frey Project'
  Check "inicio com o Windows usa --hidden ($run)" ($run -eq "`"$app\frey-project.exe`" --hidden")
  Check 'atalho no Menu Iniciar' (Test-Path -LiteralPath "$([Environment]::GetFolderPath('Programs'))\Frey Project.lnk")
  Check 'sem atalho na Area de Trabalho, como pedido no teste' (-not (Test-Path -LiteralPath "$([Environment]::GetFolderPath('Desktop'))\Frey Project.lnk"))
  $reg = Get-ItemProperty $uninstallKey
  Check 'sem administrador: registro so do usuario' ($reg.DisplayName -eq 'Frey Project')

  # PC "limpo": PATH so com o Windows. Nada de winget, cargo ou Python.
  $env:Path = "$env:SystemRoot\System32;$env:SystemRoot;$env:SystemRoot\System32\WindowsPowerShell\v1.0"
  Check 'PATH de teste nao enxerga as ferramentas' (-not (Get-Command ffmpeg, ffprobe, yt-dlp, deno -ErrorAction SilentlyContinue))
  $env:FREY_PROJECT_CONFIG_DIR = $cfg
  $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9334 --remote-allow-origins=*'
  [IO.File]::WriteAllText("$cfg\settings.json", ('{"idioma":"pt","so_baixar":true,"destino":' + (ConvertTo-Json $dest) + '}'), (New-Object Text.UTF8Encoding($false)))
  $script:proc = Start-Process "$app\frey-project.exe" -ArgumentList '--hidden' -PassThru
  $tg = $null
  foreach ($i in 1..60) { Start-Sleep -Milliseconds 250; try { $tg = @(Invoke-RestMethod 'http://127.0.0.1:9334/json' -TimeoutSec 2 | Where-Object { $_.type -eq 'page' })[0]; if ($tg) { break } } catch {} }
  $script:ws = New-Object Net.WebSockets.ClientWebSocket
  $script:ws.ConnectAsync([Uri]$tg.webSocketDebuggerUrl, [Threading.CancellationToken]::None).Wait()
  Start-Sleep -Seconds 1
  $tools = Eval 'window.__TAURI__.core.invoke("convert_tools").then(JSON.stringify)' | ConvertFrom-Json
  Check "app acha o ffmpeg embutido (GPU: $($tools.gpu))" ($tools.ffmpeg -eq $true)

  # YouTube real: yt-dlp + Deno + ffmpeg, todos da pasta bin
  [void](Eval '(async()=>{const d=new DataTransfer();d.setData("text","https://www.youtube.com/watch?v=aqz-KE-bpKQ");document.getElementById("links").dispatchEvent(new ClipboardEvent("paste",{clipboardData:d,bubbles:true,cancelable:true}));})()')
  Start-Sleep -Milliseconds 700
  [void](Eval 'var q=document.querySelector("#rows select.row__quality"); q.value="audio"; q.dispatchEvent(new Event("change",{bubbles:true}));')
  [void](Eval 'document.getElementById("go").click()')
  $seen = @{}; $t0 = Get-Date
  while (((Get-Date) - $t0).TotalSeconds -lt 240) {
    Start-Sleep -Milliseconds 300
    Get-CimInstance Win32_Process -Filter "Name='yt-dlp.exe' OR Name='ffmpeg.exe' OR Name='ffprobe.exe' OR Name='deno.exe'" | ForEach-Object { if ($_.ExecutablePath) { $seen[$_.ExecutablePath] = 1 } }
    if ((Eval 'document.getElementById("go").disabled') -eq $false -and ((Get-Date) - $t0).TotalSeconds -gt 2) { break }
  }
  $status = Eval 'document.getElementById("status").textContent'
  [void]$results.Add("INFO  status do YouTube: $status | linha: $(Eval 'document.querySelector(''#rows .row__state'').textContent')")
  [void]$results.Add("INFO  executaveis vistos: $(($seen.Keys | ForEach-Object { $_.Replace($app, '<app>') }) -join ', ')")
  $ytSeen = @($seen.Keys | Where-Object { $_ -like '*yt-dlp.exe' })
  Check 'yt-dlp usado e o embutido' ($ytSeen.Count -ge 1 -and @($ytSeen | Where-Object { $_ -notlike "$app\*" }).Count -eq 0)
  Check 'nenhuma ferramenta de fora da pasta do app' (@($seen.Keys | Where-Object { $_ -notlike "$app\*" }).Count -eq 0)
  $audio = @(Get-ChildItem -LiteralPath $dest -File | Where-Object Extension -eq '.m4a')
  Check 'audio do YouTube baixado' ($audio.Count -eq 1)
  if ($audio.Count -eq 1) {
    $duration = [double](& "$app\bin\ffprobe.exe" -v error -show_entries format=duration -of csv=p=0 $audio[0].FullName)
    Check "audio inteiro, lido pelo ffprobe embutido (${duration}s)" ($duration -gt 600)
    [void]$results.Add(('INFO  audio: {0:N1} MB' -f ($audio[0].Length / 1MB)))

    # Conversor real com o ffmpeg embutido
    $json = ConvertTo-Json -Compress @{ files = @(@{ id = 9001; path = $audio[0].FullName }); ext = 'wav'; mode = 'auto'; crf = 22; useGpu = $false; outDir = '' }
    [void](Eval "window.__TAURI__.core.invoke('start_convert',{request:$json}).then(()=>1)")
    foreach ($i in 1..120) { Start-Sleep -Milliseconds 500; if (-not (Get-CimInstance Win32_Process -Filter "Name='ffmpeg.exe'")) { if ($i -gt 2) { break } } }
    Start-Sleep -Seconds 1
    $wav = @(Get-ChildItem -LiteralPath $dest -File | Where-Object Extension -eq '.wav')
    Check 'conversor funciona sem ffmpeg no PATH' ($wav.Count -eq 1 -and $wav[0].Length -gt 1000000)
  }
} catch {
  Check "erro: $($_.Exception.Message)" $false
} finally {
  if ($script:ws) { $script:ws.Dispose() }
  # O desinstalador precisa encerrar sozinho a copia que ficou aberta.
  $env:Path = $savedPath
  Remove-Item Env:FREY_PROJECT_CONFIG_DIR, Env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS -ErrorAction SilentlyContinue
  if (Test-Path -LiteralPath $uninstaller) {
    $u = Start-Process $uninstaller -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART' -Wait -PassThru
    foreach ($i in 1..40) { if (-not (Test-Path -LiteralPath "$app\frey-project.exe")) { break }; Start-Sleep -Milliseconds 250 }
    Start-Sleep -Seconds 2
    Check 'desinstalador terminou sem erro' ($u.ExitCode -eq 0)
    Check 'desinstalar encerra o app aberto' ($script:proc.HasExited)
    Check 'desinstalar remove os arquivos' (-not (Test-Path -LiteralPath "$app\frey-project.exe") -and -not (Test-Path -LiteralPath "$app\bin"))
    Check 'desinstalar remove o inicio com o Windows' (-not (Get-ItemProperty $runKey -ErrorAction SilentlyContinue).'Frey Project')
    Check 'desinstalar remove o atalho' (-not (Test-Path -LiteralPath "$([Environment]::GetFolderPath('Programs'))\Frey Project.lnk"))
    Check 'desinstalar limpa o registro' (-not (Test-Path $uninstallKey))
    Check 'desinstalar nao toca nos arquivos baixados' (@(Get-ChildItem -LiteralPath $dest -File).Count -ge 1)
  }
  if ($script:proc -and -not $script:proc.HasExited) { Stop-Process -Id $script:proc.Id -Force }
  Remove-Item -LiteralPath $dest -Recurse -Force -ErrorAction SilentlyContinue
}
$results | ForEach-Object { $_ }
if (@($results | Where-Object { $_ -like 'FAIL*' }).Count) { exit 1 }
