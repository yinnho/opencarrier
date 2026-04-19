# Session Storage Naming Discussion

> Working document for migrating OpenCarrier's serve-mode session storage from SQLite to JSONL.

## What is A2A in OpenCarrier?

**A2A = Agent-to-Agent Protocol** (Google's cross-framework agent interoperability protocol).

In OpenCarrier:

- `crates/opencarrier-runtime/src/a2a.rs` defines A2A standard types: `AgentCard`, `AgentCapabilities`, `AgentSkill`, etc.
- `docs/A2A-PROTOCOL-PLAN.md` positions A2A as the communication protocol between **agentd (reverse proxy)** and **opencarrier (upstream backend)**.
- `serve.rs` is explicitly labeled `A2A Serve Mode — stdin/stdout JSON-RPC server for agentd`.

## What does `serve.rs` actually handle?

`serve` mode processes **three** input formats, not just A2A:

| Format | Examples |
|--------|----------|
| **A2A / JSON-RPC 2.0** | `hello`, `sendMessage`, `getAgentCard`, `listAgents`, `compactMemory`, `bye` |
| **ACP** | `initialize`, `session/new`, `session/prompt`, `session/list` |
| **yingheclient format** | `ChatRequest` with `conversationType`, `chatType`, `conversationId` |

## Implication for naming

`YingheSessionManager` stores persistent sessions for **all external clients** hitting `serve` mode (regardless of whether they speak A2A, ACP, or the legacy yingheclient format).

Renaming it to `A2aSessionManager` / `a2a_session.rs` would **narrow** its scope incorrectly, because aginx speaks ACP and also uses this storage path.

## Better naming candidates

| Candidate | File | Pros | Cons |
|-----------|------|------|------|
| `ServeSessionManager` | `serve_session.rs` | Accurate — this only exists for `serve` mode | Tied to CLI mode name |
| `StdioSessionManager` | `stdio_session.rs` | Emphasizes stdin/stdout transport | Could imply other stdio modes too |
| `ExternalSessionManager` | `external_session.rs` | Contrasts with internal SQLite `SessionStore` | Slightly vague |

## Current proposal

Use **`ServeSessionManager`** / **`serve_session.rs`** because:

1. It lives inside `opencarrier-memory` but is **only instantiated in `serve.rs`**.
2. It clearly distinguishes from the internal `SessionStore` (SQLite-based, used by the kernel for `CanonicalSession` and agent memory).
3. It doesn't falsely claim to be "A2A-only" when ACP clients also use it.

## Storage path

```
~/.opencarrier/sessions/
  default/
    {session_id}.jsonl       # Claude-compatible JSONL, readable by aginx
  serve_session_map.json     # session_key -> session_id mapping
```

## Aginx config

```toml
id = "opencarrier"
name = "OpenCarrier"
agent_type = "process"
protocol = "acp"
command = "opencarrier"
args = ["serve"]
storage_path = "~/.opencarrier/sessions"
```

## Notes

- SQLite `MemorySubstrate` / `SessionStore` remains unchanged for internal agent memory (`CanonicalSession`, KV store, etc.).
- The `yinghe_sessions.db` SQLite file will be removed entirely.
- This document is a working draft and may be updated as the migration proceeds.
