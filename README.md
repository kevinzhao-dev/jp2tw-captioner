# Video Translator (Rust)

Adds Traditional Chinese subtitles (translated from Japanese audio) to MP4 videos using OpenAI APIs.

## Features

- Extracts audio from MP4 via `ffmpeg` (16kHz mono WAV)
- Transcribes Japanese speech to text with OpenAI Whisper (`whisper-1`)
- Translates segment-by-segment to Traditional Chinese (Taiwan) using GPT (default: `gpt-4o-mini`)
- Generates `.srt` subtitles
- Optionally muxes SRT into MP4 as a subtitle track (no re-encode) or burns subtitles into the video (re-encode)

## Requirements

- `ffmpeg` installed and available in `PATH`
- OpenAI API key in environment: `OPENAI_API_KEY=sk-...`

## Quick Start

```bash
# Build
cargo build --release

# Basic: produce SRT alongside input
OPENAI_API_KEY=sk-... \
  ./target/release/video-translator \
  --input /path/to/video.mp4

# Custom SRT path
OPENAI_API_KEY=sk-... \
  ./target/release/video-translator \
  --input /path/to/video.mp4 \
  --output-srt /path/to/output.zh-TW.srt

# Mux subtitles track into MP4 (mov_text)
OPENAI_API_KEY=sk-... \
  ./target/release/video-translator \
  --input /path/to/video.mp4 \
  --output-mp4 /path/to/video.zh-TW.muxed.mp4

# Burn-in subtitles (re-encode video)
OPENAI_API_KEY=sk-... \
  ./target/release/video-translator \
  --input /path/to/video.mp4 \
  --burn-in \
  --output-mp4 /path/to/video.zh-TW.burned.mp4
```

## CLI Options

- `--input <FILE>`: Input MP4 path (required)
- `--output-srt <FILE>`: Output SRT path (optional; default: `input.zh-TW.srt`)
- `--output-mp4 <FILE>`: Output MP4 path with subtitles track or burned-in
- `--burn-in`: Burn subtitles into the video (re-encode; implies `--output-mp4`)
- `--whisper-model <NAME>`: Transcription model (default: `whisper-1`)
- `--translate-model <NAME>`: Translation chat model (default: `gpt-4o-mini`)

## Notes

- Transcription expects Japanese audio; `language` is set to `ja`.
- Translation prompts the model to return strict JSON; requires models supporting `response_format: { type: "json_object" }`. If a model rejects this, switch to another model (e.g., `gpt-4o`).
- Muxing uses `mov_text` codec; many players support toggling subtitles on/off.
- Burning uses `-vf subtitles=...` and re-encodes the video. Requires `ffmpeg` with `libass`.

## Project Goal (from AGENTS.md)

- Implement a project that adds Traditional Chinese subtitles (translated from Japanese audio) to videos.
- The video file is mp4
- Using OpenAI APIs (Whisper or GPT)
- Implemented in Rust

## Troubleshooting

- `ffmpeg not available in PATH`: Install via Homebrew (`brew install ffmpeg`), apt (`sudo apt-get install ffmpeg`), or Chocolatey (`choco install ffmpeg`).
- OpenAI errors: ensure `OPENAI_API_KEY` is set and billing/quota is available.
- No segments returned by Whisper: ensure the model supports `verbose_json` with segments; otherwise try another audio format or model.

## License

This project does not include a license header by default; consult repository owner for licensing.
