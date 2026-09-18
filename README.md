# FAN

[![CI](https://github.com/laurentlucian/fan/actions/workflows/ci.yml/badge.svg)](https://github.com/laurentlucian/fan/actions/workflows/ci.yml)

Play one audio stream on several output devices at once.
Pick your speakers, headset, and anything else — hit Start.

## Download

[FAN-Setup.exe](https://github.com/laurentlucian/fan/releases/download/latest/FAN-Setup.exe) — always the latest build.

## Requirements

Windows 10 1809+ or Windows 11, x64.

## How it works

FAN captures the Windows default output with WASAPI loopback and renders that stream to every device you select.
The source device is the Windows default and always plays — it is one of the outputs.

## Build from source

```
pnpm install
pnpm tauri build
```

## License

MIT
