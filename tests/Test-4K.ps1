# Testa "4K para edicao" de verdade: baixa um filme aberto da Blender Foundation (cerca de 700 MB),
# cancela no download, retoma, cancela na conversao, retoma e confere o resultado.
# Roda pelo depurador do WebView, com configuracao temporaria e sem mouse ou teclado do Windows.
param(
  [Parameter(Mandatory)][string]$OutDir,
  [string]$Exe,
  [string]$Url = 'https://www.youtube.com/watch?v=aqz-KE-bpKQ'
)
$ErrorActionPreference = 'Stop'
if (-not $Exe) { $Exe = Join-Path $env:LOCALAPPDATA 'frey-project-target\release\frey-project.exe' }
if (Get-Process frey-project -ErrorAction SilentlyContinue) { throw 'O Frey Project esta aberto. Encerre antes de testar.' }
Remove-Item -LiteralPath $OutDir -Recurse -Force -ErrorAction SilentlyContinue
$dest = Join-Path $OutDir 'dest'; $cfg = Join-Path $OutDir 'config'
New-Item -ItemType Directory -Force -Path $dest, $cfg | Out-Null
[IO.File]::WriteAllText("$cfg\settings.json", ('{"idioma":"pt","so_baixar":true,"destino":' + (ConvertTo-Json $dest) + '}'), (New-Object Text.UTF8Encoding($false)))
[IO.File]::WriteAllText("$cfg\modelos.json", '{"abrirAoFinal":false,"modelos":[{"nome":"T","pastaRaiz":"{nome}","pastas":["a"],"pastaDestino":"a"}]}', (New-Object Text.UTF8Encoding($false)))
$env:FREY_PROJECT_CONFIG_DIR = $cfg
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9336 --remote-allow-origins=*'
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
function Row { Eval 'document.querySelector("#rows .row__state").textContent' }
function Status { Eval 'document.getElementById("status").textContent' }
function Busy { (Eval 'document.getElementById("go").disabled') -eq $true }
function Tools { @(Get-CimInstance Win32_Process -Filter "Name='yt-dlp.exe' OR Name='ffmpeg.exe' OR Name='deno.exe'") }
function Temp { @(Get-ChildItem -LiteralPath $dest -Directory -Force | Where-Object Name -like '.frey-4k-*') }
function WaitFor([scriptblock]$cond, [int]$seconds, [string]$what) {
  $t0 = Get-Date
  while (((Get-Date) - $t0).TotalSeconds -lt $seconds) { if (& $cond) { return $true }; Start-Sleep -Milliseconds 500 }
  Info "tempo esgotado esperando: $what (status: $(Status))"
  $false
}
$proc = Start-Process $Exe -ArgumentList '--hidden' -PassThru
try {
  $tg = $null
  foreach ($i in 1..60) { Start-Sleep -Milliseconds 250; try { $tg = @(Invoke-RestMethod 'http://127.0.0.1:9336/json' -TimeoutSec 2 | Where-Object { $_.type -eq 'page' })[0]; if ($tg) { break } } catch {} }
  $script:ws = New-Object Net.WebSockets.ClientWebSocket
  $script:ws.ConnectAsync([Uri]$tg.webSocketDebuggerUrl, [Threading.CancellationToken]::None).Wait()
  Start-Sleep -Seconds 1
  [void](Eval "(async()=>{const d=new DataTransfer();d.setData('text',$(ConvertTo-Json $Url));document.getElementById('links').dispatchEvent(new ClipboardEvent('paste',{clipboardData:d,bubbles:true,cancelable:true}));})()")
  Start-Sleep -Milliseconds 700
  Check 'opcao 4K para edicao aparece' ((Eval '[...document.querySelector("#rows select.row__quality").options].some(o=>o.value==="edit4k"&&o.text==="4K para edição")') -eq $true)
  [void](Eval 'var q=document.querySelector("#rows select.row__quality"); q.value="edit4k"; q.dispatchEvent(new Event("change",{bubbles:true}));')

  # 1) Cancela durante o download
  [void](Eval 'document.getElementById("go").click()')
  Check 'comeca a baixar' (WaitFor { (Status) -match '%' } 120 'progresso')
  Start-Sleep -Seconds 6
  Info "antes de cancelar: $(Status)"
  [void](Eval 'document.getElementById("cancel").click()')
  Check 'cancelar devolve o painel' (WaitFor { -not (Busy) } 30 'cancelar')
  Start-Sleep -Seconds 1
  Check 'cancelar encerra yt-dlp, ffmpeg e Deno' ((Tools).Count -eq 0)
  $parts = @(if (Temp) { Get-ChildItem -LiteralPath (Temp)[0].FullName -File -Force })
  Info ('parcial: {0} arquivo(s), {1:N0} MB' -f $parts.Count, (($parts | Measure-Object Length -Sum).Sum / 1MB))
  Check 'parcial fica na pasta temporaria' ($parts.Count -ge 1)
  Check 'nada solto no destino ainda' (@(Get-ChildItem -LiteralPath $dest -File).Count -eq 0)

  # 2) Retoma e cancela durante a conversao
  $t1 = Get-Date
  [void](Eval 'document.getElementById("go").click()')
  Check 'retoma e chega a conversao' (WaitFor { (Row) -match 'Convertendo' } 900 'conversao')
  Info ('download terminou em {0:N0}s' -f ((Get-Date) - $t1).TotalSeconds)
  Start-Sleep -Seconds 8
  $ff = @(Get-CimInstance Win32_Process -Filter "Name='ffmpeg.exe'")
  Info "ffmpeg: $(if ($ff) { $ff[0].CommandLine -replace '.*(-c:v\s+\S+).*', '$1' })"
  Info "convertendo: $(Status)"
  [void](Eval 'document.getElementById("cancel").click()')
  Check 'cancelar na conversao devolve o painel' (WaitFor { -not (Busy) } 30 'cancelar conversao')
  Start-Sleep -Seconds 1
  Check 'cancelar encerra o ffmpeg' ((Tools).Count -eq 0)
  Check 'conversao parcial e apagada' (@(Get-ChildItem -LiteralPath $dest -File).Count -eq 0)
  $source = @(Get-ChildItem -LiteralPath (Temp)[0].FullName -File -Force | Where-Object Extension -notin '.part', '.ytdl')
  Check 'video baixado continua guardado' ($source.Count -eq 1)
  if ($source.Count) {
    $v = & ffprobe -v error -select_streams v:0 -show_entries stream=codec_name,width,height -of csv=p=0 $source[0].FullName
    Info ('baixado: {0} ({1:N0} MB)' -f $v, ($source[0].Length / 1MB))
    Check 'veio em 4K' ($v -match ',3840,2160')
  }

  # 3) Retoma sem baixar de novo e termina
  $t2 = Get-Date
  [void](Eval 'document.getElementById("go").click()')
  Check 'nova tentativa vai direto para a conversao' (WaitFor { (Row) -match 'Convertendo' } 60 'conversao de novo')
  $finished = WaitFor { -not (Busy) } 3000 'fim da conversao'
  Check 'conversao termina' $finished
  Info ('conversao levou {0:N0}s' -f ((Get-Date) - $t2).TotalSeconds)
  Info "status: $(Status) | linha: $(Row)"
  Start-Sleep -Seconds 1
  $final = @(Get-ChildItem -LiteralPath $dest -File)
  Check 'sobra um arquivo so, MP4' ($final.Count -eq 1 -and $final[0].Extension -eq '.mp4')
  Check 'pasta temporaria removida' ((Temp).Count -eq 0)
  if ($final.Count) {
    $v = & ffprobe -v error -select_streams v:0 -show_entries stream=codec_name,width,height,pix_fmt,r_frame_rate -of csv=p=0 $final[0].FullName
    $a = & ffprobe -v error -select_streams a:0 -show_entries stream=codec_name -of csv=p=0 $final[0].FullName
    $d = [double](& ffprobe -v error -show_entries format=duration -of csv=p=0 $final[0].FullName)
    Info ('final: {0} | audio {1} | {2:N0}s | {3:N0} MB | {4}' -f $v, $a, $d, ($final[0].Length / 1MB), $final[0].Name)
    Check 'video H.264 em 4K, 8 bits' ($v -match '^h264,3840,2160,yuv420p')
    Check 'audio AAC junto' ($a -match 'aac')
    Check 'duracao inteira' ($d -gt 600)
    $err = & ffmpeg -v error -i $final[0].FullName -map 0 -c copy -f null - 2>&1 | Select-Object -First 3
    Check 'arquivo integro' (-not $err)
  }
} catch {
  Check "erro: $($_.Exception.Message)" $false
} finally {
  if ($script:ws) { $script:ws.Dispose() }
  if (-not $proc.HasExited) { Stop-Process -Id $proc.Id -Force }
  Tools | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
  Remove-Item Env:FREY_PROJECT_CONFIG_DIR, Env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS -ErrorAction SilentlyContinue
  Remove-Item -LiteralPath $dest -Recurse -Force -ErrorAction SilentlyContinue
}
"RESUMO PASS=$(@($results | Where-Object { $_ -like 'PASS*' }).Count) FAIL=$(@($results | Where-Object { $_ -like 'FAIL*' }).Count)"
if (@($results | Where-Object { $_ -like 'FAIL*' }).Count) { exit 1 }
