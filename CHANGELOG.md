# Changelog

## v1.0.0

- Initial release of jp2tw-captioner
- Adds Traditional Chinese subtitles (translated from Japanese) to MP4 videos
- Whisper transcription with chunking + retries for long videos
- GPT translation with batching + retries and tolerant JSON parsing
- Bilingual subtitles option (ZH first line, JP second)
- Burn-in output by default via `--output` (defaults to `input.zh.mp4`)
- Font handling for CJK with ASS + `--font-dir`, `--font-name`, `--font-size`
- Helper script `scripts/prepare_fonts.sh` for Noto CJK fonts
- Unit tests for core formatting and helpers
- CI: format, clippy, tests on push/PR
- Release workflow: builds binaries on tags `v*` and uploads artifacts
