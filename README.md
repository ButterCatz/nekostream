# NekoStream

A Rust Discord voice bot that joins a voice channel and keeps waiting for live audio input, then streams audio into Discord voice.

## Notice

- This project was created with substantial AI assistance and includes AI-generated code.
- Review and test before using it in production.

## What this first version does

- Joins and leaves Discord voice channels.
- Ingest listener starts with `docker compose up` and keeps running.
- `/start` and `/stop` control output to Discord per guild.
- `/start` auto-joins your current channel if needed.
- Supports two ingest modes:
	- `raw_tcp`: listens on TCP and expects raw `f32le` PCM.
	- `ffmpeg_url`: bot starts ffmpeg in-container and converts an input URL into PCM.
- Supports per-server autoleave policy.
- Discord response text is loaded from TOML templates at runtime.
- Supports locale switch (currently `zh-TW` and `en-US`).
- Runs in Docker.

## Commands

Use slash commands:

- `/join` - join the voice channel you are currently in.
- `/leave` - stop the stream and leave the voice channel.
- `/start` - auto-join your current voice channel, then enable sending input audio to Discord.
- `/stop` - stop sending audio to Discord for this server (listener keeps running).
- `/autoleave set mode:<disabled|when_no_humans>` - set per-server auto-leave behavior.
- `/autoleave status` - show current auto-leave mode.
- `/status` - show whether a stream is active.

## Configuration

- `AUDIO_INPUT_MODE=raw_tcp|ffmpeg_url`
- `FFMPEG_INPUT_URL=...` (required in `ffmpeg_url` mode)
- `BOT_LOCALE=zh-TW|en-US`
- `MESSAGE_TEMPLATES_PATH=config/messages.toml`

Message templates:
- Default file: `config/messages.toml`
- You can edit texts without rebuilding the binary.
- Restart container after editing template file to apply changes.

Internal defaults (usually no need to set manually):
- `AUDIO_BIND_ADDR=0.0.0.0:3030` in raw_tcp mode
- `AUDIO_SAMPLE_RATE=48000`
- `AUDIO_CHANNELS=2`
- `FFMPEG_BIN=ffmpeg` in ffmpeg_url mode

Discord token source:
- `DISCORD_TOKEN` in `.env`

## Running with Docker

1. Set `DISCORD_TOKEN` in `.env`.
2. Start the container:

```bash
docker compose up --build
```

3. Join a Discord voice channel and run `/join` in a text channel.
4. Run `/start` once.

`/start` can be used directly without `/join`; it will auto-join your current voice channel.

Autoleave examples:
- `/autoleave set mode:disabled`
- `/autoleave set mode:when_no_humans`
- `/autoleave status`

Autoleave setting is persisted per server in `data/guilds.json` (inside container: `/app/data/guilds.json`).

For `raw_tcp` mode:
5. Send raw PCM into `localhost:3030`.

For `ffmpeg_url` mode:
5. Set `AUDIO_INPUT_MODE=ffmpeg_url` and `FFMPEG_INPUT_URL` in `.env`.
6. Run `/start` once. The bot keeps ffmpeg running and auto-retries on source disconnect.

Example `FFMPEG_INPUT_URL` for OBS SRT output listener:

`srt://0.0.0.0:9998?mode=listener&latency=200`

## Stability tuning (music bot style)

This bot now applies a stability-oriented ffmpeg ingest profile commonly used in music bots:
- Large input queue (`-thread_queue_size 4096`) to reduce burst/jitter drops.
- Audio-only decode (`-vn -sn -dn`) to avoid wasting decode budget on video/subtitles.
- Async audio resample (`aresample=async=1:min_hard_comp=0.100:first_pts=0`) to smooth timestamp jitter.

Recommended OBS audio settings for stable playback:
- Audio encoder: AAC
- Rate control: CBR
- Audio bitrate: 160 kbps (128 kbps minimum, 192 kbps if bandwidth is stable)
- Sample rate: 48 kHz
- Channels: Stereo

If playback still stutters, increase SRT latency to `250` or `300`.

## OBS wiring

Recommended path with bot-side conversion:
1. In OBS, use output that can target SRT/RTMP/URL (for example SRT).
2. Set OBS target to the endpoint represented by `FFMPEG_INPUT_URL`.
3. Start stream in OBS.
4. In Discord, run `/start` once.

The bot handles conversion in-container through ffmpeg and forwards PCM to Discord voice.

## License

This project is licensed under the MIT License. See [LICENSE](LICENSE).
