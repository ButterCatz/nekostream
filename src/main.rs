mod bridge;
mod config;
mod messages;

use crate::{
    bridge::PcmBridge,
    config::{AppConfig, AudioInputMode},
    messages::MessageCatalog,
};
use anyhow::{bail, Context as AnyhowContext, Result};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serenity::{
    async_trait,
    builder::{
        CreateCommand, CreateCommandOption, CreateInteractionResponse,
        CreateInteractionResponseMessage,
    },
    client::{Client, Context as DiscordContext, EventHandler},
    model::{
        application::{Command, CommandInteraction, CommandOptionType, Interaction},
        gateway::Ready,
        id::{ChannelId, GuildId, UserId},
        voice::VoiceState,
    },
    prelude::GatewayIntents,
};
use songbird::{
    input::{Input, RawAdapter},
    SerenityInit,
};
use std::{
    collections::HashMap,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};
use tokio::{
    io::AsyncReadExt,
    net::{TcpListener, TcpStream},
    process::Command as TokioCommand,
    sync::{broadcast::Sender, RwLock},
    time::{sleep, Duration},
};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

const GUILDS_SETTINGS_PATH: &str = "data/guilds.json";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum AutoLeaveMode {
    Disabled,
    WhenNoHumans,
}

impl AutoLeaveMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::WhenNoHumans => "when_no_humans",
        }
    }

    fn from_option(value: &str) -> Option<Self> {
        match value {
            "disabled" => Some(Self::Disabled),
            "when_no_humans" => Some(Self::WhenNoHumans),
            _ => None,
        }
    }
}

#[derive(Clone)]
struct AudioIngress {
    bridge: Arc<PcmBridge>,
}

struct GuildState {
    track: Option<songbird::tracks::TrackHandle>,
}

#[derive(Serialize, Deserialize, Default)]
struct AutoLeaveStore {
    guild_modes: HashMap<u64, AutoLeaveMode>,
}

struct AutoLeaveSettings {
    path: PathBuf,
    modes: RwLock<HashMap<GuildId, AutoLeaveMode>>,
}

impl AutoLeaveSettings {
    async fn load(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let store = if path.exists() {
            let content = tokio::fs::read_to_string(&path).await.with_context(|| {
                format!("failed to read autoleave settings at {}", path.display())
            })?;
            serde_json::from_str::<AutoLeaveStore>(&content)
                .with_context(|| format!("invalid autoleave settings json at {}", path.display()))?
        } else {
            AutoLeaveStore::default()
        };

        let mut modes = HashMap::new();
        for (guild_id, mode) in store.guild_modes {
            modes.insert(GuildId::new(guild_id), mode);
        }

        info!(
            settings_path = %path.display(),
            guild_count = modes.len(),
            "autoleave settings loaded"
        );

        Ok(Self {
            path,
            modes: RwLock::new(modes),
        })
    }

    async fn get(&self, guild_id: GuildId) -> AutoLeaveMode {
        self.modes
            .read()
            .await
            .get(&guild_id)
            .copied()
            .unwrap_or(AutoLeaveMode::Disabled)
    }

    async fn set(&self, guild_id: GuildId, mode: AutoLeaveMode) -> Result<()> {
        {
            let mut modes = self.modes.write().await;
            modes.insert(guild_id, mode);
        }
        self.persist().await
    }

    async fn persist(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await.with_context(|| {
                    format!("failed to create settings directory {}", parent.display())
                })?;
            }
        }

        let modes = self.modes.read().await;
        let store = AutoLeaveStore {
            guild_modes: modes.iter().map(|(k, v)| (k.get(), *v)).collect(),
        };
        let json = serde_json::to_string_pretty(&store)
            .context("failed to serialize autoleave settings")?;

        tokio::fs::write(&self.path, json).await.with_context(|| {
            format!(
                "failed to write autoleave settings to {}",
                self.path.display()
            )
        })?;

        Ok(())
    }
}

#[derive(Clone)]
struct BotState {
    config: AppConfig,
    ingress: Arc<AudioIngress>,
    guilds: Arc<DashMap<GuildId, GuildState>>,
    autoleave: Arc<AutoLeaveSettings>,
    messages: Arc<MessageCatalog>,
}

struct Handler {
    state: BotState,
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: DiscordContext, ready: Ready) {
        if let Err(error) = Command::set_global_commands(&ctx.http, slash_commands()).await {
            warn!(%error, "failed to register slash commands");
        }

        info!(bot = %ready.user.name, "Discord bot connected");
    }

    async fn interaction_create(&self, ctx: DiscordContext, interaction: Interaction) {
        let Interaction::Command(command) = interaction else {
            return;
        };

        let result = match command.data.name.as_str() {
            "join" => handle_join(&ctx, &command, &self.state).await,
            "leave" => handle_leave(&ctx, &command, &self.state).await,
            "start" => handle_start(&ctx, &command, &self.state).await,
            "stop" => handle_stop(&ctx, &command, &self.state).await,
            "autoleave" => handle_autoleave(&command, &self.state).await,
            "status" => handle_status(&ctx, &command, &self.state).await,
            _ => Ok(self.state.messages.t("unknown_command")),
        };

        let response = match result {
            Ok(content) => (content, false),
            Err(error) => {
                warn!(%error, "slash command failed");
                (
                    self.state
                        .messages
                        .tr("command_error", &[("error", &error.to_string())]),
                    true,
                )
            }
        };

        if let Err(error) = respond_to_command(&ctx, &command, &response.0, response.1).await {
            warn!(%error, "failed to send slash command response");
        }
    }

    async fn voice_state_update(
        &self,
        ctx: DiscordContext,
        _old: Option<VoiceState>,
        new: VoiceState,
    ) {
        let Some(guild_id) = new.guild_id else {
            return;
        };

        if self.state.autoleave.get(guild_id).await != AutoLeaveMode::WhenNoHumans {
            return;
        }

        let my_user_id = ctx.cache.current_user().id;
        if let Err(error) =
            maybe_autoleave_when_empty(&ctx, &self.state, guild_id, my_user_id).await
        {
            warn!(%error, guild = %guild_id, "autoleave check failed");
        }
    }
}

fn slash_commands() -> Vec<CreateCommand> {
    vec![
        CreateCommand::new("join").description("Join your current voice channel"),
        CreateCommand::new("leave").description("Leave voice and stop streaming"),
        CreateCommand::new("start")
            .description("Enable sending already-listening input to Discord"),
        CreateCommand::new("stop").description("Stop the active audio stream"),
        CreateCommand::new("autoleave")
            .description("Configure auto-leave behavior")
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::SubCommand,
                    "set",
                    "Set auto-leave mode for this server",
                )
                .add_sub_option(
                    CreateCommandOption::new(CommandOptionType::String, "mode", "Auto-leave mode")
                        .required(true)
                        .add_string_choice("Do not auto-leave", "disabled")
                        .add_string_choice("Leave when no humans remain", "when_no_humans"),
                ),
            )
            .add_option(CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "status",
                "Show current auto-leave mode for this server",
            )),
        CreateCommand::new("status").description("Show stream status"),
    ]
}

async fn respond_to_command(
    ctx: &DiscordContext,
    command: &CommandInteraction,
    content: &str,
    ephemeral: bool,
) -> Result<()> {
    command
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content(content)
                    .ephemeral(ephemeral),
            ),
        )
        .await
        .context("failed to create interaction response")?;

    Ok(())
}

async fn handle_join(
    ctx: &DiscordContext,
    command: &CommandInteraction,
    state: &BotState,
) -> Result<String> {
    let (guild_id, channel_id) = resolve_user_voice_channel(ctx, command)?;
    let bot_user_id = ctx.cache.current_user().id;

    let manager = songbird::get(ctx)
        .await
        .context("songbird voice client is not registered")?
        .clone();

    let already_in_same_channel =
        bot_voice_channel(ctx, guild_id, bot_user_id).is_some_and(|current| current == channel_id);

    if already_in_same_channel {
        return Ok(state.messages.t("join_already_in_channel"));
    }

    match manager.join(guild_id, channel_id).await {
        Ok(_) => Ok(state.messages.t("join_success")),
        Err(error) => Err(error.into()),
    }
}

fn resolve_user_voice_channel(
    ctx: &DiscordContext,
    command: &CommandInteraction,
) -> Result<(GuildId, ChannelId)> {
    let guild_id = command
        .guild_id
        .context("this command only works in a server")?;
    let user_id = command.user.id;

    let channel_id = {
        let guild = guild_id
            .to_guild_cached(&ctx.cache)
            .context("guild data is not in cache yet")?;
        guild
            .voice_states
            .get(&user_id)
            .and_then(|state| state.channel_id)
            .context("you must be in a voice channel first")?
    };

    Ok((guild_id, channel_id))
}

async fn handle_leave(
    ctx: &DiscordContext,
    command: &CommandInteraction,
    state: &BotState,
) -> Result<String> {
    let guild_id = command
        .guild_id
        .context("this command only works in a server")?;

    if let Some(mut entry) = state.guilds.get_mut(&guild_id) {
        if let Some(track) = entry.track.take() {
            let _ = track.stop();
        }
    }

    let manager = songbird::get(ctx)
        .await
        .context("songbird voice client is not registered")?
        .clone();

    if manager.get(guild_id).is_some() {
        manager.remove(guild_id).await?;
    }

    Ok(state.messages.t("leave_success"))
}

async fn handle_start(
    ctx: &DiscordContext,
    command: &CommandInteraction,
    state: &BotState,
) -> Result<String> {
    let (guild_id, channel_id) = resolve_user_voice_channel(ctx, command)?;
    let bot_user_id = ctx.cache.current_user().id;

    if let Some(existing) = state.guilds.get(&guild_id) {
        if existing.track.is_some() {
            return Ok(state.messages.t("start_already_active"));
        }
    }

    let manager = songbird::get(ctx)
        .await
        .context("songbird voice client is not registered")?
        .clone();

    let already_in_same_channel =
        bot_voice_channel(ctx, guild_id, bot_user_id).is_some_and(|current| current == channel_id);

    if !already_in_same_channel {
        manager
            .join(guild_id, channel_id)
            .await
            .context("failed to join voice channel")?;
    }

    let call_lock = manager
        .get(guild_id)
        .context("failed to get voice call after joining")?;

    let input: Input = RawAdapter::new(
        state.ingress.bridge.source(),
        state.config.audio_sample_rate,
        state.config.audio_channels,
    )
    .into();

    let track = {
        let mut call = call_lock.lock().await;
        call.play_input(input)
    };

    state
        .guilds
        .insert(guild_id, GuildState { track: Some(track) });

    Ok(state.messages.t("start_enabled"))
}

async fn handle_stop(
    _ctx: &DiscordContext,
    command: &CommandInteraction,
    state: &BotState,
) -> Result<String> {
    let guild_id = command
        .guild_id
        .context("this command only works in a server")?;

    if let Some(mut entry) = state.guilds.get_mut(&guild_id) {
        if let Some(track) = entry.track.take() {
            let _ = track.stop();
            Ok(state.messages.t("stop_success"))
        } else {
            Ok(state.messages.t("stop_already_stopped"))
        }
    } else {
        Ok(state.messages.t("stop_already_stopped"))
    }
}

async fn handle_autoleave(command: &CommandInteraction, state: &BotState) -> Result<String> {
    let guild_id = command
        .guild_id
        .context("this command only works in a server")?;

    let subcommand = command
        .data
        .options
        .first()
        .context("missing subcommand; use set or status")?;

    match subcommand.name.as_str() {
        "set" => {
            let mode = match &subcommand.value {
                serenity::model::application::CommandDataOptionValue::SubCommand(options) => {
                    options
                        .iter()
                        .find(|option| option.name == "mode")
                        .and_then(|option| option.value.as_str())
                        .and_then(AutoLeaveMode::from_option)
                        .context("invalid mode")?
                }
                _ => anyhow::bail!("invalid subcommand payload"),
            };

            state.autoleave.set(guild_id, mode).await?;
            info!(guild = %guild_id, mode = %mode.as_str(), "autoleave mode updated");

            let message = match mode {
                AutoLeaveMode::Disabled => state.messages.t("autoleave_set_disabled"),
                AutoLeaveMode::WhenNoHumans => state.messages.t("autoleave_set_when_no_humans"),
            };

            Ok(message)
        }
        "status" => {
            let mode = state.autoleave.get(guild_id).await;
            let mode_text = localized_autoleave_mode(mode, state);
            Ok(state
                .messages
                .tr("autoleave_status", &[("mode", &mode_text)]))
        }
        _ => anyhow::bail!("unknown autoleave subcommand"),
    }
}

async fn handle_status(
    _ctx: &DiscordContext,
    command: &CommandInteraction,
    state: &BotState,
) -> Result<String> {
    let guild_id = command
        .guild_id
        .context("this command only works in a server")?;

    let streaming_active = if let Some(entry) = state.guilds.get(&guild_id) {
        if let Some(track) = &entry.track {
            let info = track
                .get_info()
                .await
                .context("failed to read track status")?;
            !matches!(info.playing, songbird::tracks::PlayMode::Stop)
        } else {
            false
        }
    } else {
        false
    };

    let autoleave_mode = state.autoleave.get(guild_id).await;
    let streaming_text = if streaming_active {
        state.messages.t("status_streaming_active")
    } else {
        state.messages.t("status_streaming_inactive")
    };
    let autoleave_text = localized_autoleave_mode(autoleave_mode, state);

    let content = state.messages.tr(
        "status_summary",
        &[("streaming", &streaming_text), ("autoleave", &autoleave_text)],
    );

    Ok(content)
}

fn localized_autoleave_mode(mode: AutoLeaveMode, state: &BotState) -> String {
    match mode {
        AutoLeaveMode::Disabled => state.messages.t("autoleave_mode_disabled"),
        AutoLeaveMode::WhenNoHumans => state.messages.t("autoleave_mode_when_no_humans"),
    }
}

fn count_humans_in_bot_channel(
    ctx: &DiscordContext,
    guild_id: GuildId,
    bot_user_id: UserId,
) -> Option<usize> {
    let guild = guild_id.to_guild_cached(&ctx.cache)?;
    let bot_channel = bot_voice_channel(ctx, guild_id, bot_user_id)?;

    let humans = guild
        .voice_states
        .iter()
        .filter(|(user_id, vs)| **user_id != bot_user_id && vs.channel_id == Some(bot_channel))
        .filter(|(user_id, _)| !ctx.cache.user(**user_id).is_some_and(|u| u.bot))
        .count();

    Some(humans)
}

fn bot_voice_channel(
    ctx: &DiscordContext,
    guild_id: GuildId,
    bot_user_id: UserId,
) -> Option<ChannelId> {
    let guild = guild_id.to_guild_cached(&ctx.cache)?;
    guild
        .voice_states
        .get(&bot_user_id)
        .and_then(|vs| vs.channel_id)
}

async fn maybe_autoleave_when_empty(
    ctx: &DiscordContext,
    state: &BotState,
    guild_id: GuildId,
    bot_user_id: UserId,
) -> Result<()> {
    let humans = match count_humans_in_bot_channel(ctx, guild_id, bot_user_id) {
        Some(value) => value,
        None => return Ok(()),
    };

    if humans > 0 {
        return Ok(());
    }

    if let Some(mut entry) = state.guilds.get_mut(&guild_id) {
        if let Some(track) = entry.track.take() {
            let _ = track.stop();
        }
    }

    let manager = songbird::get(ctx)
        .await
        .context("songbird voice client is not registered")?
        .clone();

    if manager.get(guild_id).is_some() {
        manager.remove(guild_id).await?;
        info!(guild = %guild_id, "auto-left voice because no humans remained");
    }

    Ok(())
}

async fn run_pcm_listener(
    bind_addr: SocketAddr,
    sender: Sender<Vec<u8>>,
    stop: CancellationToken,
) -> Result<()> {
    let listener = TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("failed to bind audio listener at {bind_addr}"))?;

    info!(%bind_addr, "PCM listener started and waiting for connections");

    loop {
        tokio::select! {
            _ = stop.cancelled() => {
                return Ok(());
            }
            connection = listener.accept() => {
                let (socket, peer) = connection.context("audio listener accept failed")?;
                info!(%peer, "PCM client connected");

                if let Err(error) = pump_pcm_stream(socket, sender.clone(), stop.clone()).await {
                    warn!(%error, %peer, "PCM client stream ended with error");
                } else {
                    info!(%peer, "PCM client disconnected, waiting for next connection");
                }
            }
        }
    }
}

async fn run_ffmpeg_stream_loop(
    sender: Sender<Vec<u8>>,
    stop: CancellationToken,
    ffmpeg_bin: String,
    input_url: String,
    sample_rate: u32,
    channels: u32,
) -> Result<()> {
    info!(%input_url, ffmpeg = %ffmpeg_bin, "starting ffmpeg input loop");

    loop {
        if stop.is_cancelled() {
            return Ok(());
        }

        let run_result = run_ffmpeg_once(
            sender.clone(),
            stop.clone(),
            ffmpeg_bin.clone(),
            input_url.clone(),
            sample_rate,
            channels,
        )
        .await;

        if stop.is_cancelled() {
            return Ok(());
        }

        if let Err(error) = run_result {
            warn!(
                %error,
                input_url = %input_url,
                sample_rate,
                channels,
                "ffmpeg process ended; retrying in 1 second"
            );
        }

        tokio::select! {
            _ = stop.cancelled() => return Ok(()),
            _ = sleep(Duration::from_secs(1)) => {}
        }
    }
}

async fn run_ffmpeg_once(
    sender: Sender<Vec<u8>>,
    stop: CancellationToken,
    ffmpeg_bin: String,
    input_url: String,
    sample_rate: u32,
    channels: u32,
) -> Result<()> {
    if input_url.trim().is_empty() {
        bail!("FFMPEG_INPUT_URL is empty while AUDIO_INPUT_MODE=ffmpeg_url");
    }

    let mut child = TokioCommand::new(&ffmpeg_bin)
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("warning")
        .arg("-fflags")
        .arg("+genpts+discardcorrupt")
        .arg("-flags")
        .arg("low_delay")
        .arg("-max_delay")
        .arg("500000")
        .arg("-thread_queue_size")
        .arg("4096")
        .arg("-i")
        .arg(&input_url)
        .arg("-vn")
        .arg("-sn")
        .arg("-dn")
        .arg("-af")
        .arg("aresample=async=1:min_hard_comp=0.100:first_pts=0")
        .arg("-f")
        .arg("f32le")
        .arg("-ar")
        .arg(sample_rate.to_string())
        .arg("-ac")
        .arg(channels.to_string())
        .arg("-acodec")
        .arg("pcm_f32le")
        .arg("pipe:1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to start ffmpeg from {ffmpeg_bin}"))?;

    info!(%input_url, "ffmpeg process started");

    let mut stdout = child
        .stdout
        .take()
        .context("failed to capture ffmpeg stdout")?;
    let mut stderr = child
        .stderr
        .take()
        .context("failed to capture ffmpeg stderr")?;
    let stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf).await;
        String::from_utf8_lossy(&buf).into_owned()
    });
    let mut buffer = vec![0u8; 16 * 1024];

    loop {
        tokio::select! {
            _ = stop.cancelled() => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                let _ = stderr_task.await;
                return Ok(());
            }
            read = stdout.read(&mut buffer) => {
                let read = read.context("failed to read ffmpeg stdout")?;

                if read == 0 {
                    let status = child.wait().await.context("failed waiting for ffmpeg exit")?;
                    let stderr_text = stderr_task.await.unwrap_or_else(|_| String::new());
                    let stderr_excerpt = summarize_stderr(&stderr_text);
                    if stderr_excerpt.is_empty() {
                        bail!("ffmpeg exited with status: {status}");
                    }
                    bail!("ffmpeg exited with status: {status}; stderr: {stderr_excerpt}");
                }

                // If there is no active /start output yet, keep ingest alive and drop this chunk.
                let _ = sender.send(buffer[..read].to_vec());
            }
        }
    }
}

fn summarize_stderr(stderr_text: &str) -> String {
    let cleaned = stderr_text.trim();
    if cleaned.is_empty() {
        return String::new();
    }

    let max_chars = 600;
    let mut excerpt = cleaned.chars().rev().take(max_chars).collect::<Vec<_>>();
    excerpt.reverse();
    let excerpt = excerpt.into_iter().collect::<String>();

    if cleaned.chars().count() > max_chars {
        format!("...{excerpt}")
    } else {
        excerpt
    }
}

async fn spawn_ingress_listener(
    config: AppConfig,
    sender: Sender<Vec<u8>>,
    stop: CancellationToken,
) {
    match config.audio_input_mode {
        AudioInputMode::RawTcp => {
            if let Err(error) = run_pcm_listener(config.audio_bind_addr, sender, stop).await {
                warn!(%error, "audio listener stopped");
            }
        }
        AudioInputMode::FfmpegUrl => {
            if let Err(error) = run_ffmpeg_stream_loop(
                sender,
                stop,
                config.ffmpeg_bin,
                config.ffmpeg_input_url.unwrap_or_default(),
                config.audio_sample_rate,
                config.audio_channels,
            )
            .await
            {
                warn!(%error, "ffmpeg stream worker stopped");
            }
        }
    }
}

async fn pump_pcm_stream(
    mut socket: TcpStream,
    sender: Sender<Vec<u8>>,
    stop: CancellationToken,
) -> Result<()> {
    let mut buffer = vec![0u8; 16 * 1024];

    loop {
        tokio::select! {
            _ = stop.cancelled() => {
                return Ok(());
            }
            read = socket.read(&mut buffer) => {
                let read = read.context("failed to read PCM stream")?;
                if read == 0 {
                    return Ok(());
                }

                // In always-listen mode there may be no active guild outputs yet.
                let _ = sender.send(buffer[..read].to_vec());
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let config = AppConfig::from_env()?;
    let intents = GatewayIntents::GUILDS | GatewayIntents::GUILD_VOICE_STATES;

    let bridge = Arc::new(PcmBridge::new());
    let sender = bridge.sender();
    let ingress_stop = CancellationToken::new();

    tokio::spawn(spawn_ingress_listener(
        config.clone(),
        sender.clone(),
        ingress_stop.clone(),
    ));

    let autoleave = Arc::new(
        AutoLeaveSettings::load(Path::new(GUILDS_SETTINGS_PATH))
            .await
            .context("failed to initialize autoleave settings")?,
    );
    let messages = Arc::new(MessageCatalog::load(
        &config.message_templates_path,
        &config.bot_locale,
    ));

    info!(
        settings_path = GUILDS_SETTINGS_PATH,
        "guild settings storage initialized"
    );
    info!(
        locale = messages.locale(),
        templates_path = %config.message_templates_path,
        "message templates loaded"
    );

    let state = BotState {
        config: config.clone(),
        ingress: Arc::new(AudioIngress { bridge }),
        guilds: Arc::new(DashMap::new()),
        autoleave,
        messages,
    };

    let mut client = Client::builder(&config.discord_token, intents)
        .event_handler(Handler { state })
        .register_songbird()
        .await
        .context("failed to create Discord client")?;

    if let Err(error) = client.start().await {
        error!(%error, "Discord client stopped unexpectedly");
    }

    Ok(())
}
