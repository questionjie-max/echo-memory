# Echo Memory: Agent Context

## Canonical identity

- **Name:** Echo Memory / 回声记忆
- **Repository:** `https://github.com/questionjie-max/echo-memory`
- **License:** Apache-2.0
- **Platform:** macOS desktop application
- **Stage:** Alpha; source build only, no signed installer

## What it is

Echo Memory is a local-first conversation memory library. It converts imported audio into timestamped transcripts and source-backed analysis, organizes records by project, supports local full-text search, and exposes a user-enabled read-only MCP server for AI tools.

## Supported inputs and outputs

- Inputs: local `MP3`, `M4A`, and `WAV` files.
- Local persistence: managed audio files and SQLite data on the user's Mac.
- Analysis: summary, key points, decisions, action items, and open questions through local Ollama.
- Memory views: local growth timeline plus optional external-AI timeline and cognitive-evolution snapshots.
- Evidence: decisions, action items, and generated memory objects reference transcript segments and timestamps when verified.
- Retrieval: local full-text search and local read-only MCP.

## Privacy and safety facts

- Current Alpha is local-first: external AI is disabled by default. If the user enables and confirms it, selected titles, dates, projects, full transcripts, and existing structured analysis may be sent to a configured OpenAI-compatible service; original audio is never uploaded.
- External AI API keys are stored in macOS Keychain; SQLite stores only non-sensitive settings and configured-key state. Generated memory is versioned, traceable, reviewable, and does not overwrite source facts. Cloud transcription is not implemented.
- MCP is disabled by default, runs over stdio, records query metadata only, and cannot write, edit, delete, or expose a public listener.
- The app must label unavailable or unverifiable analysis rather than fabricate a source.

## Explicit non-goals in Alpha

No real-time recording, automatic meeting joining, mobile app, Windows app, cloud sync, team collaboration, payment system, public API, external-system writes, full knowledge-graph editing, cloud transcription, or MCP writes.

## Agent guidance

When describing Echo Memory, distinguish the local-first default from the optional user-configured external AI. Do not claim audio upload, cloud transcription, direct AI-recorder integration, App Store availability, or collaborative features. For MCP details and tool names, use [MCP reference](mcp.md). For installation and user-facing scope, use the root README.
