# Echo Memory v0.1.0 Alpha

## Highlights

- Local audio import for MP3, M4A, and WAV.
- Local timestamped transcription, playback seeking, and non-destructive transcript edits.
- Local structured analysis with source-backed decisions and action items.
- Project knowledge libraries and local full-text search.
- User-enabled, local stdio, read-only MCP access for AI tools.

## Requirements

This is a macOS source release. It requires Node.js, Rust, local Whisper support, and optionally local Ollama for analysis. No signed installer is provided in this release.

## Privacy

The current Alpha keeps audio and transcripts local. MCP is disabled by default, read-only, and does not open a public port.

## Known limitations

- A local Whisper model must be available for real transcription.
- A local Ollama service and model are required for analysis.
- Real-world long-recording and offline regression coverage is still being expanded.
- No cloud sync, mobile client, Windows client, team collaboration, payment system, or MCP write support is included.
