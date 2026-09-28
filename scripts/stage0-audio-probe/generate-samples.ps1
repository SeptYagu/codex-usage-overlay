param(
    [string]$OutputDirectory = (Join-Path $PSScriptRoot 'samples')
)

$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null

ffmpeg -hide_banner -loglevel error -y -f lavfi -i 'sine=frequency=440:duration=1' -c:a libmp3lame (Join-Path $OutputDirectory 'short.mp3')
ffmpeg -hide_banner -loglevel error -y -f lavfi -i 'sine=frequency=440:duration=1' -c:a aac -f adts (Join-Path $OutputDirectory 'short.aac')
ffmpeg -hide_banner -loglevel error -y -f lavfi -i 'sine=frequency=440:duration=1' -c:a aac (Join-Path $OutputDirectory 'short.m4a')
ffmpeg -hide_banner -loglevel error -y -f lavfi -i 'sine=frequency=440:duration=1' -c:a pcm_s16le (Join-Path $OutputDirectory 'short.wav')
ffmpeg -hide_banner -loglevel error -y -f lavfi -i 'sine=frequency=440:duration=13' -c:a pcm_s16le (Join-Path $OutputDirectory 'long.wav')
