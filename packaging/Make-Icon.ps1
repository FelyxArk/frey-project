# Gera os icones do Frey Project a partir de branding\frey-face.png: fundo branco e carinha preta.
# Recorta a carinha, que ocupa pouco da imagem original, para continuar legivel em 16 px.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$here = $PSScriptRoot
$source = Join-Path $here 'branding\frey-face.png'
$icons = Join-Path $here '..\src-tauri\icons'
$src = [Drawing.Bitmap]::FromFile($source)

# Caixa da parte escura
$minX = $src.Width; $minY = $src.Height; $maxX = 0; $maxY = 0
for ($y = 0; $y -lt $src.Height; $y++) { for ($x = 0; $x -lt $src.Width; $x++) {
  $p = $src.GetPixel($x, $y)
  if ($p.A -gt 128 -and $p.GetBrightness() -lt 0.5) {
    if ($x -lt $minX) { $minX = $x }; if ($x -gt $maxX) { $maxX = $x }
    if ($y -lt $minY) { $minY = $y }; if ($y -gt $maxY) { $maxY = $y }
  }
} }
if ($maxX -le $minX) { throw 'Nao encontrei a carinha na imagem.' }
$side = [Math]::Max($maxX - $minX, $maxY - $minY)
$margin = $side * 0.20
$cropSide = $side + 2 * $margin
$cropX = ($minX + $maxX) / 2 - $cropSide / 2
$cropY = ($minY + $maxY) / 2 - $cropSide / 2

function New-Icon([int]$size) {
  $b = New-Object Drawing.Bitmap($size, $size, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
  $g = [Drawing.Graphics]::FromImage($b)
  $g.SmoothingMode = 'AntiAlias'; $g.InterpolationMode = 'HighQualityBicubic'; $g.PixelOffsetMode = 'HighQuality'
  $g.Clear([Drawing.Color]::Transparent)
  # Fundo branco de canto arredondado
  $r = [Math]::Max(2, $size * 0.2); $d = $r * 2; $s = $size - 1
  $path = New-Object Drawing.Drawing2D.GraphicsPath
  $path.AddArc(0, 0, $d, $d, 180, 90); $path.AddArc($s - $d, 0, $d, $d, 270, 90)
  $path.AddArc($s - $d, $s - $d, $d, $d, 0, 90); $path.AddArc(0, $s - $d, $d, $d, 90, 90); $path.CloseFigure()
  $g.FillPath([Drawing.Brushes]::White, $path)
  $g.SetClip($path)
  $attr = New-Object Drawing.Imaging.ImageAttributes
  $attr.SetWrapMode('TileFlipXY')
  $g.DrawImage($src, (New-Object Drawing.Rectangle(0, 0, $size, $size)), [single]$cropX, [single]$cropY, [single]$cropSide, [single]$cropSide, 'Pixel', $attr)
  $g.Dispose(); $path.Dispose(); $attr.Dispose()
  $b
}

$sizes = 16, 20, 24, 32, 40, 48, 64, 128, 256
$pngs = @{}
foreach ($size in $sizes) {
  $b = New-Icon $size
  $ms = New-Object IO.MemoryStream
  $b.Save($ms, [Drawing.Imaging.ImageFormat]::Png)
  $pngs[$size] = $ms.ToArray()
  $ms.Dispose(); $b.Dispose()
}

# .ico com PNG dentro de cada tamanho
$ico = New-Object IO.MemoryStream
$w = New-Object IO.BinaryWriter($ico)
$w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
foreach ($size in $sizes) {
  $dim = if ($size -ge 256) { 0 } else { $size }
  $w.Write([byte]$dim); $w.Write([byte]$dim); $w.Write([byte]0); $w.Write([byte]0)
  $w.Write([uint16]1); $w.Write([uint16]32); $w.Write([uint32]$pngs[$size].Length); $w.Write([uint32]$offset)
  $offset += $pngs[$size].Length
}
foreach ($size in $sizes) { $w.Write($pngs[$size]) }
$w.Flush()
[IO.File]::WriteAllBytes((Join-Path $icons 'icon.ico'), $ico.ToArray())
[IO.File]::WriteAllBytes((Join-Path $icons 'icon.png'), $pngs[256])
[IO.File]::WriteAllBytes((Join-Path $here 'branding\frey-setup.ico'), $ico.ToArray())
$w.Dispose()

# Imagem do cabecalho do instalador
$h = New-Icon 58
$h.Save((Join-Path $here 'branding\frey-header.png'), [Drawing.Imaging.ImageFormat]::Png)
$h.Dispose()

# Folha de conferencia: icones sobre fundo claro e escuro, no tamanho real
$sheet = New-Object Drawing.Bitmap(620, 330)
$g = [Drawing.Graphics]::FromImage($sheet)
$g.FillRectangle((New-Object Drawing.SolidBrush([Drawing.Color]::FromArgb(236, 236, 236))), 0, 0, 620, 165)
$g.FillRectangle((New-Object Drawing.SolidBrush([Drawing.Color]::FromArgb(30, 30, 34))), 0, 165, 620, 165)
foreach ($row in 0, 1) {
  $x = 10
  foreach ($size in 16, 20, 24, 32, 48, 64, 128) {
    $ms = New-Object IO.MemoryStream(, $pngs[$size]); $i = [Drawing.Image]::FromStream($ms)
    $g.DrawImageUnscaled($i, $x, 15 + $row * 165); $x += $size + 18; $i.Dispose(); $ms.Dispose()
  }
}
$g.Dispose()
$sheet.Save((Join-Path $here 'branding\icon-preview.png'), [Drawing.Imaging.ImageFormat]::Png)
$sheet.Dispose(); $src.Dispose()
"crop: x=$([int]$cropX) y=$([int]$cropY) lado=$([int]$cropSide); .ico $((Get-Item (Join-Path $icons 'icon.ico')).Length) bytes"
