use anyhow::{anyhow, Context, Result};
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::env;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

#[derive(Parser, Debug)]
#[command(name = "video-translator", version, about = "Add Traditional Chinese subtitles translated from Japanese audio to mp4 videos using OpenAI")] 
struct Args {
    /// Input MP4 video file
    #[arg(short, long)]
    input: PathBuf,

    /// Output SRT subtitle file (default: alongside input with .zh-TW.srt)
    #[arg(long)]
    output_srt: Option<PathBuf>,

    /// Output MP4 file with subtitles track muxed in (mov_text)
    #[arg(long)]
    output_mp4: Option<PathBuf>,

    /// Burn subtitles into the video (re-encode); implies --output-mp4
    #[arg(long, default_value_t = false)]
    burn_in: bool,

    /// Whisper model for transcription
    #[arg(long, default_value = "whisper-1")]
    whisper_model: String,

    /// Chat model for translation
    #[arg(long, default_value = "gpt-4o-mini")]
    translate_model: String,
}

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

    // API key
    let api_key = env::var("OPENAI_API_KEY")
        .context("Set OPENAI_API_KEY environment variable for OpenAI access")?;

    // Ensure ffmpeg exists
    ensure_ffmpeg()?;

    // Prepare outputs
    let output_srt = args
        .output_srt
        .unwrap_or_else(|| default_srt_path(&args.input));
    let output_mp4 = args.output_mp4.clone();

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

    // 2) Transcribe (Japanese) with Whisper
    progress.set_message("Transcribing Japanese audio (OpenAI Whisper)...");
    let transcript = transcribe_whisper_verbose(&wav_path, &api_key, &args.whisper_model).await?;
    let segments = transcript
        .segments
        .ok_or_else(|| anyhow!("No segments returned by Whisper (enable verbose_json support)"))?;

    if segments.is_empty() {
        return Err(anyhow!("Whisper returned zero segments"));
    }

    // 3) Translate to Traditional Chinese using GPT
    progress.set_message("Translating to Traditional Chinese (OpenAI GPT)...");
    let ja_lines: Vec<String> = segments.iter().map(|s| s.text.clone()).collect();
    let zh_lines = translate_lines_zh_tw(&ja_lines, &api_key, &args.translate_model).await?;
    if zh_lines.len() != ja_lines.len() {
        return Err(anyhow!(
            "Translation count mismatch: {} vs {}",
            zh_lines.len(),
            ja_lines.len()
        ));
    }

    // 4) Write SRT
    progress.set_message("Writing SRT subtitles...");
    write_srt(&output_srt, &segments, &zh_lines)?;

    // 5) Optionally mux or burn-in
    if args.burn_in || output_mp4.is_some() {
        let out_mp4 = output_mp4
            .unwrap_or_else(|| default_output_video_path(&args.input, args.burn_in));
        if args.burn_in {
            progress.set_message("Burning subtitles into video (re-encode with ffmpeg)...");
            burn_in_subtitles(&args.input, &output_srt, &out_mp4)?;
        } else {
            progress.set_message("Muxing subtitles track into MP4 (mov_text)...");
            mux_subtitles(&args.input, &output_srt, &out_mp4)?;
        }
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

async fn transcribe_whisper_verbose(wav_path: &Path, api_key: &str, model: &str) -> Result<WhisperVerboseJson> {
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

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Debug, Deserialize)]
struct ChatMessage {
    content: String,
}

async fn translate_lines_zh_tw(lines: &[String], api_key: &str, model: &str) -> Result<Vec<String>> {
    if lines.is_empty() {
        return Ok(vec![]);
    }

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
        "temperature": 0,
        // response_format json_object is supported by newer models; fallback to instruction-only if not supported.
        "response_format": {"type": "json_object"},
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

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow!("OpenAI translation error {}: {}", status, text));
    }

    // Parse JSON content from message
    let raw: serde_json::Value = resp.json().await.context("Parse chat response JSON")?;
    let content = raw["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| anyhow!("Unexpected chat response structure"))?;

    let parsed: serde_json::Value = serde_json::from_str(content)
        .context("Chat content not valid JSON; model may not support json_object format")?;
    let arr = parsed["translations"].as_array().ok_or_else(|| anyhow!(
        "Translation JSON missing 'translations' array"
    ))?;
    let mut out = Vec::with_capacity(arr.len());
    for v in arr {
        out.push(v.as_str().unwrap_or("").to_string());
    }
    Ok(out)
}

fn write_srt(path: &Path, segments: &[WhisperSegment], lines: &[String]) -> Result<()> {
    use std::io::Write;
    let mut f = std::fs::File::create(path).with_context(|| format!("Create SRT at {}", path.display()))?;

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
    let mut out = input.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
    out.push(format!("{}.zh-TW.srt", base));
    out
}

fn default_output_video_path(input: &Path, burn_in: bool) -> PathBuf {
    let mut p = input.to_path_buf();
    p.set_extension("");
    let base = p.file_name().and_then(|s| s.to_str()).unwrap_or("output");
    let mut out = input.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
    if burn_in {
        out.push(format!("{}.zh-TW.burned.mp4", base));
    } else {
        out.push(format!("{}.zh-TW.muxed.mp4", base));
    }
    out
}

fn mux_subtitles(input: &Path, srt: &Path, out: &Path) -> Result<()> {
    // Add SRT as mov_text subtitles track without re-encoding video
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-i",
            input.to_str().unwrap(),
            "-i",
            srt.to_str().unwrap(),
            "-c",
            "copy",
            "-c:s",
            "mov_text",
            "-metadata:s:s:0",
            "language=zht",
            out.to_str().unwrap(),
        ])
        .status()
        .context("ffmpeg mux subtitles failed")?;
    if !status.success() {
        return Err(anyhow!("ffmpeg muxing failed"));
    }
    Ok(())
}

fn burn_in_subtitles(input: &Path, srt: &Path, out: &Path) -> Result<()> {
    // Burn subtitles using subtitles filter (requires libass). Re-encodes video.
    let filter = format!("subtitles={}", escape_for_ffmpeg(srt));
    let status = Command::new("ffmpeg")
        .args([
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
    s.replace("\\", "\\\\").replace(":", "\\:").replace("=", "\\=")
}

