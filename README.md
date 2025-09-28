# jp2tw-captioner (Rust)

Adds Traditional Chinese subtitles (translated from Japanese audio) to MP4 videos using OpenAI APIs.

## Features

- Extracts audio from MP4 via `ffmpeg` (16kHz mono WAV)
- Transcribes Japanese speech to text with OpenAI Whisper (`whisper-1`)
- Translates segment-by-segment to Traditional Chinese (Taiwan) using GPT (default: `gpt-4o-mini`)
- Generates `.srt` subtitles
- Burned-in subtitles MP4 via `--output` (re-encode)

## Requirements

- `ffmpeg` installed and available in `PATH`
- OpenAI API key in environment: `OPENAI_API_KEY=sk-...`

### Using a .env file

This app loads a `.env` file automatically (via `dotenvy`) so you can avoid exporting the key in your shell each time.

1) Create a file named `.env` in the project root:

```
OPENAI_API_KEY=sk-your-key
```

2) Make sure `.env` is ignored by git (already configured).

3) Run the tool normally; it will read the key from `.env`:

```
cargo run --release -- --input /path/to/video.mp4
```

Alternative without code changes:

```
set -a; source .env; set +a
./target/release/jp2tw-captioner --input /path/to/video.mp4
```

## Quick Start

```bash
# Build
cargo build --release

# Basic: produce bilingual SRT and burned-in MP4 (default)
OPENAI_API_KEY=sk-... \
  ./target/release/jp2tw-captioner \
  --input /path/to/video.mp4

# Custom SRT path
OPENAI_API_KEY=sk-... \
  ./target/release/jp2tw-captioner \
  --input /path/to/video.mp4 \
  --output-srt /path/to/output.zh-TW.srt

# Burn-in subtitles with custom output name
OPENAI_API_KEY=sk-... \
  ./target/release/jp2tw-captioner \
  --input /path/to/video.mp4 \
  --output /path/to/video.zh.mp4

# Default filename if --output has no path (uses input.zh.mp4)
OPENAI_API_KEY=sk-... \
  ./target/release/jp2tw-captioner \
  --input /path/to/video.mp4 \
  --output

# Disable bilingual or adjust font size if desired
OPENAI_API_KEY=sk-... \
  ./target/release/jp2tw-captioner \
  --input /path/to/video.mp4 \
  --font-size 28

# Long video best practice (smaller chunks, smaller batches)
OPENAI_API_KEY=sk-... \
  ./target/release/jp2tw-captioner \
  --input /path/to/long-video.mp4 \
  --chunk-seconds 300 \
  --translate-batch-size 40 \
  --output
```

## CLI Options

- `--input <FILE>`: Input MP4 path (required)
- `--output-srt <FILE>`: Output SRT path (optional; default: `input.zh-TW.srt`)
- `--output <FILE>`: Output MP4 path (default name if omitted). Default behavior burns in subtitles and writes MP4.
- `--burn-in`: Burn subtitles into the video (re-encode). Default: on.
- `--bilingual`: Output bilingual subtitles (ZH first line, JP second). Default: on.
- `--whisper-model <NAME>`: Transcription model (default: `whisper-1`)
- `--translate-model <NAME>`: Translation chat model (default: `gpt-4o-mini`)
- `--translate-batch-size <N>`: Lines per translation batch (default: 60)
- `--chunk-seconds <N>`: Seconds per audio chunk for transcription (default: 600)
- `--font-dir <PATH>`: Fonts directory for burn-in (default: `./fonts`)
- `--font-name <NAME>`: Font family for burn-in (default: `Noto Sans CJK TC`)
- `--font-size <N>`: Font size for burn-in (ASS). Defaults to 36, or 30 when `--bilingual`.

## Fonts for Burn-in

For burned-in subtitles, ffmpeg/libass must find a font with Traditional Chinese glyphs. Install Noto CJK and prepare a local fonts folder for reliable results.

1) Install fonts

macOS (Homebrew):

```
brew install --cask font-noto-sans-cjk
brew install --cask font-noto-serif-cjk
```

Linux (Debian/Ubuntu):

```
sudo apt-get install -y fonts-noto-cjk
```

2) Prepare project fonts directory

```
scripts/prepare_fonts.sh  # copies Noto CJK TC fonts into ./fonts
```

3) Use the fonts directory

```
./target/release/jp2tw-captioner \
  --input /path/to/video.mp4 \
  --font-dir ./fonts \
  --font-name "Noto Sans CJK TC" \
  --output /path/to/video.zh.mp4

# or set an env var so --font-dir is optional
export JP2TW_CAPTIONER_FONTS_DIR=./fonts
```

The app automatically prefers a local `./fonts` directory if present. For backward compatibility, it also respects `VIDEO_TRANSLATOR_FONTS_DIR`.

## Performance Tips

- Long videos: If you see intermittent 500/502/503/429 errors, try smaller chunks, e.g. `--chunk-seconds 300`.
- Large subtitle counts: If a batch errors or returns the wrong count, the tool falls back to smaller batches or single-line translation automatically. You can also lower `--translate-batch-size` (e.g., 40).
- Bilingual sizing: Use `--font-size` to fine-tune legibility. Defaults to 30 for bilingual, 36 otherwise.

## Notes

- Transcription expects Japanese audio; `language` is set to `ja`.
- Translation prompts the model to return strict JSON; requires models supporting `response_format: { type: "json_object" }`. If a model rejects this, switch to another model (e.g., `gpt-4o`).
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
- Rectangles instead of Chinese text (burn-in): install Noto CJK fonts and run `scripts/prepare_fonts.sh`, or set `--font-dir` to a folder containing a CJK-capable font and `--font-name` to its family name.
- ffmpeg interactive prompt noise: suppressed via `-nostdin` in all calls.

## License

This project does not include a license header by default; consult repository owner for licensing.
