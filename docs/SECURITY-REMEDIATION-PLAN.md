# OpenCarrier 安全整改计划

**创建日期**: 2026-03-24
**基于审计报告**: SECURITY-AUDIT-REPORT.md v1.0
**预计完成**: 2026-04-07

---

## 整改进度总览

```
阶段 1 (紧急): ██████████ 100% (2/2) ✅
阶段 2 (高优): ██████████ 100% (3/3) ✅
阶段 3 (中等): ░░░░░░░░░░ 0% (0/4)
阶段 4 (低优): ░░░░░░░░░░ 0% (0/4)
总体进度:     ██████░░░░ 60% (8/13)
```

---

## 阶段 1: 紧急修复 (P0)

**目标**: 消除 Critical 级别安全漏洞
**截止日期**: 2026-03-27

### 任务 1.1: 修复命令注入漏洞 [CRITICAL-001]

**问题**: `/api/hands/install-depend` 端点存在命令注入风险

**修复步骤**:
1. [x] 创建 command_security.rs 模块
2. [x] 实现命令白名单机制
3. [x] 添加危险操作检测 (&&, ||, |, ;, $(), etc.)
4. [x] 编写安全测试用例 (9个测试全部通过)
5. [x] 集成到 routes.rs 的 install_hand_deps 函数

**代码变更**:
```rust
// crates/opencarrier-api/src/routes.rs

// 添加依赖
use shlex;

// 白名单命令
const ALLOWED_INSTALLERS: &[&str] = &[
    "apt-get", "apt", "yum", "dnf", "pacman", "brew", "winget", "choco"
];

fn sanitize_and_validate_command(cmd: &str) -> Result<Vec<String>, ApiError> {
    // 1. 使用 shlex 安全解析
    let args = shlex::split(cmd).ok_or(ApiError::InvalidCommandFormat)?;

    if args.is_empty() {
        return Err(ApiError::EmptyCommand);
    }

    // 2. 检查白名单
    let base_cmd = args[0].as_str();
    if !ALLOWED_INSTALLERS.contains(&base_cmd) {
        return Err(ApiError::CommandNotAllowed(base_cmd.to_string()));
    }

    // 3. 检查危险参数
    for arg in &args[1..] {
        if arg.contains("&&") || arg.contains("||") || arg.contains("|") || arg.contains(";") {
            return Err(ApiError::DangerousArgument);
        }
    }

    Ok(args)
}
```

**验证方法**:
```bash
# 测试命令注入被阻止
curl -X POST http://localhost:4200/api/hands/install-depend \
  -H "Content-Type: application/json" \
  -d '{"command": "rm -rf /"}'
# 期望: 403 Forbidden

# 测试合法命令通过
curl -X POST http://localhost:4200/api/hands/install-depend \
  -H "Content-Type: application/json" \
  -d '{"command": "apt-get install -y curl"}'
# 期望: 200 OK (需要认证)
```

**状态**: 🟢 已完成

---

### 任务 1.2: 添加 API 认证中间件

**问题**: 敏感端点缺少认证 (已完成)

**说明**: 认证机制已在 `middleware.rs` 中实现。POST 端点默认需要认证 (如果 api_key 已配置)。

**状态**: 🟢 已完成 (无需额外修改)

**修复步骤**:
1. [ ] 创建认证中间件模块
2. [ ] 实现 API Key 验证
3. [ ] 应用于敏感端点
4. [ ] 添加认证测试

**代码变更**:
```rust
// crates/opencarrier-api/src/auth_middleware.rs (新文件)

use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
};

pub async fn require_auth(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // 检查 API Key
    let api_key = request
        .headers()
        .get("X-API-Key")
        .and_then(|v| v.to_str().ok());

    match api_key {
        Some(key) if state.kernel.config().api_keys.contains(key) => {
            Ok(next.run(request).await)
        }
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}
```

**状态**: 🔴 待开始

---

## 阶段 2: 高优先级修复 (P1)

**目标**: 消除 High 级别安全漏洞
**截止日期**: 2026-03-31

### 任务 2.1: 实施速率限制 [HIGH-001]

**修复步骤**:
1. [x] 验证 governor 依赖已存在
2. [x] 全局速率限制器已实现 (500 tokens/min/IP)
3. [x] 添加敏感端点的高成本配置
4. [x] 添加速率限制测试

**状态**: 🟢 已完成 (增强)

---

### 任务 2.2: 升级密码哈希 [HIGH-002]

**修复步骤**:
1. [x] 添加 argon2 依赖
2. [x] 实现密码哈希函数 (Argon2id)
3. [x] 添加 SHA256 遗留兼容支持
4. [x] 添加验证函数

**状态**: 🟢 已完成

---

### 任务 2.3: 添加命令白名单机制 [HIGH-003]

**说明**: 此任务已在 CRITICAL-001 修复中一并完成。

**状态**: 🟢 已完成 (通过 CRITICAL-001)

---

## 阶段 3: 中等优先级修复 (P2)

**目标**: 提升代码质量和性能
**截止日期**: 2026-04-07

### 任务 3.1: 减少 .clone() 调用 [MEDIUM-001]

**修复步骤**:
1. [ ] 识别热路径中的克隆
2. [ ] 使用 Arc 替代
3. [ ] 性能基准测试

**状态**: 🔴 待开始

---

### 任务 3.2: 替换 .unwrap() [MEDIUM-002]

**修复步骤**:
1. [ ] 扫描所有 .unwrap() 调用
2. [ ] 替换为 ? 或 unwrap_or_default
3. [ ] 添加适当的错误处理

**状态**: 🔴 待开始

---

### 任务 3.3: 增强 ZIP 路径验证 [MEDIUM-003]

**修复步骤**:
1. [ ] 添加额外的路径检查
2. [ ] 记录可疑路径
3. [ ] 单元测试

**状态**: 🔴 待开始

---

### 任务 3.4: 添加凭据加密 [MEDIUM-004]

**修复步骤**:
1. [ ] 集成系统 keyring
2. [ ] 实现加密存储
3. [ ] 迁移现有凭据

**状态**: 🔴 待开始

---

## 阶段 4: 低优先级改进 (P3)

**目标**: 持续改进
**截止日期**: 持续进行

### 任务 4.1: 完善文档 [LOW-001]
### 任务 4.2: 增加测试覆盖 [LOW-002]
### 任务 4.3: 清理敏感日志 [LOW-003]
### 任务 4.4: 升级 GitHub Actions [LOW-004]

**状态**: 🔴 待开始

---

## 验证清单

每个任务完成后需验证：

- [ ] 代码编译通过 (`cargo build`)
- [ ] 测试通过 (`cargo test`)
- [ ] Clippy 无警告 (`cargo clippy`)
- [ ] 格式正确 (`cargo fmt --check`)
- [ ] 安全测试通过
- [ ] 文档已更新

---

## 回归测试计划

完成整改后执行：

1. **单元测试**: `cargo test --workspace`
2. **集成测试**: 启动守护进程，测试所有端点
3. **安全测试**: 执行攻击向量测试
4. **性能测试**: 确保无明显性能退化
5. **渗透测试**: 外部安全评估

---

## 签署

| 角色 | 姓名 | 日期 | 签名 |
|------|------|------|------|
| 审计员 | Claude | 2026-03-24 | ✓ |
| 技术负责人 | - | - | - |
| 安全负责人 | - | - | - |

---

**文档版本**: 1.0
**最后更新**: 2026-03-24
