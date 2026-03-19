# OpenCarrier Android CLI 部署方案

> 状态：规划设计阶段 | 最后更新：2026-03-19

## 概述

将 OpenCarrier 作为底层 CLI 产品在 Android 设备上运行，作为 yingheapp 的补充方案。适用于需要后台常驻、低资源占用的场景。

## 目标场景

1. **旧设备复用** - 将旧 Android 手机作为专用载体设备
2. **后台服务** - 7x24 小时运行，无需前台 UI
3. **嵌入式场景** - Android 开发板、IoT 设备
4. **测试开发** - 快速验证功能，无需编译完整 App

## 技术方案对比

| 方案 | 复杂度 | 稳定性 | 资源占用 | 适用场景 |
|------|--------|--------|----------|----------|
| Termux 环境 | 低 | 中 | 低 (~100MB) | 快速验证、个人使用 |
| 独立二进制 | 中 | 高 | 极低 | 系统集成、 rooted 设备 |
| yingheapp 集成 | 高 | 高 | 中 | 生产环境、普通用户 |

## 推荐架构

```
┌─────────────────────────────────────────────────────────────┐
│                        Android 设备                          │
│  ┌──────────────────────────────────────────────────────┐   │
│  │  Termux（Linux 容器环境）                             │   │
│  │  ┌──────────────────────────────────────────────┐    │   │
│  │  │  OpenCarrier CLI                              │    │   │
│  │  │  • 后台运行 agent 循环                          │    │   │
│  │  │  • WebSocket 连接到 relay.yinnho.cn            │    │   │
│  │  │  • 本地 HTTP API (port 8080)                   │    │   │
│  │  │  • 数据存储在 ~/yinghe/                         │    │   │
│  │  └──────────────────────────────────────────────┘    │   │
│  └──────────────────────────────────────────────────────┘   │
│                            │                                 │
│                            │ 本地网络                         │
│                            ▼                                 │
│  ┌──────────────────────────────────────────────────────┐   │
│  │  yingheapp（可选）                                    │   │
│  │  • 通过 localhost:8080 与 CLI 通信                   │   │
│  │  • 查看状态、配置参数                                  │   │
│  └──────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

## 部署方案详解

### 方案一：Termux 环境（推荐用于验证）

#### 1.1 环境准备

```bash
# 安装 Termux（从 F-Droid）
# https://f-droid.org/packages/com.termux/

# 更新包管理器
pkg update && pkg upgrade -y

# 安装依赖
pkg install -y rust git openssl clang lld
```

#### 1.2 编译安装

```bash
# 克隆代码
git clone https://github.com/yinnho/opencarrier.git
cd opencarrier

# 编译（Release 模式，减少体积）
cargo build --release --bin yinghe

# 安装到 Termux 环境
cp target/release/yinghe $PREFIX/bin/
chmod +x $PREFIX/bin/yinghe
```

#### 1.3 配置后台运行

```bash
# 安装 Termux 插件（从 F-Droid）
# - Termux:Boot（开机自启）
# - Termux:WakeLock（防止休眠）

# 创建启动脚本
mkdir -p ~/.termux/boot
cat > ~/.termux/boot/start-yinghe.sh << 'EOF'
#!/data/data/com.termux/files/usr/bin/sh
termux-wake-lock
cd ~/opencarrier
nohup yinghe > ~/yinghe.log 2>&1 &
EOF
chmod +x ~/.termux/boot/start-yinghe.sh
```

#### 1.4 绑定流程

```bash
# 运行绑定命令
yinghe bind

# 显示配对码，在应合 App 中输入
# 绑定完成后自动保存 token
```

### 方案二：交叉编译二进制

#### 2.1 在开发机编译

```bash
# 添加 Android 目标
rustup target add aarch64-linux-android

# 配置链接器（根据 NDK 路径调整）
cat >> ~/.cargo/config.toml << EOF
[target.aarch64-linux-android]
linker = "$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$(uname -s | tr '[:upper:]' '[:lower:]')-x86_64/bin/aarch64-linux-android21-clang"
EOF

# 编译
cargo build --target aarch64-linux-android --release --bin yinghe
```

#### 2.2 部署到设备

```bash
# 推送到设备
adb push target/aarch64-linux-android/release/yinghe /data/local/tmp/yinghe
adb shell chmod +x /data/local/tmp/yinghe

# 运行（需要 root 才能常驻后台）
adb shell su -c "/data/local/tmp/yinghe bind"
```

### 方案三：系统集成（高级）

创建 Android 系统服务，需要定制 ROM 或 root：

```bash
# 推送到系统目录（需要 root）
adb root
adb remount
adb push yinghe /system/xbin/
adb shell chmod 755 /system/xbin/yinghe

# 创建 init.rc 服务
cat > /etc/init/yinghe.rc << EOF
service yinghe /system/xbin/yinghe
    class main
    user system
    group system
    disabled

on property:sys.boot_completed=1
    start yinghe
EOF
```

## 与 yingheapp 集成

### 通信协议

OpenCarrier CLI 暴露本地 HTTP API：

```bash
# 查询状态
curl http://localhost:8080/api/status

# 获取配置
curl http://localhost:8080/api/config

# 发送消息（测试用）
curl -X POST http://localhost:8080/api/send \
  -H "Content-Type: application/json" \
  -d '{"message": "Hello from Android"}'
```

### yingheapp 集成点

```kotlin
// 在 yingheapp 中检查 CLI 状态
fun checkCarrierStatus(): Boolean {
    return try {
        val response = OkHttpClient()
            .newCall(Request.Builder()
                .url("http://localhost:8080/api/status")
                .build())
            .execute()
        response.isSuccessful
    } catch (e: Exception) {
        false
    }
}
```

## 待解决问题

### 高优先级

- [ ] **精简依赖** - 移除 Tauri（桌面 GUI），减少编译体积
- [ ] **后台保活** - 测试 Termux 在各种 Android 版本上的保活策略
- [ ] **电池优化** - 申请忽略电池优化白名单
- [ ] **网络切换** - 处理 WiFi/4G 切换时的重连逻辑

### 中优先级

- [ ] **自动更新** - 实现 CLI 自动检测并下载新版本
- [ ] **日志上报** - 将日志发送到云端，便于远程调试
- [ ] **安全配置** - 本地 API 添加认证，防止未授权访问
- [ ] **存储管理** - 自动清理日志和临时文件

### 低优先级

- [ ] **多实例** - 支持一台设备运行多个载体（不同配置）
- [ ] **插件系统** - 支持动态加载技能（Skill）
- [ ] **Web 管理** - 内置 Web UI，方便浏览器配置

## 依赖项清单

### 编译时依赖

| 依赖 | 版本 | 说明 |
|------|------|------|
| Rust | >= 1.75 | 主语言 |
| Android NDK | r25+ | 交叉编译（方案二） |
| OpenSSL | 3.x | TLS 支持 |

### 运行时依赖

| 依赖 | 来源 | 说明 |
|------|------|------|
| Termux | F-Droid | Linux 环境（方案一） |
| 网络权限 | Android | WebSocket 连接 |
| 存储权限 | Android | 保存配置和日志 |
| 后台运行 | Android | 保活服务 |

## 资源占用预估

| 项目 | 内存 | 存储 | 网络 |
|------|------|------|------|
| OpenCarrier CLI | 50-100MB | 100MB | 1-5MB/hour |
| Termux 环境 | 50MB | 500MB | - |
| 日志文件 | - | 10MB/day | - |

## 下一步行动

1. **验证可行性**（1-2 天）
   - 在 Termux 中编译运行 OpenCarrier
   - 测试绑定流程是否正常
   - 验证 WebSocket 连接稳定性

2. **精简改造**（3-5 天）
   - 移除 Tauri 依赖
   - 添加本地 HTTP API
   - 优化启动速度

3. **自动化脚本**（1-2 天）
   - 创建一键安装脚本
   - 配置 Termux:Boot 自启
   - 编写 systemd-style 管理脚本

4. **文档完善**（持续）
   - 用户安装指南
   - 故障排查手册
   - 与 yingheapp 集成文档

## 参考资源

- [Termux Wiki](https://wiki.termux.com/)
- [Rust Android NDK](https://github.com/rust-windowing/android-ndk-rs)
- [yingheapp 项目](../yingheapp/) - 参考 App 端实现
- [yingheclient 项目](../yingheclient/) - 参考 TypeScript 载体实现

---

*本文档为规划设计，具体实现以代码为准。*
