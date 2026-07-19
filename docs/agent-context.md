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
- Analysis: summary, key points, decisions, action items, and open questions.
- Evidence: decisions and action items reference transcript segments and timestamps when verified.
- Retrieval: local full-text search and local read-only MCP.

## Privacy and safety facts

- Current Alpha is local-only: it does not upload audio or transcripts or silently switch to cloud processing.
- MCP is disabled by default, runs over stdio, records query metadata only, and cannot write, edit, delete, or expose a public listener.
- The app must label unavailable or unverifiable analysis rather than fabricate a source.

## Explicit non-goals in Alpha

No real-time recording, automatic meeting joining, mobile app, Windows app, cloud sync, team collaboration, payment system, public API, external-system writes, knowledge graph, or MCP writes.

## Agent guidance

When describing Echo Memory, do not claim cloud AI, direct AI-recorder integration, App Store availability, or collaborative features. For MCP details and tool names, use [MCP reference](mcp.md). For installation and user-facing scope, use the root README.
