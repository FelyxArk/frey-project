# Testa no app real: nao reabrir pasta ja aberta, inicio do download e formatos de audio.
# Roda pelo depurador do WebView, com configuracao temporaria, sem mouse nem teclado do Windows.
# Abre janelas do Explorer em pastas temporarias e fecha ao final.
# Downloads reais: audio de um filme aberto da Blender Foundation e, se definidos,
# FREY_TEST_DRIVE_FILE (arquivo pequeno) e FREY_TEST_DRIVE_BIG (arquivo grande: baixa so o inicio e cancela).
param(
  [Parameter(Mandatory)][string]$OutDir,
  [string]$Exe,
  [string]$Url = 'https://www.youtube.com/watch?v=aqz-KE-bpKQ'
)
$ErrorActionPreference = 'Stop'
if (-not $Exe) { $Exe = Join-Path $env:LOCALAPPDATA 'frey-project-target\release\frey-project.exe' }
if (Get-Process frey-project -ErrorAction SilentlyContinue) { throw 'O Frey Project esta aberto. Encerre antes de testar.' }
Remove-Item -LiteralPath $OutDir -Recurse -Force -ErrorAction SilentlyContinue
$dest = Join-Path $OutDir 'dest'; $other = Join-Path $OutDir 'outra'; $cfg = Join-Path $OutDir 'config'; $big = Join-Path $OutDir 'grande'
New-Item -ItemType Directory -Force -Path $dest, $other, $cfg, $big | Out-Null
[IO.File]::WriteAllText("$cfg\settings.json", ('{"idioma":"pt","so_baixar":true,"destino":' + (ConvertTo-Json $dest) + '}'), (New-Object Text.UTF8Encoding($false)))
$env:FREY_PROJECT_CONFIG_DIR = $cfg
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9337 --remote-allow-origins=*'
$results = New-Object System.Collections.ArrayList
function Check([string]$l, [bool]$p) { $line = '{0}  {1}' -f $(if ($p) { 'PASS' } else { 'FAIL' }), $l; [void]$results.Add($line); Write-Host $line }
function Info([string]$l) { $line = "INFO  $l"; [void]$results.Add($line); Write-Host $line }
$script:ws = $null
function Eval([string]$expr) {
  $id = Get-Random
  $msg = ConvertTo-Json -Compress -Depth 6 @{ id = $id; method = 'Runtime.evaluate'; params = @{ expression = $expr; returnByValue = $true } }
  $seg = New-Object ArraySegment[byte] (,[Text.Encoding]::UTF8.GetBytes($msg))
  $script:ws.SendAsync($seg, 'Text', $true, [Threading.CancellationToken]::None).Wait()
  while ($true) {
    $buf = New-Object byte[] 1048576; $text = ''
    do { $r = $script:ws.ReceiveAsync((New-Object ArraySegment[byte] (,$buf)), [Threading.CancellationToken]::None); $r.Wait(); $text += [Text.Encoding]::UTF8.GetString($buf, 0, $r.Result.Count) } while (-not $r.Result.EndOfMessage)
    $o = $text | ConvertFrom-Json
    if ($o.id -eq $id) { return $o.result.result.value }
  }
}
function Status { Eval 'document.getElementById("status").textContent' }
function Busy { (Eval 'document.getElementById("go").disabled') -eq $true }
function Go { [void](Eval 'document.getElementById("go").click()') }
function ClearRows { [void](Eval '[...document.querySelectorAll("#rows .row button")].forEach(b=>b.click())'); Start-Sleep -Milliseconds 200 }
function AddLink([string]$link) {
  [void](Eval "(async()=>{const d=new DataTransfer();d.setData('text',$(ConvertTo-Json $link));document.getElementById('links').dispatchEvent(new ClipboardEvent('paste',{clipboardData:d,bubbles:true,cancelable:true}));})()")
  Start-Sleep -Milliseconds 500
}
function Quality([string]$value) { [void](Eval "var q=document.querySelector('#rows select.row__quality'); q.value='$value'; q.dispatchEvent(new Event('change',{bubbles:true}));") }
function WaitDone([int]$seconds) { $t0 = Get-Date; Start-Sleep -Milliseconds 500; while (((Get-Date) - $t0).TotalSeconds -lt $seconds) { if (-not (Busy)) { return $true }; Start-Sleep -Milliseconds 300 }; $false }
function ExplorerAt([string]$path) {
  $want = [IO.Path]::GetFullPath($path).TrimEnd('\')
  @((New-Object -ComObject Shell.Application).Windows() | Where-Object { try { $_.Document.Folder.Self.Path.TrimEnd('\') -ieq $want } catch { $false } })
}
function CloseExplorerAt([string]$path) { ExplorerAt $path | ForEach-Object { try { $_.Quit() } catch {} }; Start-Sleep -Milliseconds 500 }
function WaitExplorer([string]$path, [int]$atLeast, [int]$seconds = 8) { foreach ($i in 1..($seconds * 4)) { $n = @(ExplorerAt $path).Count; if ($n -ge $atLeast) { return $n }; Start-Sleep -Milliseconds 250 }; @(ExplorerAt $path).Count }

$proc = Start-Process $Exe -ArgumentList '--hidden' -PassThru
try {
  $tg = $null
  foreach ($i in 1..60) { Start-Sleep -Milliseconds 250; try { $tg = @(Invoke-RestMethod 'http://127.0.0.1:9337/json' -TimeoutSec 2 | Where-Object { $_.type -eq 'page' })[0]; if ($tg) { break } } catch {} }
  $script:ws = New-Object Net.WebSockets.ClientWebSocket
  $script:ws.ConnectAsync([Uri]$tg.webSocketDebuggerUrl, [Threading.CancellationToken]::None).Wait()
  Start-Sleep -Seconds 1

  # 1) Audio: os 5 formatos, com o tempo ate o primeiro progresso
  $expected = @(@('audio', '.m4a', 'aac'), @('audio-mp3', '.mp3', 'mp3'), @('audio-wav', '.wav', 'pcm_s16le'), @('audio-flac', '.flac', 'flac'), @('audio-opus', '.opus', 'opus'))
  # Abre a pasta so depois do primeiro download: abrir pasta tem teste proprio abaixo.
  [void](Eval 'window.__TAURI__.core.invoke("get_models").then(a=>window.__TAURI__.core.invoke("save_models",{models:a.models,openAtEnd:false}))'); Start-Sleep -Milliseconds 500
  foreach ($e in $expected) {
    ClearRows; AddLink $Url; Quality $e[0]
    if ($e[0] -eq 'audio') { Check 'lista de qualidade tem os 5 audios' ((Eval '["audio","audio-mp3","audio-wav","audio-flac","audio-opus"].every(v=>[...document.querySelector("#rows select.row__quality").options].some(o=>o.value===v))') -eq $true); Info "rotulos: $(Eval '[...document.querySelector(''#rows select.row__quality'').options].map(o=>o.text).join('' | '')')" }
    $sw = [Diagnostics.Stopwatch]::StartNew(); Go
    $first = $null; $conn = $false
    while ($sw.Elapsed.TotalSeconds -lt 60) { $st = Status; if ($st -match 'Conectando') { $conn = $true }; if ($st -match '%') { $first = $sw.Elapsed.TotalSeconds; break }; if (-not (Busy) -and $sw.Elapsed.TotalSeconds -gt 2) { break }; Start-Sleep -Milliseconds 100 }
    $done = WaitDone 300
    $file = @(Get-ChildItem -LiteralPath $dest -File | Where-Object Extension -eq $e[1])
    $codec = if ($file) { & ffprobe -v error -select_streams a:0 -show_entries stream=codec_name -of csv=p=0 $file[0].FullName }
    $dur = if ($file) { [double](& ffprobe -v error -show_entries format=duration -of csv=p=0 $file[0].FullName) } else { 0 }
    Check ("$($e[0]): $($e[1]) $codec, $([int]$dur)s, " + ('{0:N1} MB' -f $(if ($file) { $file[0].Length / 1MB } else { 0 })) + ", primeiro progresso em " + $(if ($first) { '{0:N1}s' -f $first } else { 'nao visto' })) ($done -and $file.Count -eq 1 -and $codec -match $e[2] -and $dur -gt 600)
    if ($e[0] -eq 'audio') { Check 'mostra Conectando enquanto prepara' $conn }
  }
  Check 'baixar outro formato nao apaga os anteriores' (@($expected | Where-Object { -not (Get-ChildItem -LiteralPath $dest -File | Where-Object Extension -eq $_[1]) }).Count -eq 0)
  $left = @(Get-ChildItem -LiteralPath $dest -File -Force | Where-Object { $_.Extension -in '.part', '.ytdl', '.webm', '.temp' })
  Check 'sem temporarios sobrando' ($left.Count -eq 0)

  # 2) Tempo ate o video comecar, com o yt-dlp real
  ClearRows; AddLink $Url; Quality '1080'
  $sw = [Diagnostics.Stopwatch]::StartNew(); Go; $first = $null
  while ($sw.Elapsed.TotalSeconds -lt 60) { if ((Status) -match '%') { $first = $sw.Elapsed.TotalSeconds; break }; Start-Sleep -Milliseconds 100 }
  Check ('video 1080p comeca a baixar (em {0:N1}s)' -f $first) ($null -ne $first)
  [void](Eval 'document.getElementById("cancel").click()'); [void](WaitDone 30)
  Get-ChildItem -LiteralPath $dest -File -Force | Where-Object { $_.Extension -in '.part', '.ytdl' -or $_.Name -match '\.f\d+\.' } | Remove-Item -Force
  ClearRows

  # 3) Drive pelo caminho rapido
  if ($env:FREY_TEST_DRIVE_FILE) {
    AddLink $env:FREY_TEST_DRIVE_FILE
    $before = @(Get-ChildItem -LiteralPath $dest -File).Count
    $sw = [Diagnostics.Stopwatch]::StartNew(); Go; $ok = WaitDone 60
    Check ('Drive, arquivo pequeno de ponta a ponta em {0:N1}s' -f $sw.Elapsed.TotalSeconds) ($ok -and @(Get-ChildItem -LiteralPath $dest -File).Count -eq $before + 1)
    ClearRows
  } else { Info 'Drive pulado: defina FREY_TEST_DRIVE_FILE' }
  if ($env:FREY_TEST_DRIVE_BIG) {
    AddLink $env:FREY_TEST_DRIVE_BIG
    $before = @(Get-ChildItem -LiteralPath $dest -File -Force | ForEach-Object Name)
    $sw = [Diagnostics.Stopwatch]::StartNew(); Go; $first = $null
    while ($sw.Elapsed.TotalSeconds -lt 40) { if ((Status) -match '%') { $first = $sw.Elapsed.TotalSeconds; break }; Start-Sleep -Milliseconds 50 }
    Check ('Drive, arquivo grande comeca sem confirmacao (em {0:N1}s)' -f $first) ($null -ne $first)
    $st = Status
    Check 'arquivo grande mostra o nome real, nao o ID' ($st -notmatch '[\w-]{30,}')
    [void](Eval 'document.getElementById("cancel").click()'); [void](WaitDone 20)
    $new = @(Get-ChildItem -LiteralPath $dest -File -Force | Where-Object { $before -notcontains $_.Name })
    Check 'parcial do arquivo grande usa o nome real' ($new.Count -eq 1 -and $new[0].Name -notmatch '^[\w-]{30,}' -and $new[0].Extension -eq '.part')
    $new | Remove-Item -Force
    ClearRows
  }

  # 4) Pasta ja aberta nao abre outra
  [void](Eval 'window.__TAURI__.core.invoke("get_models").then(a=>window.__TAURI__.core.invoke("save_models",{models:a.models,openAtEnd:true}))'); Start-Sleep -Milliseconds 500
  CloseExplorerAt $dest
  Check 'comeca sem janela na pasta' (@(ExplorerAt $dest).Count -eq 0)
  [void](Eval "window.__TAURI__.core.invoke('start_job',{request:{model:'',name:'',dest:$(ConvertTo-Json $dest),confirmExisting:false,convertTo:'',downloadOnly:true,items:[{id:9001,url:'https://www.youtube.com/watch?v=aqz-KE-bpKQ',kind:'youtube',folder:'',quality:'audio'}]}}).then(()=>1)")
  $t0 = Get-Date; while (((Get-Date) - $t0).TotalSeconds -lt 90 -and (Get-CimInstance Win32_Process -Filter "Name='yt-dlp.exe'")) { Start-Sleep -Milliseconds 300 }; Start-Sleep -Seconds 1
  Check 'pasta fechada: abre uma janela ao terminar' ((WaitExplorer $dest 1) -eq 1)
  foreach ($i in 1..2) {
    [void](Eval "window.__TAURI__.core.invoke('start_job',{request:{model:'',name:'',dest:$(ConvertTo-Json $dest),confirmExisting:false,convertTo:'',downloadOnly:true,items:[{id:900$(1 + $i),url:'https://www.youtube.com/watch?v=aqz-KE-bpKQ',kind:'youtube',folder:'',quality:'audio'}]}}).then(()=>1)")
    Start-Sleep -Seconds 2
    $t0 = Get-Date; while (((Get-Date) - $t0).TotalSeconds -lt 60 -and (Get-CimInstance Win32_Process -Filter "Name='yt-dlp.exe'")) { Start-Sleep -Milliseconds 300 }
    Start-Sleep -Seconds 3
    Check "pasta aberta: novo download $i nao abre outra" (@(ExplorerAt $dest).Count -eq 1)
  }
  [void](Eval 'window.__TAURI__.core.invoke("open_project")'); Start-Sleep -Seconds 3
  Check 'botao Abrir pasta tambem nao duplica' (@(ExplorerAt $dest).Count -eq 1)
  # Com barra final e letras diferentes continua sendo a mesma pasta
  [void](Eval "window.__TAURI__.core.invoke('start_job',{request:{model:'',name:'',dest:$(ConvertTo-Json ($dest.ToUpper() + '\')),confirmExisting:false,convertTo:'',downloadOnly:true,items:[{id:9009,url:'https://www.youtube.com/watch?v=aqz-KE-bpKQ',kind:'youtube',folder:'',quality:'audio'}]}}).then(()=>1)")
  Start-Sleep -Seconds 2; $t0 = Get-Date; while (((Get-Date) - $t0).TotalSeconds -lt 60 -and (Get-CimInstance Win32_Process -Filter "Name='yt-dlp.exe'")) { Start-Sleep -Milliseconds 300 }; Start-Sleep -Seconds 3
  Check 'caminho escrito diferente nao duplica' (@(ExplorerAt $dest).Count -eq 1)
  # Outra pasta abre normalmente, sem mexer na primeira
  [void](Eval "window.__TAURI__.core.invoke('start_job',{request:{model:'',name:'',dest:$(ConvertTo-Json $other),confirmExisting:false,convertTo:'',downloadOnly:true,items:[{id:9010,url:'https://www.youtube.com/watch?v=aqz-KE-bpKQ',kind:'youtube',folder:'',quality:'audio'}]}}).then(()=>1)")
  Start-Sleep -Seconds 2; $t0 = Get-Date; while (((Get-Date) - $t0).TotalSeconds -lt 120 -and (Get-CimInstance Win32_Process -Filter "Name='yt-dlp.exe'")) { Start-Sleep -Milliseconds 300 }
  Check 'outra pasta abre a propria janela' ((WaitExplorer $other 1) -eq 1 -and @(ExplorerAt $dest).Count -eq 1)

  # 5) Conversor com os formatos novos
  $source = @(Get-ChildItem -LiteralPath $dest -File | Where-Object Extension -eq '.m4a')[0]
  $conv = Join-Path $OutDir 'conv'; New-Item -ItemType Directory -Force -Path $conv | Out-Null
  foreach ($ext in 'flac', 'opus') {
    $json = ConvertTo-Json -Compress @{ files = @(@{ id = 9100; path = $source.FullName }); ext = $ext; mode = 'auto'; crf = 22; useGpu = $false; outDir = $conv }
    [void](Eval "window.__TAURI__.core.invoke('start_convert',{request:$json}).then(()=>1)")
    Start-Sleep -Seconds 1; $t0 = Get-Date; while (((Get-Date) - $t0).TotalSeconds -lt 120 -and (Get-CimInstance Win32_Process -Filter "Name='ffmpeg.exe'")) { Start-Sleep -Milliseconds 300 }; Start-Sleep -Milliseconds 800
    $out = @(Get-ChildItem -LiteralPath $conv -File | Where-Object Extension -eq ".$ext")
    Check "Converter gera $ext" ($out.Count -eq 1 -and $out[0].Length -gt 100000)
  }
} catch {
  Check "erro: $($_.Exception.Message)" $false
} finally {
  if ($script:ws) { $script:ws.Dispose() }
  if (-not $proc.HasExited) { Stop-Process -Id $proc.Id -Force }
  foreach ($p in $dest, $other, ($dest.ToUpper())) { try { CloseExplorerAt $p } catch {} }
  Remove-Item Env:FREY_PROJECT_CONFIG_DIR, Env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS -ErrorAction SilentlyContinue
  Start-Sleep -Seconds 1
  Remove-Item -LiteralPath $dest, $other, $big, (Join-Path $OutDir 'conv') -Recurse -Force -ErrorAction SilentlyContinue
}
"RESUMO PASS=$(@($results | Where-Object { $_ -like 'PASS*' }).Count) FAIL=$(@($results | Where-Object { $_ -like 'FAIL*' }).Count)"
if (@($results | Where-Object { $_ -like 'FAIL*' }).Count) { exit 1 }
