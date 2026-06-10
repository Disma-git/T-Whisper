# Genererar assets/t-whisper.ico (blå ring + grön stapel, samma design
# som tray-ikonen) i storlekarna 256/64/48/32/16 med PNG-komprimerade
# poster. Körs vid behov: powershell -File installer\make-icon.ps1
Add-Type -AssemblyName System.Drawing

$sizes = 256, 64, 48, 32, 16
$pngs = @()

foreach ($s in $sizes) {
    $bmp = New-Object System.Drawing.Bitmap($s, $s)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.Clear([System.Drawing.Color]::Transparent)

    $ringW = [Math]::Max(2, [int]($s * 0.10))
    $pad = [Math]::Max(1, [int]($s * 0.03))

    # Mörk botten
    $dark = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 38, 38, 44))
    $g.FillEllipse($dark, $pad, $pad, $s - 2 * $pad, $s - 2 * $pad)

    # Blå statusring
    $bluePen = New-Object System.Drawing.Pen([System.Drawing.Color]::FromArgb(255, 90, 120, 200), $ringW)
    $half = [int]($ringW / 2)
    $g.DrawEllipse($bluePen, $pad + $half, $pad + $half, $s - 2 * ($pad + $half), $s - 2 * ($pad + $half))

    # Grön nivåstapel (3/4 höjd som "logotypläge")
    $barW = [int]($s * 0.30)
    $barH = [int]($s * 0.45)
    $barX = [int](($s - $barW) / 2)
    $barY = [int]($s * 0.72) - $barH
    $green = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 70, 200, 90))
    $g.FillRectangle($green, $barX, $barY, $barW, $barH)

    $g.Dispose()
    $ms = New-Object System.IO.MemoryStream
    $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    $pngs += , @($s, $ms.ToArray())
}

# Bygg ICO-filen: header + katalogposter + PNG-data
$out = New-Object System.IO.MemoryStream
$bw = New-Object System.IO.BinaryWriter($out)
$bw.Write([UInt16]0)            # reserverad
$bw.Write([UInt16]1)            # typ: ikon
$bw.Write([UInt16]$pngs.Count)  # antal bilder

$offset = 6 + 16 * $pngs.Count
foreach ($entry in $pngs) {
    $s = $entry[0]; $data = $entry[1]
    $dim = if ($s -ge 256) { 0 } else { $s }
    $bw.Write([Byte]$dim)         # bredd
    $bw.Write([Byte]$dim)         # höjd
    $bw.Write([Byte]0)            # palett
    $bw.Write([Byte]0)            # reserverad
    $bw.Write([UInt16]1)          # färgplan
    $bw.Write([UInt16]32)         # bitar/pixel
    $bw.Write([UInt32]$data.Length)
    $bw.Write([UInt32]$offset)
    $offset += $data.Length
}
foreach ($entry in $pngs) { $bw.Write($entry[1]) }
$bw.Flush()

$dest = Join-Path $PSScriptRoot "..\assets\t-whisper.ico"
New-Item -ItemType Directory -Force (Split-Path $dest) | Out-Null
[System.IO.File]::WriteAllBytes($dest, $out.ToArray())
Write-Host "Skrev $dest ($($out.Length) byte)"
