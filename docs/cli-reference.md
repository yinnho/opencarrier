# OpenCarrier CLI Reference

Complete command-line reference for `opencarrier`, the CLI tool for the OpenCarrier Agent OS.

## Overview

The `opencarrier` binary is the primary interface for managing the OpenCarrier Agent OS. It supports two modes of operation:

- **Daemon mode** -- When a daemon is running (`opencarrier start`), CLI commands communicate with it over HTTP. This is the recommended mode for production use.
- **In-process mode** -- When no daemon is detected, commands that support it will boot an ephemeral in-process kernel. Agents spawned in this mode are not persisted and will be lost when the process exits.

Running `opencarrier` with no subcommand launches the interactive TUI (terminal user interface) built with ratatui, which provides a full dashboard experience in the terminal.

## Installation

### From source (cargo)

```bash
cargo install --path crates/opencarrier-cli
```

### Build from workspace

```bash
cargo build --release -p opencarrier-cli
# Binary: target/release/opencarrier (or opencarrier.exe on Windows)
```

### Docker

```bash
docker run -it opencarrier/opencarrier:latest
```

### Shell installer

```bash
curl -fsSL https://get.opencarrier.ai | sh
```

## Global Options

These options apply to all commands.

| Option | Description |
|---|---|
| `--config <PATH>` | Path to a custom config file. Overrides the default `~/.opencarrier/config.toml`. |
| `--help` | Print help information for any command or subcommand. |
| `--version` | Print the version of the `opencarrier` binary. |

**Environment variables:**

| Variable | Description |
|---|---|
| `RUST_LOG` | Controls log verbosity (e.g. `info`, `debug`, `opencarrier_kernel=trace`). |
| `OPENCARRIER_AGENTS_DIR` | Override the agent templates directory. |
| `EDITOR` / `VISUAL` | Editor used by `opencarrier config edit`. Falls back to `notepad` (Windows) or `vi` (Unix). |

---

## Command Reference

### opencarrier (no subcommand)

Launch the interactive TUI dashboard.

```
opencarrier [--config <PATH>]
```

The TUI provides a full-screen terminal interface with panels for agents, chat, workflows, channels, skills, settings, and more. Tracing output is redirected to `~/.opencarrier/tui.log` to avoid corrupting the terminal display.

Press `Ctrl+C` to exit. A second `Ctrl+C` force-exits the process.

---

### opencarrier init

Initialize the OpenCarrier workspace. Creates `~/.opencarrier/` with subdirectories (`data/`, `agents/`) and a default `config.toml`.

```
opencarrier init [--quick]
```

**Options:**

| Option | Description |
|---|---|
| `--quick` | Skip interactive prompts. Auto-detects the best available LLM provider and writes config immediately. Suitable for CI/scripts. |

**Behavior:**

- Without `--quick`: Launches an interactive 5-step onboarding wizard (ratatui TUI) that walks through provider selection, API key configuration, and optionally starts the daemon.
- With `--quick`: Auto-detects providers by checking environment variables in priority order: Groq, Gemini, DeepSeek, Anthropic, OpenAI, OpenRouter. Falls back to Groq if none are found.
- File permissions are restricted to owner-only (`0600` for files, `0700` for directories) on Unix.

**Example:**

```bash
# Interactive setup
opencarrier init

# Non-interactive (CI/scripts)
export GROQ_API_KEY="gsk_..."
opencarrier init --quick
```

---

### opencarrier start

Start the OpenCarrier daemon (kernel + API server).

```
opencarrier start [--config <PATH>]
```

**Behavior:**

- Checks if a daemon is already running; exits with an error if so.
- Boots the OpenCarrier kernel (loads config, initializes SQLite database, loads agents, connects MCP servers, starts background tasks).
- Starts the HTTP API server on the address specified in `config.toml` (default: `127.0.0.1:4200`).
- Writes `daemon.json` to `~/.opencarrier/` so other CLI commands can discover the running daemon.
- Blocks until interrupted with `Ctrl+C`.

**Output:**

```
  OpenCarrier Agent OS v0.1.0

  Starting daemon...

  [ok] Kernel booted (groq/llama-3.3-70b-versatile)
  [ok] 50 models available
  [ok] 3 agent(s) loaded

  API:        http://127.0.0.1:4200
  Dashboard:  http://127.0.0.1:4200/
  Provider:   groq
  Model:      llama-3.3-70b-versatile

  hint: Open the dashboard in your browser, or run `opencarrier chat`
  hint: Press Ctrl+C to stop the daemon
```

**Example:**

```bash
# Start with default config
opencarrier start

# Start with custom config
opencarrier start --config /path/to/config.toml
```

---

### opencarrier status

Show the current kernel/daemon status.

```
opencarrier status [--json]
```

**Options:**

| Option | Description |
|---|---|
| `--json` | Output machine-readable JSON for scripting. |

**Behavior:**

- If a daemon is running: queries `GET /api/status` and displays agent count, provider, model, uptime, API URL, data directory, and lists active agents.
- If no daemon is running: boots an in-process kernel and shows persisted state. Displays a warning that the daemon is not running.

**Example:**

```bash
opencarrier status

opencarrier status --json | jq '.agent_count'
```

---

### opencarrier doctor

Run diagnostic checks on the OpenCarrier installation.

```
opencarrier doctor [--json] [--repair]
```

**Options:**

| Option | Description |
|---|---|
| `--json` | Output results as JSON for scripting. |
| `--repair` | Attempt to auto-fix issues (create missing directories, config, remove stale files). Prompts for confirmation before each repair. |

**Checks performed:**

1. **OpenCarrier directory** -- `~/.opencarrier/` exists
2. **.env file** -- exists and has correct permissions (0600 on Unix)
3. **Config TOML syntax** -- `config.toml` parses without errors
4. **Daemon status** -- whether a daemon is running
5. **Port 4200 availability** -- if daemon is not running, checks if the port is free
6. **Stale daemon.json** -- leftover `daemon.json` from a crashed daemon
7. **Database file** -- SQLite magic bytes validation
8. **Disk space** -- warns if less than 100MB available (Unix only)
9. **Agent manifests** -- validates all `.toml` files in `~/.opencarrier/agents/`
10. **LLM provider keys** -- checks env vars for 10 providers (Groq, OpenRouter, Anthropic, OpenAI, DeepSeek, Gemini, Google, Together, Mistral, Fireworks), performs live validation (401/403 detection)
11. **Channel tokens** -- format validation for Telegram, Discord, Slack tokens
12. **Config consistency** -- checks that `api_key_env` references in config match actual environment variables
13. **Rust toolchain** -- `rustc --version`

**Example:**

```bash
opencarrier doctor

opencarrier doctor --repair

opencarrier doctor --json
```

---

### opencarrier dashboard

Open the web dashboard in the default browser.

```
opencarrier dashboard
```

**Behavior:**

- Requires a running daemon.
- Opens the daemon URL (e.g. `http://127.0.0.1:4200/`) in the system browser.
- Copies the URL to the system clipboard (uses PowerShell on Windows, `pbcopy` on macOS, `xclip`/`xsel` on Linux).

**Example:**

```bash
opencarrier dashboard
```

---

### opencarrier completion

Generate shell completion scripts.

```
opencarrier completion <SHELL>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<SHELL>` | Target shell. One of: `bash`, `zsh`, `fish`, `elvish`, `powershell`. |

**Example:**

```bash
# Bash
opencarrier completion bash > ~/.bash_completion.d/opencarrier

# Zsh
opencarrier completion zsh > ~/.zfunc/_opencarrier

# Fish
opencarrier completion fish > ~/.config/fish/completions/opencarrier.fish

# PowerShell
opencarrier completion powershell > opencarrier.ps1
```

---

## Agent Commands

### opencarrier agent new

Spawn an agent from a built-in template.

```
opencarrier agent new [<TEMPLATE>]
```

**Arguments:**

| Argument | Description |
|---|---|
| `<TEMPLATE>` | Template name (e.g. `coder`, `assistant`, `researcher`). If omitted, displays an interactive picker listing all available templates. |

**Behavior:**

- Templates are discovered from: the repo `agents/` directory (dev builds), `~/.opencarrier/agents/` (installed), and `OPENCARRIER_AGENTS_DIR` (env override).
- Each template is a directory containing an `agent.toml` manifest.
- In daemon mode: sends `POST /api/agents` with the manifest. Agent is persistent.
- In standalone mode: boots an in-process kernel. Agent is ephemeral.

**Example:**

```bash
# Interactive picker
opencarrier agent new

# Spawn by name
opencarrier agent new coder

# Spawn the assistant template
opencarrier agent new assistant
```

---

### opencarrier agent spawn

Spawn an agent from a custom manifest file.

```
opencarrier agent spawn <MANIFEST>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<MANIFEST>` | Path to an agent manifest TOML file. |

**Behavior:**

- Reads and parses the TOML manifest file.
- In daemon mode: sends the raw TOML to `POST /api/agents`.
- In standalone mode: boots an in-process kernel and spawns the agent locally.

**Example:**

```bash
opencarrier agent spawn ./my-agent/agent.toml
```

---

### opencarrier agent list

List all running agents.

```
opencarrier agent list [--json]
```

**Options:**

| Option | Description |
|---|---|
| `--json` | Output as JSON array for scripting. |

**Output columns:** ID, NAME, STATE, PROVIDER, MODEL (daemon mode) or ID, NAME, STATE, CREATED (in-process mode).

**Example:**

```bash
opencarrier agent list

opencarrier agent list --json | jq '.[].name'
```

---

### opencarrier agent chat

Start an interactive chat session with a specific agent.

```
opencarrier agent chat <AGENT_ID>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<AGENT_ID>` | Agent UUID. Obtain from `opencarrier agent list`. |

**Behavior:**

- Opens a REPL-style chat loop.
- Type messages at the `you>` prompt.
- Agent responses display at the `agent>` prompt, followed by token usage and iteration count.
- Type `exit`, `quit`, or press `Ctrl+C` to end the session.

**Example:**

```bash
opencarrier agent chat a1b2c3d4-e5f6-7890-abcd-ef1234567890
```

---

### opencarrier agent kill

Terminate a running agent.

```
opencarrier agent kill <AGENT_ID>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<AGENT_ID>` | Agent UUID to terminate. |

**Example:**

```bash
opencarrier agent kill a1b2c3d4-e5f6-7890-abcd-ef1234567890
```

---

## Workflow Commands

All workflow commands require a running daemon.

### opencarrier workflow list

List all registered workflows.

```
opencarrier workflow list
```

**Output columns:** ID, NAME, STEPS, CREATED.

---

### opencarrier workflow create

Create a workflow from a JSON definition file.

```
opencarrier workflow create <FILE>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<FILE>` | Path to a JSON file describing the workflow steps. |

**Example:**

```bash
opencarrier workflow create ./my-workflow.json
```

---

### opencarrier workflow run

Execute a workflow by ID.

```
opencarrier workflow run <WORKFLOW_ID> <INPUT>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<WORKFLOW_ID>` | Workflow UUID. Obtain from `opencarrier workflow list`. |
| `<INPUT>` | Input text to pass to the workflow. |

**Example:**

```bash
opencarrier workflow run abc123 "Analyze this code for security issues"
```

---

## Trigger Commands

All trigger commands require a running daemon.

### opencarrier trigger list

List all event triggers.

```
opencarrier trigger list [--agent-id <ID>]
```

**Options:**

| Option | Description |
|---|---|
| `--agent-id <ID>` | Filter triggers by the owning agent's UUID. |

**Output columns:** TRIGGER ID, AGENT ID, ENABLED, FIRES, PATTERN.

---

### opencarrier trigger create

Create an event trigger for an agent.

```
opencarrier trigger create <AGENT_ID> <PATTERN_JSON> [--prompt <TEMPLATE>] [--max-fires <N>]
```

**Arguments:**

| Argument | Description |
|---|---|
| `<AGENT_ID>` | UUID of the agent that owns the trigger. |
| `<PATTERN_JSON>` | Trigger pattern as a JSON string. |

**Options:**

| Option | Default | Description |
|---|---|---|
| `--prompt <TEMPLATE>` | `"Event: {{event}}"` | Prompt template. Use `{{event}}` as a placeholder for the event data. |
| `--max-fires <N>` | `0` (unlimited) | Maximum number of times the trigger will fire. |

**Pattern examples:**

```bash
# Fire on any lifecycle event
opencarrier trigger create <AGENT_ID> '{"lifecycle":{}}'

# Fire when a specific agent is spawned
opencarrier trigger create <AGENT_ID> '{"agent_spawned":{"name_pattern":"*"}}'

# Fire on agent termination
opencarrier trigger create <AGENT_ID> '{"agent_terminated":{}}'

# Fire on all events (limited to 10 fires)
opencarrier trigger create <AGENT_ID> '{"all":{}}' --max-fires 10
```

---

### opencarrier trigger delete

Delete a trigger by ID.

```
opencarrier trigger delete <TRIGGER_ID>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<TRIGGER_ID>` | UUID of the trigger to delete. |

---

## Skill Commands

### opencarrier skill list

List all installed skills.

```
opencarrier skill list
```

**Output columns:** NAME, VERSION, TOOLS, DESCRIPTION.

Loads skills from `~/.opencarrier/skills/` plus bundled skills compiled into the binary.

---

### opencarrier skill install

Install a skill from a local directory, git URL, or FangHub marketplace.

```
opencarrier skill install <SOURCE>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<SOURCE>` | Skill name (FangHub), local directory path, or git URL. |

**Behavior:**

- **Local directory:** Looks for `skill.toml` in the directory. If not found, checks for OpenClaw-format skills (SKILL.md with YAML frontmatter) and auto-converts them.
- **Remote (FangHub):** Fetches and installs from the FangHub marketplace. Skills pass through SHA256 verification and prompt injection scanning.

**Example:**

```bash
# Install from local directory
opencarrier skill install ./my-skill/

# Install from FangHub
opencarrier skill install web-search

# Install an OpenClaw-format skill
opencarrier skill install ./openclaw-skill/
```

---

### opencarrier skill remove

Remove an installed skill.

```
opencarrier skill remove <NAME>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<NAME>` | Name of the skill to remove. |

**Example:**

```bash
opencarrier skill remove web-search
```

---

### opencarrier skill search

Search the FangHub marketplace for skills.

```
opencarrier skill search <QUERY>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<QUERY>` | Search query string. |

**Example:**

```bash
opencarrier skill search "docker kubernetes"
```

---

### opencarrier skill create

Interactively scaffold a new skill project.

```
opencarrier skill create
```

**Behavior:**

Prompts for:
- Skill name
- Description
- Runtime (`python`, `node`, or `wasm`; defaults to `python`)

Creates a directory under `~/.opencarrier/skills/<name>/` with:
- `skill.toml` -- manifest file
- `src/main.py` (or `src/index.js`) -- entry point with boilerplate

**Example:**

```bash
opencarrier skill create
# Skill name: my-tool
# Description: A custom analysis tool
# Runtime (python/node/wasm) [python]: python
```

---

## Channel Commands

### opencarrier channel list

List configured channels and their status.

```
opencarrier channel list
```

**Output columns:** CHANNEL, ENV VAR, STATUS.

Checks `config.toml` for channel configuration sections and environment variables for required tokens. Status is one of: `Ready`, `Missing env`, `Not configured`.

**Channels checked:** webchat, telegram, discord, slack, whatsapp, signal, matrix, email.

---

### opencarrier channel setup

Interactive setup wizard for a channel integration.

```
opencarrier channel setup [<CHANNEL>]
```

**Arguments:**

| Argument | Description |
|---|---|
| `<CHANNEL>` | Channel name. If omitted, displays an interactive picker. |

**Supported channels:** `telegram`, `discord`, `slack`, `whatsapp`, `email`, `signal`, `matrix`.

Each wizard:
1. Displays step-by-step instructions for obtaining credentials.
2. Prompts for tokens/credentials.
3. Saves tokens to `~/.opencarrier/.env` with owner-only permissions.
4. Appends the channel configuration block to `config.toml` (prompts for confirmation).
5. Warns to restart the daemon if one is running.

**Example:**

```bash
# Interactive picker
opencarrier channel setup

# Direct setup
opencarrier channel setup telegram
opencarrier channel setup discord
opencarrier channel setup slack
```

---

### opencarrier channel test

Send a test message through a configured channel.

```
opencarrier channel test <CHANNEL>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<CHANNEL>` | Channel name to test. |

Requires a running daemon. Sends `POST /api/channels/<channel>/test`.

**Example:**

```bash
opencarrier channel test telegram
```

---

### opencarrier channel enable

Enable a channel integration.

```
opencarrier channel enable <CHANNEL>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<CHANNEL>` | Channel name to enable. |

In daemon mode: sends `POST /api/channels/<channel>/enable`. Without a daemon: prints a note that the change will take effect on next start.

---

### opencarrier channel disable

Disable a channel without removing its configuration.

```
opencarrier channel disable <CHANNEL>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<CHANNEL>` | Channel name to disable. |

In daemon mode: sends `POST /api/channels/<channel>/disable`. Without a daemon: prints a note to edit `config.toml`.

---

## Config Commands

### opencarrier config show

Display the current configuration file.

```
opencarrier config show
```

Prints the contents of `~/.opencarrier/config.toml` with the file path as a header comment.

---

### opencarrier config edit

Open the configuration file in your editor.

```
opencarrier config edit
```

Uses `$EDITOR`, then `$VISUAL`, then falls back to `notepad` (Windows) or `vi` (Unix).

---

### opencarrier config get

Get a single configuration value by dotted key path.

```
opencarrier config get <KEY>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<KEY>` | Dotted key path into the TOML structure. |

**Example:**

```bash
opencarrier config get default_model.provider
# groq

opencarrier config get api_listen
# 127.0.0.1:4200

opencarrier config get memory.decay_rate
# 0.05
```

---

### opencarrier config set

Set a configuration value by dotted key path.

```
opencarrier config set <KEY> <VALUE>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<KEY>` | Dotted key path. |
| `<VALUE>` | New value. Type is inferred from the existing value (integer, float, boolean, or string). |

**Warning:** This command re-serializes the TOML file, which strips all comments.

**Example:**

```bash
opencarrier config set default_model.provider anthropic
opencarrier config set default_model.model claude-sonnet-4-20250514
opencarrier config set api_listen "0.0.0.0:4200"
```

---

### opencarrier config set-key

Save an LLM provider API key to `~/.opencarrier/.env`.

```
opencarrier config set-key <PROVIDER>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<PROVIDER>` | Provider name (e.g. `groq`, `anthropic`, `openai`, `gemini`, `deepseek`, `openrouter`, `together`, `mistral`, `fireworks`, `perplexity`, `cohere`, `xai`, `brave`, `tavily`). |

**Behavior:**

- Prompts interactively for the API key.
- Saves to `~/.opencarrier/.env` as `<PROVIDER_NAME>_API_KEY=<value>`.
- Runs a live validation test against the provider's API.
- File permissions are restricted to owner-only on Unix.

**Example:**

```bash
opencarrier config set-key groq
# Paste your groq API key: gsk_...
# [ok] Saved GROQ_API_KEY to ~/.opencarrier/.env
# Testing key... OK
```

---

### opencarrier config delete-key

Remove an API key from `~/.opencarrier/.env`.

```
opencarrier config delete-key <PROVIDER>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<PROVIDER>` | Provider name. |

**Example:**

```bash
opencarrier config delete-key openai
```

---

### opencarrier config test-key

Test provider connectivity with the stored API key.

```
opencarrier config test-key <PROVIDER>
```

**Arguments:**

| Argument | Description |
|---|---|
| `<PROVIDER>` | Provider name. |

**Behavior:**

- Reads the API key from the environment (loaded from `~/.opencarrier/.env`).
- Hits the provider's models/health endpoint.
- Reports `OK` (key accepted) or `FAILED (401/403)` (key rejected).
- Exits with code 1 on failure.

**Example:**

```bash
opencarrier config test-key groq
# Testing groq (GROQ_API_KEY)... OK
```

---

## Quick Chat

### opencarrier chat

Quick alias for starting a chat session.

```
opencarrier chat [<AGENT>]
```

**Arguments:**

| Argument | Description |
|---|---|
| `<AGENT>` | Optional agent name or UUID. |

**Behavior:**

- **Daemon mode:** Finds the agent by name or ID among running agents. If no agent name is given, uses the first available agent. If no agents exist, suggests `opencarrier agent new`.
- **Standalone mode (no daemon):** Boots an in-process kernel and auto-spawns an agent from templates. Searches for an agent matching the given name, then falls back to `assistant`, then to the first available template.

This is the simplest way to start chatting -- it works with or without a daemon.

**Example:**

```bash
# Chat with the default agent
opencarrier chat

# Chat with a specific agent by name
opencarrier chat coder

# Chat with a specific agent by UUID
opencarrier chat a1b2c3d4-e5f6-7890-abcd-ef1234567890
```

---

## Migration

### opencarrier migrate

Migrate configuration and agents from another agent framework.

```
opencarrier migrate --from <FRAMEWORK> [--source-dir <PATH>] [--dry-run]
```

**Options:**

| Option | Description |
|---|---|
| `--from <FRAMEWORK>` | Source framework. One of: `openclaw`, `langchain`, `autogpt`. |
| `--source-dir <PATH>` | Path to the source workspace. Auto-detected if not set (e.g. `~/.openclaw`, `~/.langchain`, `~/Auto-GPT`). |
| `--dry-run` | Show what would be imported without making changes. |

**Behavior:**

- Converts agent configurations, YAML manifests, and settings from the source framework into OpenCarrier format.
- Saves imported data to `~/.opencarrier/`.
- Writes a `migration_report.md` summarizing what was imported.

**Example:**

```bash
# Dry run migration from OpenClaw
opencarrier migrate --from openclaw --dry-run

# Migrate from OpenClaw (auto-detect source)
opencarrier migrate --from openclaw

# Migrate from LangChain with explicit source
opencarrier migrate --from langchain --source-dir /home/user/.langchain

# Migrate from AutoGPT
opencarrier migrate --from autogpt
```

---

## MCP Server

### opencarrier mcp

Start an MCP (Model Context Protocol) server over stdio.

```
opencarrier mcp
```

**Behavior:**

- Exposes running OpenCarrier agents as MCP tools via JSON-RPC 2.0 over stdin/stdout with Content-Length framing.
- Each agent becomes a callable tool named `opencarrier_agent_<name>` (hyphens replaced with underscores).
- Connects to a running daemon via HTTP if available; otherwise boots an in-process kernel.
- Protocol version: `2024-11-05`.
- Maximum message size: 10MB (security limit).

**Supported MCP methods:**

| Method | Description |
|---|---|
| `initialize` | Returns server capabilities and info. |
| `tools/list` | Lists all available agent tools. |
| `tools/call` | Sends a message to an agent and returns the response. |

**Tool input schema:**

Each agent tool accepts a single `message` (string) argument.

**Integration with Claude Desktop / other MCP clients:**

Add to your MCP client configuration:

```json
{
  "mcpServers": {
    "opencarrier": {
      "command": "opencarrier",
      "args": ["mcp"]
    }
  }
}
```

---

## Daemon Auto-Detect

The CLI uses a two-step mechanism to detect a running daemon:

1. **Read `daemon.json`:** On startup, the daemon writes `~/.opencarrier/daemon.json` containing the listen address (e.g. `127.0.0.1:4200`). The CLI reads this file to learn where the daemon is.

2. **Health check:** The CLI sends `GET http://<listen_addr>/api/health` with a 2-second timeout. If the health check succeeds, the daemon is considered running and the CLI uses HTTP to communicate with it.

If either step fails (no `daemon.json`, stale file, health check timeout), the CLI falls back to in-process mode for commands that support it. Commands that require a daemon (workflows, triggers, channel test/enable/disable, dashboard) will exit with an error and a helpful message.

**Daemon lifecycle:**

```
opencarrier start          # Starts daemon, writes daemon.json
                        # Other CLI instances detect daemon.json
opencarrier status         # Connects to daemon via HTTP
Ctrl+C                  # Daemon shuts down, daemon.json removed

opencarrier doctor --repair  # Cleans up stale daemon.json from crashes
```

---

## Environment File

OpenCarrier loads `~/.opencarrier/.env` into the process environment on every CLI invocation. System environment variables take priority over `.env` values.

The `.env` file stores API keys and secrets:

```bash
GROQ_API_KEY=gsk_...
ANTHROPIC_API_KEY=sk-ant-...
GEMINI_API_KEY=AIza...
TELEGRAM_BOT_TOKEN=123456:ABC-DEF...
```

Manage keys with the `config set-key` / `config delete-key` commands rather than editing the file directly, as these commands enforce correct permissions.

---

## Exit Codes

| Code | Meaning |
|---|---|
| `0` | Success. |
| `1` | General error (invalid arguments, failed operations, missing daemon, parse errors, spawn failures). |
| `130` | Interrupted by second `Ctrl+C` (force exit). |

---

## Examples

### First-time setup

```bash
# 1. Set your API key
export GROQ_API_KEY="gsk_your_key_here"

# 2. Initialize OpenCarrier
opencarrier init --quick

# 3. Start the daemon
opencarrier start
```

### Daily usage

```bash
# Quick chat (auto-spawns agent if needed)
opencarrier chat

# Chat with a specific agent
opencarrier chat coder

# Check what's running
opencarrier status

# Open the web dashboard
opencarrier dashboard
```

### Agent management

```bash
# Spawn from a template
opencarrier agent new assistant

# Spawn from a custom manifest
opencarrier agent spawn ./agents/custom-agent/agent.toml

# List running agents
opencarrier agent list

# Chat with an agent by UUID
opencarrier agent chat <UUID>

# Kill an agent
opencarrier agent kill <UUID>
```

### Workflow automation

```bash
# Create a workflow
opencarrier workflow create ./review-pipeline.json

# List workflows
opencarrier workflow list

# Run a workflow
opencarrier workflow run <WORKFLOW_ID> "Review the latest PR"
```

### Event triggers

```bash
# Create a trigger that fires on agent spawn
opencarrier trigger create <AGENT_ID> '{"agent_spawned":{"name_pattern":"*"}}' \
  --prompt "New agent spawned: {{event}}" \
  --max-fires 100

# List all triggers
opencarrier trigger list

# List triggers for a specific agent
opencarrier trigger list --agent-id <AGENT_ID>

# Delete a trigger
opencarrier trigger delete <TRIGGER_ID>
```

### Skill management

```bash
# Search FangHub
opencarrier skill search "code review"

# Install a skill
opencarrier skill install code-reviewer

# List installed skills
opencarrier skill list

# Create a new skill
opencarrier skill create

# Remove a skill
opencarrier skill remove code-reviewer
```

### Channel setup

```bash
# Interactive channel picker
opencarrier channel setup

# Direct channel setup
opencarrier channel setup telegram

# Check channel status
opencarrier channel list

# Test a channel
opencarrier channel test telegram

# Enable/disable channels
opencarrier channel enable discord
opencarrier channel disable slack
```

### Configuration

```bash
# View config
opencarrier config show

# Get a specific value
opencarrier config get default_model.provider

# Change provider
opencarrier config set default_model.provider anthropic
opencarrier config set default_model.model claude-sonnet-4-20250514
opencarrier config set default_model.api_key_env ANTHROPIC_API_KEY

# Manage API keys
opencarrier config set-key anthropic
opencarrier config test-key anthropic
opencarrier config delete-key openai

# Open in editor
opencarrier config edit
```

### Migration from other frameworks

```bash
# Preview migration
opencarrier migrate --from openclaw --dry-run

# Run migration
opencarrier migrate --from openclaw

# Migrate from LangChain
opencarrier migrate --from langchain --source-dir ~/.langchain
```

### MCP integration

```bash
# Start MCP server for Claude Desktop or other MCP clients
opencarrier mcp
```

### Diagnostics

```bash
# Run all diagnostic checks
opencarrier doctor

# Auto-repair issues
opencarrier doctor --repair

# Machine-readable diagnostics
opencarrier doctor --json
```

### Shell completions

```bash
# Generate and install completions for your shell
opencarrier completion bash >> ~/.bashrc
opencarrier completion zsh > "${fpath[1]}/_opencarrier"
opencarrier completion fish > ~/.config/fish/completions/opencarrier.fish
```

---

## Supported LLM Providers

The following providers are recognized by `opencarrier config set-key` and `opencarrier doctor`:

| Provider | Environment Variable | Default Model |
|---|---|---|
| Groq | `GROQ_API_KEY` | `llama-3.3-70b-versatile` |
| Gemini | `GEMINI_API_KEY` or `GOOGLE_API_KEY` | `gemini-2.5-flash` |
| DeepSeek | `DEEPSEEK_API_KEY` | `deepseek-chat` |
| Anthropic | `ANTHROPIC_API_KEY` | `claude-sonnet-4-20250514` |
| OpenAI | `OPENAI_API_KEY` | `gpt-4o` |
| OpenRouter | `OPENROUTER_API_KEY` | `openrouter/google/gemini-2.5-flash` |
| Together | `TOGETHER_API_KEY` | -- |
| Mistral | `MISTRAL_API_KEY` | -- |
| Fireworks | `FIREWORKS_API_KEY` | -- |
| Perplexity | `PERPLEXITY_API_KEY` | -- |
| Cohere | `COHERE_API_KEY` | -- |
| xAI | `XAI_API_KEY` | -- |

Additional search/fetch provider keys: `BRAVE_API_KEY`, `TAVILY_API_KEY`.
