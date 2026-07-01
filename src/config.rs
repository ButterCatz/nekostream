use anyhow::{Context, Result};
use std::{env, net::SocketAddr};

#[derive(Clone, Debug)]
pub enum AudioInputMode {
    RawTcp,
    FfmpegUrl,
}

impl AudioInputMode {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "raw_tcp" => Some(Self::RawTcp),
            "ffmpeg_url" => Some(Self::FfmpegUrl),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub discord_token: String,
    pub audio_bind_addr: SocketAddr,
    pub audio_sample_rate: u32,
    pub audio_channels: u32,
    pub audio_input_mode: AudioInputMode,
    pub ffmpeg_bin: String,
    pub ffmpeg_input_url: Option<String>,
    pub bot_locale: String,
    pub message_templates_path: String,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        dotenvy::dotenv().ok();

        let discord_token = env::var("DISCORD_TOKEN").context("DISCORD_TOKEN is required")?;
        let audio_bind_addr = env::var("AUDIO_BIND_ADDR")
            .unwrap_or_else(|_| "0.0.0.0:3030".to_string())
            .parse()
            .context("AUDIO_BIND_ADDR must be a valid socket address")?;
        let audio_sample_rate = env::var("AUDIO_SAMPLE_RATE")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(48_000);
        let audio_channels = env::var("AUDIO_CHANNELS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(2);
        let audio_input_mode_raw = env::var("AUDIO_INPUT_MODE")
            .unwrap_or_else(|_| "raw_tcp".to_string())
            .to_lowercase();
        let audio_input_mode = AudioInputMode::parse(&audio_input_mode_raw).with_context(|| {
            format!(
                "AUDIO_INPUT_MODE must be one of: raw_tcp, ffmpeg_url (got: {audio_input_mode_raw})"
            )
        })?;
        let ffmpeg_bin = env::var("FFMPEG_BIN").unwrap_or_else(|_| "ffmpeg".to_string());
        let ffmpeg_input_url = env::var("FFMPEG_INPUT_URL").ok();
        let bot_locale = env::var("BOT_LOCALE").unwrap_or_else(|_| "zh-TW".to_string());
        let message_templates_path = env::var("MESSAGE_TEMPLATES_PATH")
            .unwrap_or_else(|_| "config/messages.toml".to_string());

        if matches!(audio_input_mode, AudioInputMode::FfmpegUrl) && ffmpeg_input_url.is_none() {
            anyhow::bail!("FFMPEG_INPUT_URL is required when AUDIO_INPUT_MODE=ffmpeg_url");
        }

        Ok(Self {
            discord_token,
            audio_bind_addr,
            audio_sample_rate,
            audio_channels,
            audio_input_mode,
            ffmpeg_bin,
            ffmpeg_input_url,
            bot_locale,
            message_templates_path,
        })
    }
}
