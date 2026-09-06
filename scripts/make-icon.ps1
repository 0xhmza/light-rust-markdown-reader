# Draws assets\mdlite.ico from scratch. No image files are checked in as
# artwork: every size is rendered at its own resolution so the small ones stay
# crisp instead of being a blurry downscale of the large one.
param([string]$Out = (Join-Path $PSScriptRoot '..\assets\mdlite.ico'))
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

# A rounded tile in the viewer's own blue, carrying a page of text: one heading
# bar and two lines. Few shapes, high contrast, so 16 pixels still reads.
function New-Tile([int]$Size) {
    $bmp = New-Object Drawing.Bitmap($Size, $Size, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = 'AntiAlias'
    $g.InterpolationMode = 'HighQualityBicubic'
    $g.PixelOffsetMode = 'HighQuality'

    $inset = [Math]::Max(0.5, $Size * 0.045)
    $box = New-Object Drawing.RectangleF($inset, $inset, ($Size - 2 * $inset), ($Size - 2 * $inset))
    $radius = [Math]::Max(1.5, $Size * 0.22)
    $path = New-Object Drawing.Drawing2D.GraphicsPath
    $d = $radius * 2
    $path.AddArc($box.Left, $box.Top, $d, $d, 180, 90)
    $path.AddArc($box.Right - $d, $box.Top, $d, $d, 270, 90)
    $path.AddArc($box.Right - $d, $box.Bottom - $d, $d, $d, 0, 90)
    $path.AddArc($box.Left, $box.Bottom - $d, $d, $d, 90, 90)
    $path.CloseFigure()

    $top = [Drawing.Color]::FromArgb(255, 106, 165, 250)   # the dark palette's accent
    $bottom = [Drawing.Color]::FromArgb(255, 21, 85, 162)  # the light palette's accent
    $brush = New-Object Drawing.Drawing2D.LinearGradientBrush($box, $top, $bottom, 90.0)
    $g.FillPath($brush, $path)

    # Heading bar, then two lines of text, as one centred group.
    $x = $Size * 0.26
    $heading = [Math]::Max(2.0, $Size * 0.135)
    $line = [Math]::Max(1.0, $Size * 0.085)
    $gap = [Math]::Max(1.0, $Size * 0.088)
    $total = $heading + $gap + $line + $gap + $line
    $y = ($Size - $total) / 2
    $ink = New-Object Drawing.SolidBrush([Drawing.Color]::FromArgb(255, 255, 255, 255))
    $faint = New-Object Drawing.SolidBrush([Drawing.Color]::FromArgb(214, 255, 255, 255))
    foreach ($bar in @(
            @{ w = 0.48; h = $heading; b = $ink },
            @{ w = 0.44; h = $line; b = $faint },
            @{ w = 0.30; h = $line; b = $faint })) {
        $rect = New-Object Drawing.RectangleF($x, $y, ($Size * $bar.w), $bar.h)
        if ($Size -ge 32) {
            # Rounded ends, but only where there are pixels to round.
            $r = $bar.h / 2
            $p = New-Object Drawing.Drawing2D.GraphicsPath
            $p.AddArc($rect.Left, $rect.Top, ($r * 2), ($r * 2), 90, 180)
            $p.AddArc(($rect.Right - $r * 2), $rect.Top, ($r * 2), ($r * 2), 270, 180)
            $p.CloseFigure()
            $g.FillPath($bar.b, $p)
            $p.Dispose()
        }
        else {
            $g.FillRectangle($bar.b, $rect)
        }
        $y += $bar.h + $gap
    }
    $ink.Dispose(); $faint.Dispose(); $brush.Dispose(); $path.Dispose(); $g.Dispose()
    $bmp
}

# A 32-bit DIB with the trailing AND mask the icon format still expects.
function ConvertTo-Dib([Drawing.Bitmap]$Bitmap) {
    $w = $Bitmap.Width; $h = $Bitmap.Height
    $stream = New-Object IO.MemoryStream
    $writer = New-Object IO.BinaryWriter($stream)
    $writer.Write([uint32]40); $writer.Write([int32]$w); $writer.Write([int32]($h * 2))
    $writer.Write([uint16]1); $writer.Write([uint16]32); $writer.Write([uint32]0)
    $writer.Write([uint32]($w * $h * 4)); 0..3 | ForEach-Object { $writer.Write([uint32]0) }
    $data = $Bitmap.LockBits((New-Object Drawing.Rectangle(0, 0, $w, $h)),
        [Drawing.Imaging.ImageLockMode]::ReadOnly, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $row = New-Object byte[] ($w * 4)
    for ($y = $h - 1; $y -ge 0; $y--) {
        [Runtime.InteropServices.Marshal]::Copy(
            [IntPtr]($data.Scan0.ToInt64() + $y * $data.Stride), $row, 0, $row.Length)
        $writer.Write($row)
    }
    $Bitmap.UnlockBits($data)
    # Every pixel opaque: 32-bit icons take their shape from the alpha channel.
    $maskRow = [Math]::Floor(($w + 31) / 32) * 4
    $writer.Write((New-Object byte[] ($maskRow * $h)))
    $writer.Flush()
    , $stream.ToArray()
}

function ConvertTo-Png([Drawing.Bitmap]$Bitmap) {
    $stream = New-Object IO.MemoryStream
    $Bitmap.Save($stream, [Drawing.Imaging.ImageFormat]::Png)
    , $stream.ToArray()
}

# Small sizes as DIBs, where every shell path can read them; the large ones as
# PNG, which is what keeps the icon from costing more than the program.
$sizes = @(16, 20, 24, 32, 40, 48, 64, 128, 256)
$images = foreach ($size in $sizes) {
    $bmp = New-Tile $size
    [byte[]]$bytes = if ($size -le 32) { ConvertTo-Dib $bmp } else { ConvertTo-Png $bmp }
    $bmp.Dispose()
    [pscustomobject]@{ Size = $size; Bytes = $bytes }
}

$dir = Split-Path -Parent $Out
New-Item -ItemType Directory -Path $dir -Force | Out-Null
$stream = New-Object IO.MemoryStream
$writer = New-Object IO.BinaryWriter($stream)
$writer.Write([uint16]0); $writer.Write([uint16]1); $writer.Write([uint16]$images.Count)
$offset = 6 + 16 * $images.Count
foreach ($image in $images) {
    $side = if ($image.Size -ge 256) { 0 } else { $image.Size }
    $writer.Write([byte]$side); $writer.Write([byte]$side)
    $writer.Write([byte]0); $writer.Write([byte]0)
    $writer.Write([uint16]1); $writer.Write([uint16]32)
    $writer.Write([uint32]$image.Bytes.Length); $writer.Write([uint32]$offset)
    $offset += $image.Bytes.Length
}
foreach ($image in $images) { $writer.Write($image.Bytes) }
$writer.Flush()
[IO.File]::WriteAllBytes((Resolve-Path -LiteralPath $dir).Path + '\' + (Split-Path -Leaf $Out), $stream.ToArray())
"{0}  {1} sizes, {2:N0} bytes" -f (Split-Path -Leaf $Out), $images.Count, $stream.Length
