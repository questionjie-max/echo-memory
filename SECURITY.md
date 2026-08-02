# Security Policy

## Supported version

The current Alpha branch is supported for security reports.

## Reporting a vulnerability

Please use GitHub's private security advisory flow for this repository. Do not report a vulnerability in a public issue.

Reports must not include real audio, transcript content, API keys, local database files, payment data, or personally identifiable information. Provide a minimal reproduction using synthetic data whenever possible.

## Security boundaries

Echo Memory is local-first, not cloud-only. Audio is kept local and is never uploaded by the external-memory feature. External AI is disabled by default; when a user enables it and confirms a generation, the selected titles, dates, projects, full transcripts, and existing structured analysis may be sent to the user-configured OpenAI-compatible service. API keys are stored in the macOS Keychain; SQLite stores only non-sensitive settings and whether a key is configured.

External AI output is stored as versioned snapshots with source references and review feedback. It does not overwrite source records, transcripts, or local single-record analysis. Cloud transcription is not implemented. MCP is disabled by default, uses local stdio, is read-only, and does not open a public listener. Any behavior that uploads audio, exposes a public listener, logs API keys, or bypasses the user's external-AI consent should be treated as security-sensitive.
