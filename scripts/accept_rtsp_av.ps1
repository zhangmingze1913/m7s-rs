param(
    [string]$Url = "rtsp://127.0.0.1:8554/live/test",
    [int]$Seconds = 3
)

$ErrorActionPreference = "Stop"
$ffmpeg = Join-Path $PSScriptRoot "..\node_modules\ffmpeg-static\ffmpeg.exe"
if (-not (Test-Path $ffmpeg)) {
    throw "ffmpeg not found: $ffmpeg"
}

foreach ($transport in @("tcp", "udp")) {
    Write-Host "== RTSP A/V acceptance: $transport =="
    & $ffmpeg -hide_banner -v error -rtsp_transport $transport `
        -i $Url -t $Seconds -map 0:v -map 0:a -f null - 2>&1 |
        ForEach-Object { Write-Host $_ }
    if ($LASTEXITCODE -ne 0) {
        throw "ffmpeg $transport failed with exit code $LASTEXITCODE"
    }
    Write-Host "PASS: $transport"
}

Write-Host "RTSP A/V acceptance passed (FFmpeg, TCP + UDP)"
