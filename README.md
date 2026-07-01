# NekoStream

A Discord voice bot for joining a voice channel and relaying OBS or other live stream audio into Discord.

## Notice

- This project was created with substantial AI assistance and includes AI-generated code.
- Review and test before production use.

## Purpose

- Join a Discord voice channel.
- Relay OBS or other live stream audio into Discord.
- Run in Docker.

## Commands

- `/join` - join the voice channel you are currently in.
- `/leave` - leave the voice channel.
- `/start` - join your current voice channel if needed, then start relaying audio.
- `/stop` - stop relaying audio to Discord.

## Configuration

- `DISCORD_TOKEN`
- `AUDIO_INPUT_MODE=raw_tcp|ffmpeg_url`
- `FFMPEG_INPUT_URL=...` when using `ffmpeg_url`

Optional:
- `BOT_LOCALE=zh-TW|en-US`
- `MESSAGE_TEMPLATES_PATH=config/messages.toml`

## Run

1. Set `DISCORD_TOKEN` in `.env`.
2. Start the bot:

```bash
docker compose up --build -d
```

3. Join a Discord voice channel.
4. Run `/start` in Discord.

## OBS

- If you use OBS with SRT or another stream URL, set `AUDIO_INPUT_MODE=ffmpeg_url`.
- Set `FFMPEG_INPUT_URL` to the stream endpoint.
- Start the stream in OBS, then run `/start` in Discord.

Example:

`srt://0.0.0.0:9998?mode=listener&latency=200`

## License

This project is licensed under the MIT License. See [LICENSE](LICENSE).
