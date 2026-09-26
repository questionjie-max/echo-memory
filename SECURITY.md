# Security Policy

## Supported version

The current Alpha branch is supported for security reports.

## Reporting a vulnerability

Please use GitHub's private security advisory flow for this repository. Do not report a vulnerability in a public issue.

Reports must not include real audio, transcript content, API keys, local database files, payment data, or personally identifiable information. Provide a minimal reproduction using synthetic data whenever possible.

## Security boundaries

Echo Memory is local-first, not cloud-only. Audio remains local by default. Third-party ASR is disabled unless the user selects a provider, completes the endpoint/model/API-key configuration, and gives separate audio-upload consent; missing configuration or consent must fail before an upload connection is created. Third-party text analysis has a separate text-send consent and sends transcript text and structured analysis, not the audio file. External processing is enforced in the service layer, not only in the UI. API keys are stored in the macOS Keychain; SQLite stores only non-sensitive settings and whether a key is configured.

External AI output is stored as versioned snapshots with source references and review feedback. It does not overwrite source records, transcripts, or local single-record analysis. Third-party ASR is implemented as an opt-in OpenAI-compatible integration; not every provider has been verified end to end with real credentials, and speaker labels are consumed only when the provider returns them. MCP is disabled by default, uses local stdio, is read-only, and does not open a public listener. Any behavior that uploads audio without the separate ASR consent, exposes a public listener, logs API keys, or bypasses either external-AI consent should be treated as security-sensitive.
