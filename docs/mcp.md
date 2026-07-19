# Echo Memory MCP Reference

Echo Memory provides a **local-only, read-only stdio MCP server**. It is disabled by default and only serves the local library after the user enables AI Connection in the app.

## Development configuration

Build the server first:

```bash
cd app/src-tauri
cargo build --bin echo-memory-mcp --features mcp-bin
```

Then configure an MCP client with the built executable:

```json
{
  "mcpServers": {
    "echo-memory": {
      "command": "/absolute/path/to/app/src-tauri/target/debug/echo-memory-mcp"
    }
  }
}
```

## Read-only tools

| Tool | Purpose |
| --- | --- |
| `search_records` | Search local record titles and transcript snippets; returns at most 10 results. |
| `get_record` | Read record metadata and up to 100 latest transcript segments. |
| `get_transcript` | Read a bounded time range of transcript segments. |
| `list_projects` | List local projects. |
| `get_project_context` | Return recent records, decisions, and action items for a project. |
| `list_action_items` | List local action items, optionally filtered by project and status. |

## Evidence and boundaries

Search results and structured context include record identifiers and time ranges so a user can return to the original transcript and audio. A response is not permission to modify a record: this server has no write, edit, delete, network-listening, or external-system tools.

The server logs tool name, record/project identifiers, and time. It does not log audio, transcript bodies, or credentials.
