# Junta os avisos de licenca de todas as bibliotecas Rust compiladas no app.
# Uso: Collect-Licenses.ps1 -OutFile licenses\Rust-Libraries.txt
param([Parameter(Mandatory)][string]$OutFile)
$ErrorActionPreference = 'Stop'
$manifest = Join-Path $PSScriptRoot '..\src-tauri\Cargo.toml'
$meta = (& cargo metadata --format-version 1 --manifest-path $manifest --filter-platform x86_64-pc-windows-msvc) -join "`n" | ConvertFrom-Json
if ($LASTEXITCODE) { throw 'cargo metadata falhou' }

# So o que entra no executavel: dependencias normais, a partir do pacote do app.
$byId = @{}; foreach ($p in $meta.packages) { $byId[$p.id] = $p }
$nodes = @{}; foreach ($n in $meta.resolve.nodes) { $nodes[$n.id] = $n }
$root = @($meta.packages | Where-Object { $_.name -eq 'frey-project' })[0]
$seen = @{}; $queue = New-Object System.Collections.Queue; $queue.Enqueue($root.id)
while ($queue.Count) {
  $id = $queue.Dequeue()
  if ($seen[$id]) { continue }
  $seen[$id] = $true
  foreach ($dep in $nodes[$id].deps) {
    if (@($dep.dep_kinds | Where-Object { $null -eq $_.kind }).Count) { $queue.Enqueue($dep.pkg) }
  }
}

$out = New-Object Text.StringBuilder
$packages = @($seen.Keys | Where-Object { $_ -ne $root.id } | ForEach-Object { $byId[$_] } | Sort-Object name, version)
$missing = New-Object System.Collections.ArrayList
[void]$out.AppendLine("Bibliotecas Rust incluidas no Frey Project: $($packages.Count)`n")
foreach ($p in $packages) {
  $dir = Split-Path -Parent $p.manifest_path
  [void]$out.AppendLine(('=' * 78))
  [void]$out.AppendLine("$($p.name) $($p.version)")
  [void]$out.AppendLine("Licenca: $($p.license)")
  if ($p.repository) { [void]$out.AppendLine("Codigo: $($p.repository)") }
  $files = @(Get-ChildItem -LiteralPath $dir -File | Where-Object { $_.Name -match '^(LICEN[SC]E|COPYING|UNLICENSE|NOTICE)' } | Sort-Object Name)
  if (-not $files.Count) { [void]$missing.Add("$($p.name) $($p.version) ($($p.license))") }
  foreach ($f in $files) {
    [void]$out.AppendLine("`n--- $($f.Name)`n")
    [void]$out.AppendLine((Get-Content -LiteralPath $f.FullName -Raw -Encoding UTF8).Trim())
  }
  [void]$out.AppendLine('')
}
[IO.File]::WriteAllText($OutFile, $out.ToString(), (New-Object Text.UTF8Encoding($true)))
Write-Host "licencas: $($packages.Count) bibliotecas, $([int]((Get-Item -LiteralPath $OutFile).Length / 1KB)) KB, sem arquivo de licenca: $($missing.Count)"
$missing | ForEach-Object { Write-Host "  sem arquivo: $_" }
$summary = $packages | Group-Object license | Sort-Object Count -Descending | ForEach-Object { "$($_.Count)x $($_.Name)" }
Write-Host ($summary -join ' | ')
