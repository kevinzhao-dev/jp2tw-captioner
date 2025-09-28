use anyhow::{anyhow, Context, Result};
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::header::CONTENT_TYPE;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::env;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;
use tokio::time::{sleep, Duration};

#[derive(Parser, Debug)]
#[command(
    name = "jp2tw-captioner",
    version,
    about = "JP→TW captioner: add Traditional Chinese subtitles (translated from Japanese audio) to MP4 videos using OpenAI"
)]
struct Args {
    /// Input MP4 video file
    #[arg(short, long)]
    input: PathBuf,

    /// Output SRT subtitle file (default: alongside input with .zh-TW.srt)
    #[arg(long)]
    output_srt: Option<PathBuf>,

    /// Output MP4 file path (default name if omitted). Can be passed without a value.
    #[arg(long = "output", num_args(0..=1), default_missing_value = "__AUTO__")]
    output: Option<String>,

    /// Burn subtitles into the video (re-encode). Default: on.
    #[arg(long, default_value_t = true)]
    burn_in: bool,

    /// Output bilingual subtitles (ZH first line, JP second line). Default: on.
    #[arg(long, default_value_t = true)]
    bilingual: bool,

    /// Directory containing fonts for burn-in (libass fontsdir)
    #[arg(long, default_value = "./fonts")]
    font_dir: Option<PathBuf>,

    /// Preferred font family name for burn-in (e.g., "Noto Sans CJK TC")
    #[arg(long, default_value = "Noto Sans CJK TC")]
    font_name: Option<String>,

    /// Font size for burn-in (ASS). If not set, uses 36 normally, 30 when --bilingual.
    #[arg(long)]
    font_size: Option<u32>,

    /// Whisper model for transcription
    #[arg(long, default_value = "whisper-1")]
    whisper_model: String,

    /// Max seconds per audio chunk for transcription
    #[arg(long, default_value_t = 600)]
    chunk_seconds: u32,

    /// Chat model for translation
    #[arg(long, default_value = "gpt-4o-mini")]
    translate_model: String,
    /// Max subtitle lines per translation batch
    #[arg(long, default_value_t = 60)]
    translate_batch_size: usize,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct WhisperVerboseJson {
    text: Option<String>,
    segments: Option<Vec<WhisperSegment>>, // Some SDKs omit this unless requested
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct WhisperSegment {
    id: Option<u32>,
    start: f64,
    end: f64,
    text: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Validate input
    if !args.input.exists() {
        return Err(anyhow!("Input file not found: {}", args.input.display()));
    }
    if args.input.extension().and_then(|s| s.to_str()) != Some("mp4") {
        eprintln!("Warning: input is not .mp4; proceeding anyway");
    }

    // Load .env if present, then read API key
    let _ = dotenvy::dotenv();
    let api_key = env::var("OPENAI_API_KEY")
        .context("Set OPENAI_API_KEY environment variable for OpenAI access")?;

    // Ensure ffmpeg exists
    ensure_ffmpeg()?;

    // Prepare outputs
    let output_srt = args
        .output_srt
        .unwrap_or_else(|| default_srt_path(&args.input));
    // Resolve output path behavior: if --output provided without path, pick default derived from input
    let output_mp4: Option<PathBuf> = match args.output.as_deref() {
        None => None,
        Some("__AUTO__") | Some("") => Some(default_output_video_path(&args.input)),
        Some(s) => Some(PathBuf::from(s)),
    };

    let progress = ProgressBar::new_spinner();
    progress.set_style(
        ProgressStyle::with_template("{spinner} {msg}")
            .unwrap()
            .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ "),
    );

    // 1) Extract audio
    progress.set_message("Extracting audio with ffmpeg...");
    let tmp = tempdir()?;
    let wav_path = tmp.path().join("audio_16k_mono.wav");
    extract_audio(&args.input, &wav_path)?;

    // 2) Transcribe (Japanese) with Whisper (chunked for long videos)
    progress.set_message("Transcribing Japanese audio (OpenAI Whisper)...");
    let segments =
        transcribe_whisper_chunked(&wav_path, &api_key, &args.whisper_model, args.chunk_seconds)
            .await?;

    if segments.is_empty() {
        return Err(anyhow!("Whisper returned zero segments"));
    }

    // 3) Translate to Traditional Chinese using GPT
    progress.set_message("Translating to Traditional Chinese (OpenAI GPT)...");
    let ja_lines: Vec<String> = segments.iter().map(|s| s.text.clone()).collect();
    let zh_lines = translate_lines_zh_tw(
        &ja_lines,
        &api_key,
        &args.translate_model,
        args.translate_batch_size,
    )
    .await?;
    // Build display lines (bilingual or zh-only)
    let display_lines: Vec<String> = if args.bilingual {
        ja_lines
            .iter()
            .zip(zh_lines.iter())
            .map(|(ja, zh)| format!("{}\n{}", zh, ja))
            .collect()
    } else {
        zh_lines.clone()
    };
    if zh_lines.len() != ja_lines.len() {
        return Err(anyhow!(
            "Translation count mismatch: {} vs {}",
            zh_lines.len(),
            ja_lines.len()
        ));
    }

    // 4) Write SRT
    progress.set_message("Writing SRT subtitles...");
    write_srt(&output_srt, &segments, &display_lines)?;

    // 5) Optionally produce MP4 (default: burn-in)
    if args.burn_in || output_mp4.is_some() {
        let out_mp4 = output_mp4.unwrap_or_else(|| default_output_video_path(&args.input));
        // Default behavior is burn-in, even if --burn-in not explicitly set
        progress.set_message("Burning subtitles into video (re-encode with ffmpeg)...");
        // Prepare an ASS file with an explicit font to avoid missing glyphs
        let ass_path = tmp.path().join("subs.ass");
        // Prefer Noto to avoid platform-private font issues
        let default_font = "Noto Sans CJK TC";
        let chosen_font = args.font_name.as_deref().unwrap_or(default_font);
        let font_size = args
            .font_size
            .unwrap_or(if args.bilingual { 30 } else { 36 });
        write_ass(&ass_path, &segments, &display_lines, chosen_font, font_size)?;

        // Try provided fonts dir or detect common/project fonts locations
        let fonts_dir = resolve_fonts_dir(args.font_dir.as_deref());
        if let Some(ref d) = fonts_dir {
            eprintln!("Using fonts dir: {}", d.display());
        } else {
            eprintln!("Warning: no fonts dir found; relying on system fallback. You can run scripts/prepare_fonts.sh");
        }
        burn_in_subtitles(&args.input, &ass_path, &out_mp4, fonts_dir.as_deref(), None)?;
        progress.finish_with_message(format!(
            "Done. SRT: {} | Video: {}",
            output_srt.display(),
            out_mp4.display()
        ));
    } else {
        progress.finish_with_message(format!("Done. SRT written to {}", output_srt.display()));
    }

    Ok(())
}

fn ensure_ffmpeg() -> Result<()> {
    let status = Command::new("ffmpeg")
        .arg("-version")
        .status()
        .context("ffmpeg is required (install via brew/apt/choco)")?;
    if !status.success() {
        return Err(anyhow!("ffmpeg not available in PATH"));
    }
    Ok(())
}

fn extract_audio(input: &Path, wav_out: &Path) -> Result<()> {
    // 16kHz mono PCM WAV
    let status = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-y",
            "-i",
            input.to_str().unwrap(),
            "-vn",
            "-acodec",
            "pcm_s16le",
            "-ar",
            "16000",
            "-ac",
            "1",
            wav_out.to_str().unwrap(),
        ])
        .status()
        .context("Failed to run ffmpeg to extract audio")?;
    if !status.success() {
        return Err(anyhow!("ffmpeg audio extraction failed"));
    }
    Ok(())
}

async fn transcribe_whisper_verbose(
    wav_path: &Path,
    api_key: &str,
    model: &str,
) -> Result<WhisperVerboseJson> {
    let client = reqwest::Client::new();

    let mut file = File::open(wav_path).context("Open audio file for transcription")?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;

    let part = reqwest::multipart::Part::bytes(buf)
        .file_name(
            wav_path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("audio.wav")
                .to_string(),
        )
        .mime_str("audio/wav")?;

    let form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("model", model.to_string())
        .text("response_format", "verbose_json".to_string())
        .text("language", "ja".to_string())
        // Ask for segment timestamps if supported
        .text("timestamp_granularities[]", "segment".to_string());

    let resp = client
        .post("https://api.openai.com/v1/audio/transcriptions")
        .bearer_auth(api_key)
        .multipart(form)
        .send()
        .await
        .context("OpenAI transcription request failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow!("OpenAI transcription error {}: {}", status, text));
    }

    let json: WhisperVerboseJson = resp.json().await.context("Parse Whisper response JSON")?;
    Ok(json)
}

async fn transcribe_whisper_chunked(
    wav_path: &Path,
    api_key: &str,
    model: &str,
    chunk_seconds: u32,
) -> Result<Vec<WhisperSegment>> {
    // Split the audio into chunked WAV files using ffmpeg segmenter
    let out_dir = wav_path.parent().unwrap_or_else(|| Path::new("."));
    let pattern = out_dir.join("chunk_%05d.wav");

    // Remove any prior chunk files with same pattern
    // Best-effort cleanup; ignore errors
    if let Ok(entries) = std::fs::read_dir(out_dir) {
        for e in entries.flatten() {
            let p = e.path();
            if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                if name.starts_with("chunk_") && name.ends_with(".wav") {
                    let _ = std::fs::remove_file(p);
                }
            }
        }
    }

    let status = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-y",
            "-i",
            wav_path.to_str().unwrap(),
            "-f",
            "segment",
            "-segment_time",
            &chunk_seconds.to_string(),
            "-c",
            "copy",
            pattern.to_str().unwrap(),
        ])
        .status()
        .context("ffmpeg segmenting failed")?;
    if !status.success() {
        return Err(anyhow!("ffmpeg failed to segment audio"));
    }

    // Collect chunk files sorted
    let mut chunks: Vec<PathBuf> = std::fs::read_dir(out_dir)
        .context("read chunk dir")?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .map(|n| n.starts_with("chunk_") && n.ends_with(".wav"))
                .unwrap_or(false)
        })
        .collect();
    chunks.sort();
    if chunks.is_empty() {
        return Err(anyhow!("No audio chunks were produced"));
    }

    let mut all: Vec<WhisperSegment> = Vec::new();
    for (i, chunk) in chunks.iter().enumerate() {
        eprintln!(
            "Transcribing chunk {}/{}: {}",
            i + 1,
            chunks.len(),
            chunk.display()
        );

        // Retry on transient errors (5xx/429) with exponential backoff
        let mut attempt = 0;
        let max_attempts = 5;
        let mut last_err: Option<anyhow::Error> = None;
        let res: Option<WhisperVerboseJson> = loop {
            match transcribe_whisper_verbose(chunk, api_key, model).await {
                Ok(json) => break Some(json),
                Err(e) => {
                    let msg = format!("{}", e);
                    // Retry for server errors or rate limits
                    if msg.contains(" 500 ")
                        || msg.contains(" 502 ")
                        || msg.contains(" 503 ")
                        || msg.contains("429")
                    {
                        attempt += 1;
                        if attempt >= max_attempts {
                            last_err = Some(e);
                            break None;
                        }
                        let backoff = 2u64.pow(attempt) * 1000; // ms
                        eprintln!(
                            "OpenAI error (attempt {}/{}). Retrying in {}ms...",
                            attempt, max_attempts, backoff
                        );
                        sleep(Duration::from_millis(backoff)).await;
                    } else {
                        last_err = Some(e);
                        break None;
                    }
                }
            }
        };
        let json = res.ok_or_else(|| last_err.unwrap())?;

        let mut segs = json.segments.ok_or_else(|| {
            anyhow!(
                "No segments returned by Whisper (verbose_json) for chunk {}",
                i
            )
        })?;
        let offset = (i as f64) * (chunk_seconds as f64);
        for s in segs.iter_mut() {
            s.start += offset;
            s.end += offset;
        }
        all.extend(segs.into_iter());
    }

    Ok(all)
}

// (Removed unused ChatResponse/ChatChoice/ChatMessage)

async fn translate_lines_zh_tw(
    lines: &[String],
    api_key: &str,
    model: &str,
    batch_size: usize,
) -> Result<Vec<String>> {
    if lines.is_empty() {
        return Ok(vec![]);
    }

    let mut result = Vec::with_capacity(lines.len());
    let mut idx = 0;
    while idx < lines.len() {
        let end = usize::min(idx + batch_size.max(1), lines.len());
        let batch = &lines[idx..end];
        let translated = translate_batch_strict(batch, api_key, model).await?;
        result.extend(translated);
        idx = end;
    }
    Ok(result)
}

async fn translate_batch_strict(
    lines: &[String],
    api_key: &str,
    model: &str,
) -> Result<Vec<String>> {
    let n = lines.len();
    let mut out: Vec<Option<String>> = vec![None; n];
    let mut stack: Vec<(usize, usize)> = Vec::new();
    if n > 0 {
        stack.push((0, n));
    }

    while let Some((start, end)) = stack.pop() {
        let len = end - start;
        if len == 0 {
            continue;
        }
        match translate_batch(&lines[start..end], api_key, model).await {
            Ok(v) if v.len() == len => {
                for (i, t) in v.into_iter().enumerate() {
                    out[start + i] = Some(t);
                }
            }
            Ok(_) | Err(_) => {
                if len == 1 {
                    let t = translate_single_fallback(&lines[start], api_key, model).await?;
                    out[start] = Some(t);
                } else {
                    let mid = start + len / 2;
                    // Process right later, left first
                    stack.push((mid, end));
                    stack.push((start, mid));
                }
            }
        }
    }

    // Collect and ensure all present
    let mut result = Vec::with_capacity(n);
    for (i, slot) in out.iter_mut().enumerate() {
        if let Some(t) = slot.take() {
            result.push(t);
        } else {
            return Err(anyhow!("Failed to translate line {}", i));
        }
    }
    Ok(result)
}

async fn translate_batch(lines: &[String], api_key: &str, model: &str) -> Result<Vec<String>> {
    let client = reqwest::Client::new();
    // Instruct model to return strict JSON
    let system = "You are a professional translator. Translate Japanese to Traditional Chinese (Taiwan). Keep meaning, tone, and honorific nuance. Do not add explanations.";

    let user = json!({
        "instruction": "Translate each item to Traditional Chinese. Return strict JSON with {\"translations\": string[]} matching the input length.",
        "source_language": "ja",
        "target_language": "zh-TW",
        "items": lines,
    })
    .to_string();

    let body = json!({
        "model": model,
        // response_format json_object is supported by newer models; fallback to instruction-only if not supported.
        "response_format": {"type": "json_object"},
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user}
        ]
    });

    // Retry on transient errors similar to transcription
    let mut attempt = 0;
    let max_attempts = 5;
    let raw: serde_json::Value = loop {
        let resp = client
            .post("https://api.openai.com/v1/chat/completions")
            .bearer_auth(api_key)
            .header(CONTENT_TYPE, "application/json")
            .body(body.to_string())
            .send()
            .await
            .context("OpenAI translation request failed")?;

        if resp.status().is_success() {
            break resp.json().await.context("Parse chat response JSON")?;
        } else {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            let msg = format!("{} {}", status, text);
            if msg.contains(" 500 ")
                || msg.contains(" 502 ")
                || msg.contains(" 503 ")
                || msg.contains("429")
            {
                attempt += 1;
                if attempt >= max_attempts {
                    return Err(anyhow!("OpenAI translation error {}: {}", status, text));
                }
                let backoff = 2u64.pow(attempt) * 1000;
                eprintln!(
                    "Translation retry {}/{} after error (status {}), waiting {}ms",
                    attempt, max_attempts, status, backoff
                );
                sleep(Duration::from_millis(backoff)).await;
                continue;
            } else {
                return Err(anyhow!("OpenAI translation error {}: {}", status, text));
            }
        }
    };

    let content = raw["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| anyhow!("Unexpected chat response structure"))?;

    // Be tolerant: try content directly, then strip code fences, then find braces
    if let Some(v) = try_parse_translations_json(content) {
        return Ok(v);
    }
    // Fallback: try to slice out the first {...} block
    let json_obj = extract_first_json_object(content).and_then(|s| try_parse_translations_json(&s));
    if let Some(v) = json_obj {
        return Ok(v);
    }

    Err(anyhow!("Translation JSON missing 'translations' array"))
}

fn try_parse_translations_json(s: &str) -> Option<Vec<String>> {
    let trimmed = s.trim();
    let candidate = if trimmed.starts_with("```") {
        // Possible fenced code block
        trimmed
            .trim_start_matches("```json")
            .trim_start_matches("```JSON")
            .trim_start_matches("```) ")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim()
            .to_string()
    } else {
        trimmed.to_string()
    };
    match serde_json::from_str::<serde_json::Value>(&candidate) {
        Ok(v) => v["translations"].as_array().map(|arr| {
            arr.iter()
                .map(|x| x.as_str().unwrap_or("").to_string())
                .collect::<Vec<_>>()
        }),
        Err(_) => None,
    }
}

fn extract_first_json_object(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut start: Option<usize> = None;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'{' {
            if depth == 0 {
                start = Some(i);
            }
            depth += 1;
        } else if b == b'}' {
            depth -= 1;
            if depth == 0 {
                if let Some(st) = start {
                    return Some(s[st..=i].to_string());
                }
            }
        }
    }
    None
}

async fn translate_single_fallback(text: &str, api_key: &str, model: &str) -> Result<String> {
    let client = reqwest::Client::new();
    let system = "You are a professional translator. Translate Japanese to Traditional Chinese (Taiwan). Output only the translated text without quotes or explanations.";
    let user = text;

    // Retry similar to batch
    let mut attempt = 0;
    let max_attempts = 5;
    loop {
        let body = json!({
            "model": model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user}
            ]
        });
        let resp = client
            .post("https://api.openai.com/v1/chat/completions")
            .bearer_auth(api_key)
            .header(CONTENT_TYPE, "application/json")
            .body(body.to_string())
            .send()
            .await
            .context("OpenAI translation request failed")?;
        if resp.status().is_success() {
            let raw: serde_json::Value = resp.json().await.context("Parse chat response JSON")?;
            let content = raw["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or("")
                .trim()
                .to_string();
            // Strip surrounding quotes if any
            let cleaned = content.trim_matches('"').to_string();
            return Ok(cleaned);
        } else {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            let msg = format!("{} {}", status, text);
            if msg.contains(" 500 ")
                || msg.contains(" 502 ")
                || msg.contains(" 503 ")
                || msg.contains("429")
            {
                attempt += 1;
                if attempt >= max_attempts {
                    return Err(anyhow!("OpenAI translation error {}: {}", status, text));
                }
                let backoff = 2u64.pow(attempt) * 1000;
                eprintln!(
                    "Single translation retry {}/{} after error (status {}), waiting {}ms",
                    attempt, max_attempts, status, backoff
                );
                sleep(Duration::from_millis(backoff)).await;
                continue;
            } else {
                return Err(anyhow!("OpenAI translation error {}: {}", status, text));
            }
        }
    }
}

fn write_srt(path: &Path, segments: &[WhisperSegment], lines: &[String]) -> Result<()> {
    use std::io::Write;
    let mut f =
        std::fs::File::create(path).with_context(|| format!("Create SRT at {}", path.display()))?;

    for (i, (seg, text)) in segments.iter().zip(lines.iter()).enumerate() {
        let idx = i + 1;
        let start = format_srt_time(seg.start);
        let end = format_srt_time(seg.end);
        writeln!(f, "{}\n{} --> {}\n{}\n", idx, start, end, text)?;
    }
    Ok(())
}

fn format_srt_time(seconds: f64) -> String {
    // HH:MM:SS,mmm
    let total_ms = (seconds * 1000.0).round() as i64;
    let ms = total_ms % 1000;
    let total_secs = total_ms / 1000;
    let s = total_secs % 60;
    let total_mins = total_secs / 60;
    let m = total_mins % 60;
    let h = total_mins / 60;
    format!("{:02}:{:02}:{:02},{:03}", h, m, s, ms)
}

fn default_srt_path(input: &Path) -> PathBuf {
    let mut p = input.to_path_buf();
    p.set_extension("");
    let base = p.file_name().and_then(|s| s.to_str()).unwrap_or("output");
    let mut out = input
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    out.push(format!("{}.zh-TW.srt", base));
    out
}

fn default_output_video_path(input: &Path) -> PathBuf {
    let mut p = input.to_path_buf();
    p.set_extension("");
    let base = p.file_name().and_then(|s| s.to_str()).unwrap_or("output");
    let mut out = input
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    out.push(format!("{}.zh.mp4", base));
    out
}

// (Removed unused mux_subtitles)

fn burn_in_subtitles(
    input: &Path,
    subs: &Path,
    out: &Path,
    fonts_dir: Option<&Path>,
    font_name: Option<&str>,
) -> Result<()> {
    // Burn subtitles using subtitles filter (requires libass). Re-encodes video.
    let mut filter = format!("subtitles={}", escape_for_ffmpeg(subs));
    if let Some(dir) = fonts_dir {
        filter.push_str(":fontsdir=");
        filter.push_str(&escape_for_ffmpeg(dir));
    }
    // If an ASS file was generated with a Style font, don't override via force_style.
    // Only apply force_style for plain SRT inputs when an explicit font is requested.
    if subs
        .extension()
        .and_then(|s| s.to_str())
        .map(|e| e.eq_ignore_ascii_case("ass"))
        == Some(false)
    {
        if let Some(name) = font_name {
            let safe = name.replace("'", "\\'");
            filter.push_str(":force_style=");
            filter.push_str(&format!("'FontName={}'", safe));
        }
    }
    let status = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-y",
            "-i",
            input.to_str().unwrap(),
            "-vf",
            &filter,
            "-c:a",
            "copy",
            out.to_str().unwrap(),
        ])
        .status()
        .context("ffmpeg burn-in subtitles failed")?;
    if !status.success() {
        return Err(anyhow!("ffmpeg burn-in failed"));
    }
    Ok(())
}

fn escape_for_ffmpeg(path: &Path) -> String {
    // Basic escaping for spaces and special chars in filter args
    let s = path.to_string_lossy();
    s.replace("\\", "\\\\")
        .replace(":", "\\:")
        .replace("=", "\\=")
}

fn write_ass(
    path: &Path,
    segments: &[WhisperSegment],
    lines: &[String],
    font_name: &str,
    font_size: u32,
) -> Result<()> {
    use std::io::Write;
    let mut f =
        std::fs::File::create(path).with_context(|| format!("Create ASS at {}", path.display()))?;

    // Basic ASS header with a single style
    writeln!(f, "[Script Info]")?;
    writeln!(f, "ScriptType: v4.00+")?;
    writeln!(f, "WrapStyle: 0")?;
    writeln!(f, "ScaledBorderAndShadow: yes")?;
    writeln!(f, "YCbCr Matrix: TV.601")?;
    writeln!(f)?;
    writeln!(f, "[V4+ Styles]")?;
    writeln!(f, "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding")?;
    let font = font_name.replace(",", " ");
    // White text, black outline/shadow, bottom-center
    writeln!(f, "Style: Default,{font},{font_size},&H00FFFFFF,&H000000FF,&H00000000,&H64000000,0,0,0,0,100,100,0,0,1,2,0,2,10,10,20,1")?;
    writeln!(f)?;
    writeln!(f, "[Events]")?;
    writeln!(
        f,
        "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text"
    )?;

    for (seg, text) in segments.iter().zip(lines.iter()) {
        let start = format_ass_time(seg.start);
        let end = format_ass_time(seg.end);
        let mut t = text.replace("\n", "\\N");
        t = t.replace("{", "(").replace("}", ")");
        writeln!(f, "Dialogue: 0,{start},{end},Default,,0,0,0,,{t}")?;
    }
    Ok(())
}

fn format_ass_time(seconds: f64) -> String {
    // h:mm:ss.cs (centiseconds)
    let total_cs = (seconds * 100.0).round() as i64;
    let cs = total_cs % 100;
    let total_secs = total_cs / 100;
    let s = total_secs % 60;
    let total_mins = total_secs / 60;
    let m = total_mins % 60;
    let h = total_mins / 60;
    format!("{}:{:02}:{:02}.{:02}", h, m, s, cs)
}

fn detect_default_fonts_dir() -> Option<PathBuf> {
    // Try common system fonts directories to help libass find CJK glyphs
    let mut candidates: Vec<PathBuf> = Vec::new();

    // Highest priority: env override (new name), fallback to legacy var
    if let Ok(env_dir) = std::env::var("JP2TW_CAPTIONER_FONTS_DIR") {
        let p = PathBuf::from(env_dir);
        if p.exists() {
            return Some(p);
        }
    }
    if let Ok(env_dir) = std::env::var("VIDEO_TRANSLATOR_FONTS_DIR") {
        let p = PathBuf::from(env_dir);
        if p.exists() {
            return Some(p);
        }
    }

    // Project-local fonts folder next
    if let Ok(cwd) = std::env::current_dir() {
        let project_fonts = cwd.join("fonts");
        if project_fonts.exists() {
            return Some(project_fonts);
        }
    }
    if cfg!(target_os = "macos") {
        candidates.push(PathBuf::from("/System/Library/Fonts"));
        candidates.push(PathBuf::from("/Library/Fonts"));
        if let Ok(home) = std::env::var("HOME") {
            candidates.push(PathBuf::from(format!("{home}/Library/Fonts")));
        }
    } else if cfg!(target_os = "windows") {
        candidates.push(PathBuf::from("C:/Windows/Fonts"));
    } else {
        candidates.push(PathBuf::from("/usr/share/fonts"));
        candidates.push(PathBuf::from("/usr/local/share/fonts"));
        candidates.push(PathBuf::from("/usr/share/fonts/truetype"));
    }

    candidates.into_iter().find(|pb| pb.exists())
}

fn resolve_fonts_dir(preferred: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = preferred {
        if p.exists() {
            return Some(p.to_path_buf());
        }
    }
    // Prefer project-local ./fonts if present
    if let Ok(cwd) = std::env::current_dir() {
        let p = cwd.join("fonts");
        if p.exists() {
            return Some(p);
        }
    }
    // Fall back to env/system detection
    detect_default_fonts_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_srt_time() {
        assert_eq!(format_srt_time(0.0), "00:00:00,000");
        assert_eq!(format_srt_time(1.234), "00:00:01,234");
        assert_eq!(format_srt_time(3661.234), "01:01:01,234");
    }

    #[test]
    fn test_format_ass_time() {
        assert_eq!(format_ass_time(0.0), "0:00:00.00");
        assert_eq!(format_ass_time(1.23), "0:00:01.23");
        assert_eq!(format_ass_time(3661.23), "1:01:01.23");
    }

    #[test]
    fn test_default_paths() {
        let input = PathBuf::from("/tmp/sample.mp4");
        let srt = default_srt_path(&input);
        assert_eq!(srt, PathBuf::from("/tmp/sample.zh-TW.srt"));

        let mp4 = default_output_video_path(&input);
        assert_eq!(mp4, PathBuf::from("/tmp/sample.zh.mp4"));
    }

    #[test]
    fn test_escape_for_ffmpeg() {
        let p = PathBuf::from("/a:b=c\\ d");
        let esc = escape_for_ffmpeg(&p);
        // ":" -> "\\:", "=" -> "\\=", "\\" -> "\\\\"
        assert!(esc.contains("\\:"));
        assert!(esc.contains("\\="));
        assert!(esc.contains("\\\\"));
    }

    #[test]
    fn test_write_srt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.srt");
        let segments = vec![
            WhisperSegment {
                id: Some(0),
                start: 0.0,
                end: 1.0,
                text: "JA0".into(),
            },
            WhisperSegment {
                id: Some(1),
                start: 2.5,
                end: 3.75,
                text: "JA1".into(),
            },
        ];
        let lines = vec!["你好".to_string(), "世界".to_string()];
        write_srt(&path, &segments, &lines).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let expected =
            "1\n00:00:00,000 --> 00:00:01,000\n你好\n\n2\n00:00:02,500 --> 00:00:03,750\n世界\n\n";
        assert_eq!(content, expected);
    }

    #[test]
    fn test_write_ass() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.ass");
        let segments = vec![
            WhisperSegment {
                id: Some(0),
                start: 0.0,
                end: 1.0,
                text: "{JA0}".into(),
            },
            WhisperSegment {
                id: Some(1),
                start: 2.5,
                end: 3.75,
                text: "line1\nline2".into(),
            },
        ];
        let lines = vec!["你好".to_string(), "世界".to_string()];
        write_ass(&path, &segments, &lines, "My Font", 30).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("Style: Default,My Font,30"));
        // Curly braces in input are replaced in Dialogue text
        assert!(content.contains(",Default,,0,0,0,,你好"));
        // Newlines become \N in ASS
        assert!(content.contains("世界"));
        assert!(!content.contains("{JA0}"));
        assert!(content.contains("0:00:00.00"));
        assert!(content.contains("0:00:01.00"));
        assert!(content.contains("0:00:02.50"));
        assert!(content.contains("0:00:03.75"));
    }

    #[test]
    fn test_json_helpers() {
        // Plain JSON
        let s = r#"{"translations":["a","b"]}"#;
        let v = try_parse_translations_json(s).unwrap();
        assert_eq!(v, vec!["a", "b"]);

        // Fenced JSON
        let s2 = "```json\n{\n  \"translations\":[\"x\",\"y\"]\n}\n```";
        let v2 = try_parse_translations_json(s2).unwrap();
        assert_eq!(v2, vec!["x", "y"]);

        // Embedded JSON
        let s3 = "Here is your result:\n{\"translations\":[\"m\",\"n\"]}\nThanks";
        let obj = extract_first_json_object(s3).unwrap();
        let v3 = try_parse_translations_json(&obj).unwrap();
        assert_eq!(v3, vec!["m", "n"]);
    }

    #[test]
    fn test_resolve_fonts_dir_prefers_provided() {
        let dir = tempfile::tempdir().unwrap();
        let chosen = resolve_fonts_dir(Some(dir.path()));
        assert_eq!(chosen.unwrap(), dir.path());
    }
}
