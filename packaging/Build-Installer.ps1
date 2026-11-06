# Gera dist\Frey-Project-Setup-<versao>.exe com o app e todas as dependencias.
# Compila em pasta propria para nao brigar com uma copia aberta do app. Use -BuildDir num disco com 7 GB livres.
# Precisa de: Rust, Inno Setup 6, e ffmpeg, ffprobe, yt-dlp e deno no PATH. Copia esses programas para o instalador.
param(
  [string]$Iscc = "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
  # Instalador offline oficial da Microsoft: https://developer.microsoft.com/microsoft-edge/webview2/
  [Parameter(Mandatory)][string]$WebView2Installer,
  [string]$BuildDir = (Join-Path $env:LOCALAPPDATA 'frey-project-target')
)
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
$root = Split-Path -Parent $here
$dist = Join-Path $root 'dist'
$stage = Join-Path $dist 'stage'
$iscc = $Iscc
$webview = $WebView2Installer
$version = (Get-Content (Join-Path $root 'src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json).version

function Resolve-Tool([string]$name) {
  $cmd = Get-Command $name -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
  if (-not $cmd) { throw "$name nao encontrado neste PC." }
  $item = Get-Item -LiteralPath $cmd.Source
  if ($item.Target) { Get-Item -LiteralPath ([string]$item.Target) } else { $item }
}

$env:Path = [Environment]::GetEnvironmentVariable('Path', 'User') + ';' + [Environment]::GetEnvironmentVariable('Path', 'Machine') + ";$env:USERPROFILE\.cargo\bin"

Write-Host '== compilando o app'
Push-Location (Join-Path $root 'src-tauri')
$env:CARGO_TARGET_DIR = $BuildDir
& cargo build --release
if ($LASTEXITCODE) { throw 'cargo build falhou' }
Remove-Item Env:CARGO_TARGET_DIR
Pop-Location

Write-Host '== preparando os arquivos'
Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue
foreach ($dir in 'bin', 'licenses', 'webview2') { New-Item -ItemType Directory -Force -Path (Join-Path $stage $dir) | Out-Null }
Copy-Item (Join-Path $BuildDir 'release\frey-project.exe') $stage
$ffmpeg = Resolve-Tool 'ffmpeg'
foreach ($name in 'ffmpeg', 'ffprobe', 'yt-dlp', 'deno') { Copy-Item (Resolve-Tool $name).FullName (Join-Path $stage 'bin') }
$license = Join-Path (Split-Path -Parent $ffmpeg.DirectoryName) 'LICENSE'
if (-not (Test-Path -LiteralPath $license)) { throw "Licenca do FFmpeg nao encontrada: $license" }
Copy-Item -LiteralPath $license (Join-Path $stage 'licenses\FFmpeg-LICENSE.txt')
Copy-Item -LiteralPath (Join-Path $root 'LICENSE') (Join-Path $stage 'licenses\Frey-Project-LICENSE.txt')
& (Join-Path $here 'Collect-Licenses.ps1') -OutFile (Join-Path $stage 'licenses\Rust-Libraries.txt')

$signature = Get-AuthenticodeSignature $webview
if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'Microsoft Corporation') { throw 'Assinatura do WebView2 invalida.' }
Copy-Item $webview (Join-Path $stage 'webview2')

# Confere que as copias rodam a partir da pasta preparada
$bin = Join-Path $stage 'bin'
$versions = @(
  "  ffmpeg: " + ((& "$bin\ffmpeg.exe" -version | Select-Object -First 1) -replace ' Copyright.*$', ''),
  "  ffprobe: " + ((& "$bin\ffprobe.exe" -version | Select-Object -First 1) -replace ' Copyright.*$', ''),
  "  yt-dlp: " + (& "$bin\yt-dlp.exe" --version),
  "  deno: " + ((& "$bin\deno.exe" --version | Select-Object -First 1)),
  "  WebView2: " + (Get-Item $webview).VersionInfo.FileVersion
)
$versions | ForEach-Object { Write-Host $_ }
$notice = (Get-Content (Join-Path $here 'NOTICES.txt') -Raw).Replace('{{VERSIONS}}', ($versions -join "`r`n"))
[IO.File]::WriteAllText((Join-Path $stage 'NOTICES.txt'), $notice, (New-Object Text.UTF8Encoding($true)))

Write-Host '== compilando o instalador'
Push-Location $here
& $iscc /Qp "/DStageRoot=$stage" "/DOutputRoot=$dist" "/DAppVersionText=$version" 'Frey-Project.iss'
if ($LASTEXITCODE) { throw 'ISCC falhou' }
Pop-Location
Remove-Item -LiteralPath $stage -Recurse -Force

$out = Get-Item (Join-Path $dist "Frey-Project-Setup-$version.exe")
Write-Host ('== pronto: {0} ({1:N0} MB)' -f $out.FullName, ($out.Length / 1MB))
Write-Host ('SHA256 ' + (Get-FileHash $out.FullName -Algorithm SHA256).Hash)
