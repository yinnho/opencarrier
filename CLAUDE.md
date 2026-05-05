# OpenCarrier — Agent Instructions

## Architecture Principles

**App = Brain, Carrier = Hands**

- **App 端**: 记忆管理 + 协调调度（大脑）
- **Carrier 端**: 任务执行 + 监控修复（双手）

关键原则：
1. 记忆存储在 App 端，Carrier 不持久化会话记忆
2. 记忆压缩由 App 发起，Carrier 执行（LLM 生成摘要）
3. Carrier 负责任务分解、工具调用、自动修复、结果保证

详见: [docs/ARCHITECTURE-PRINCIPLES.md](docs/ARCHITECTURE-PRINCIPLES.md)

## Project Overview
OpenCarrier is an open-source Agent Operating System written in Rust (14 crates).
- Config: `~/.opencarrier/config.toml`
- Default API: `http://127.0.0.1:4200`
- CLI binary: `target/release/opencarrier.exe` (or `target/debug/opencarrier.exe`)

## Build & Verify Workflow
After every feature implementation, run ALL THREE checks:
```bash
cargo build --workspace --lib          # Must compile (use --lib if exe is locked)
cargo test --workspace                 # All tests must pass (currently 1744+)
cargo clippy --workspace --all-targets -- -D warnings  # Zero warnings
```

## Architecture Notes
- **Don't touch `opencarrier-cli`** — user is actively building the interactive CLI
- `KernelHandle` trait avoids circular deps between runtime and kernel
- `AppState` in `server.rs` bridges kernel to API routes
- New routes must be registered in `server.rs` router AND implemented in `routes.rs`
- Dashboard is Alpine.js SPA in `static/index_body.html` — new tabs need both HTML and JS data/methods
- Config fields need: struct field + `#[serde(default)]` + Default impl entry + Serialize/Deserialize derives

## Deploy Workflow

**Always deploy via `git push deploy main`** — never modify files directly on the server.

```
git add <files>
git commit -m "..."        # Follow existing commit style
git push deploy main       # Triggers post-receive hook:
                           #   checkout → cargo build → cp binary → systemctl restart
```

- **Deploy target**: `ubuntu@106.75.129.173:/data/git/opencarrier.git` (a.k.a. `carrier.yinnho.cn`)
- **Post-receive hook**: checks out to `/data/www/opencarrier`, builds `--release`, atomic mv to `/home/ubuntu/.opencarrier/opencarrier`, then `systemctl restart opencarrier`
- **Server working dir** `/home/ubuntu/opencarrier` is a separate clone for debugging — the hook uses `/data/www/opencarrier`
- **DO NOT** scp files or edit directly on the server — it causes divergence and breaks the deploy pipeline

## Common Gotchas
- `opencarrier.exe` may be locked if daemon is running — use `--lib` flag or kill daemon first
- `PeerRegistry` is `Option<PeerRegistry>` on kernel but `Option<Arc<PeerRegistry>>` on `AppState` — wrap with `.as_ref().map(|r| Arc::new(r.clone()))`
- Config fields added to `KernelConfig` struct MUST also be added to the `Default` impl or build fails
- `AgentLoopResult` field is `.response` not `.response_text`
- CLI command to start daemon is `start` not `daemon`
- On Windows: use `taskkill //PID <pid> //F` (double slashes in MSYS2/Git Bash)
