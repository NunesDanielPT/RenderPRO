# Descarrega o FFmpeg (build GPL partilhada da BtbN) para src-tauri/ffmpeg/.
#   powershell -ExecutionPolicy Bypass -File scripts/fetch-ffmpeg.ps1
#
# Porque GPL: inclui libx264/libx265 (H.264/H.265 de qualidade) e o Snappy (HAP).
# O RenderPRO chama o ffmpeg.exe/ffprobe.exe como programas separados.
# Prefere a série 7.1 (estável); se a BtbN deixar de a publicar usa a estável mais recente.

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$dest = Join-Path $root "src-tauri/ffmpeg"
if (Test-Path (Join-Path $dest "ffmpeg.exe")) { Write-Host "FFmpeg já existe em $dest"; exit 0 }

$prefer = "7.1"
$headers = @{ "User-Agent" = "renderpro-build" }
if ($env:GITHUB_TOKEN) { $headers["Authorization"] = "Bearer $env:GITHUB_TOKEN" }
$release = Invoke-RestMethod -Headers $headers -Uri "https://api.github.com/repos/BtbN/FFmpeg-Builds/releases/tags/latest"
# Só builds de versões estáveis (nX.Y), nunca a "master".
$candidates = $release.assets | Where-Object { $_.name -match '^ffmpeg-n(\d+)\.(\d+)-latest-win64-gpl-shared-[\d.]+\.zip$' }
if (-not $candidates) {
  $release.assets | ForEach-Object { Write-Host "  disponível: $($_.name)" }
  throw "Não encontrei nenhum build FFmpeg win64-gpl-shared estável na release 'latest' da BtbN"
}
$asset = $candidates | Where-Object { $_.name -like "ffmpeg-n$prefer-*" } | Select-Object -First 1
if (-not $asset) {
  $asset = $candidates |
    Sort-Object { [int]($_.name -replace '^ffmpeg-n(\d+)\.(\d+).*', '$1') * 100 + [int]($_.name -replace '^ffmpeg-n(\d+)\.(\d+).*', '$2') } -Descending |
    Select-Object -First 1
}
Write-Host "FFmpeg escolhido: $($asset.name)"

$tmp = Join-Path ([System.IO.Path]::GetTempPath()) "renderpro-ffmpeg"
if (Test-Path $tmp) { Remove-Item -Recurse -Force $tmp }
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
$zip = Join-Path $tmp $asset.name
Write-Host "A descarregar $($asset.browser_download_url)"
Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $zip
Expand-Archive -Path $zip -DestinationPath $tmp -Force
$dir = Get-ChildItem $tmp -Directory | Select-Object -First 1

New-Item -ItemType Directory -Force -Path $dest | Out-Null
Copy-Item (Join-Path $dir.FullName "bin/ffmpeg.exe") $dest
Copy-Item (Join-Path $dir.FullName "bin/ffprobe.exe") $dest
Copy-Item (Join-Path $dir.FullName "bin/*.dll") $dest
Copy-Item (Join-Path $dir.FullName "LICENSE.txt") (Join-Path $dest "FFMPEG-LICENSE.txt")
Remove-Item -Recurse -Force $tmp

& (Join-Path $dest "ffmpeg.exe") -hide_banner -version | Select-Object -First 1
$enc = & (Join-Path $dest "ffmpeg.exe") -hide_banner -encoders
foreach ($e in @("hap", "libx264", "libx265", "prores_ks", "dnxhd", "pcm_s16le", "aac")) {
  if (-not ($enc -match "\s$e\s")) { throw "O FFmpeg descarregado não tem o encoder $e" }
}
Write-Host "FFmpeg pronto em $dest"
