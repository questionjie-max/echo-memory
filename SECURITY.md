# Security Policy

## Supported version

The current Alpha branch is supported for security reports.

## Reporting a vulnerability

Please use GitHub's private security advisory flow for this repository. Do not report a vulnerability in a public issue.

Reports must not include real audio, transcript content, API keys, local database files, payment data, or personally identifiable information. Provide a minimal reproduction using synthetic data whenever possible.

## Security boundaries

Echo Memory is designed to keep audio and transcripts local. MCP is disabled by default, uses local stdio, is read-only, and does not open a public listener. Any behavior that bypasses these boundaries should be treated as security-sensitive.
