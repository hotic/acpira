# Windows 兼容性审查

审查日期：2026-10-02；源码基线 Acpira 1.8.0。范围涵盖 VS Code / Cursor、IDEA 后端、Rust sidecar 和内置 Agent 的平台边界。Windows 实测通过 SSH 运行离线 fixture，不调用模型、不执行真实 OAuth、不改写现有账号或 CLI 存储。

## 平台边界

| 边界 | 发现与处理 | 验证入口 |
|---|---|---|
| 可执行文件查找 | Windows 跳过 npm 同名 POSIX shim，选中 EXE / COM / CMD / BAT；批处理走平台层统一 `platform::command::spawn_spec` | `tests/windows_launch.rs`；registry 回归 |
| 安装和终端认证 | 公共 `platform/terminal.rs` 生成 UTF-16LE 编码命令，两个 IDE 复用，TypeScript 只交付启动参数；安装脚本不再经过嵌套 `-Command`；原生 argv 通过 `ProcessStartInfo` 传递，保留 PowerShell 5.1 会丢失的空参数和双引号 | `platform::terminal`；`test/windowsTerminal.test.mjs` |
| Devin 更新 | 官方 3000.11.3 安装脚本直接覆盖运行中的 EXE。先保留旧入口，失败时恢复，仍被使用的备份留待后续清理 | `test/devin-install.test.ps1`，含运行中的 EXE |
| Devin 账号 | Windows 3000.11.3 使用 Known Folders 的 Roaming AppData，忽略 XDG / APPDATA 环境覆盖。导入按真实位置读取；终端登录观察 CLI 的正常登录文件，只接受打开流程后的写入；取消不会删除该文件。查询身份前后确认全局 key 与目标一致，避免给其他保存账号贴错标签 | `accounts::devin`；原生 CLI `--version` / `auth status` 只读观测 |
| Codex 账号目录 | `link_shared` 原来在非 Unix 上直接跳过。现在两个平台复用链接原语，排除独立的 `auth.json`，保留账号已有文件 | `accounts::cli_home`，含删除账号不删除共享会话 |
| 共享配置链接 | 以文件身份识别硬链接；目录使用符号链接或原生 junction，文件回退硬链接；不再执行 `cmd /c mklink` 污染 sidecar stdout；悬空 junction 可以替换和清理 | `platform::files`；`shared_config::links` |
| 跨进程文件锁 | 原 Windows `pid_alive` 恒为 false，会接管仍存活的慢操作。现在用原生进程句柄检查退出状态，拒绝访问时按仍存活处理 | `store::file_lock`，含过期但活着的锁持有者 |
| 原子写入 | 保留标准库的 tmp + rename，不另造 Windows 覆盖实现；增加已存在文件的覆盖回归 | `store::file_lock::tests::atomic_write_replaces_existing_content` |
| 文件 URL | 集中处理盘符、UNC、`\\?\`、中文和保留字符；解码不把 query / fragment 当文件名，拒绝编码分隔符；移除中文路径上的 UTF-8 非边界切片 | `platform::file_url`；`normalize::local_path_tests` |
| Markdown 文件导航 | 保留 `%23L12` 等字面文件名；识别 UNC 目录；非法百分号编码不会使渲染抛异常 | `test/fileLinks.test.ts` |
| 安装后发现 | 补读 User / Machine 注册表 PATH，保持 IDE 原 PATH 优先级，按 Windows 规则去重；会话、身份探测和终端继承新 PATH | `login_path`；`platform/environment.rs` |
| npm 版本诊断 | CMD 包查找同时覆盖全局目录和项目 `node_modules/.bin`，避免漏掉引擎版本和缺失平台包提示 | `adapter_info::tests::cmd_shims_find_both_global_and_project_local_packages` |
| Agent 生命周期 | 复现自然退出后遗留 helper；现在挂入非继承的 kill-on-close Job Object 后才恢复执行，自然退出 / 主动终止 / 宿主强退都清理所属后代，无关进程保持存活 | `platform/windows_process.rs`；`tests/windows_compat.rs` |
| 会话锁接管 | 原 Windows 识别恒 false、终止为空操作；现在读取原生父 PID / EXE 名，仅接管另一个 sidecar 的直接子进程，终止前持有句柄并重新核对归属 | `lock_holder.rs`；`takeover_only_targets_an_agent_of_another_sidecar` |
| ChatGPT 桥接命令 | 复现带引号路径执行失败及 helper 持有输出管道导致永不结束；shell 源码使用原始 CMD 参数，复用 Job Object，在退出后完整排空输出 | `external::chatgpt_cli::tests` 的 Windows 原生用例 |
| Pi 项目信任 / 对外 cwd | 复现 Rust 的 `\\?\` 前缀与 Pi 的 Node 路径 key 不匹配；仅在外部 CLI 配置和 canonical cwd 重试处转换为普通盘符 / UNC 路径，实际文件访问保留原语义 | `platform::paths`；`shared_config::pi_trust`；Node 原生 realpath 对照 |
| Remote / WSL | 执行平台取后端 OS。机器级 `agents.json` 隔离本地 Windows 与远端 Linux 配置；Windows 客户端不决定远端命令语法 | `store/agent_config.rs`；`test/legacyAgents.test.ts` |
| 安装归档和打包 | native-release 校验 digest / ZIP 路径 / CRC / 安装锁；平台清单包含 Windows x64 / arm64；官方 MSVC 包使用静态 CRT | `native_release` 测试；`scripts/sidecar-targets.mjs`；release workflow |

## 实测与自动回归

- Windows 主机：Windows 11（OS build 26200），PowerShell 5.1.26100.8115，Node 24.13.0，Devin 3000.11.3。
- 已通过原生 Windows 的离线安装器测试，包括正在运行的 EXE；九种内置登录参数形状的 EXE / CMD 测试也通过。
- 在 macOS 的 PowerShell 7.6.6 上强制 Legacy 参数模式，复现了原生空参数丢失和双引号损坏；公共启动器绕过该 marshalling，回归通过。
- Windows x64 原生 Rust 回归：host 121、shared 22、ACP 启动 3、生命周期 4，共 150 项通过；1 项辅助进程入口按设计忽略。通过 macOS 上的 GNU 目标交叉编译，再经 SSH 运行 Windows EXE；该结果不代替发布用 MSVC 构建的 CI。
- 注册表 PATH 和 Roaming Known Folder 使用只读原生 API，并与 Windows 自身 API 返回值对照通过；Pi 路径与 Node 24.13.0 的真实 `realpathSync` 对照通过。
- `scripts/test-windows.ps1` 是统一入口，包含原生测试与所有 Windows target 的 Clippy；独立 Windows CI 在 push / PR 时运行，发版也调用同一入口。仅编译或登记 CI 不算原生执行证明。

## 本轮仓库检查

| 命令 | 结果 |
|---|---|
| `pnpm typecheck` | 通过 |
| `ACPIRA_TEST_POWERSHELL=/tmp/acpira-devin-pwsh/pwsh pnpm test` | 61 个套件、473 项通过 |
| `pnpm build` | 通过；仍有现有 CSS `::highlight` 和大包提示 |
| `RUST_TEST_THREADS=4 ACPIRA_TEST_POWERSHELL=/tmp/acpira-devin-pwsh/pwsh cargo test --workspace`（`rust/`） | 623 项通过；4 个平台 / fixture 辅助入口按设计忽略 |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings`（`rust/`） | macOS 通过；另对 `x86_64-pc-windows-gnu` 的全部 target 检查通过 |
| `./gradlew test`（`idea/`） | 前轮审查通过；本次终端结构收敛未改动 Kotlin，未重复运行 |
| Windows 原生回归 | Rust 150 项默认并发通过；PowerShell 5.1 安装 fixture 与九种 EXE / CMD 登录参数形状通过 |
| 进程残留 | macOS 无孤儿 `fake-agent.ts`；Windows 无本轮残留 fixture 进程 |

默认并发的两轮 Rust 全套分别触发已有 Steer / Fork 时序用例失败；单项复跑通过，降低测试并发后的完整套件通过。假 Agent 的 `slow` 回合只有固定的 50 × 40 ms 窗口，该测试稳定性问题仍需单独处理；本轮未改动相关业务逻辑或延长测试等待掩盖失败。

终端边界收敛复验：Windows 脚本生成仅保留在 Rust，`platform/` 不再引用 `acp::agents`。通用命令与参数转义测试迁入 `platform::command`，安装源代码和九种登录参数的 EXE / CMD 执行测试迁入 `platform::terminal`，TypeScript 只验证启动参数交付和 POSIX 行为。此次 Rust 全量首轮在 `agent_emitted_images_land_in_the_blob_store` 读到空字节，测试仅等待文件出现而写入异步进行；单项与完整工作区复跑通过，相关逻辑和测试未修改。Windows 终端交付、Devin 离线安装及进程残留检查也通过。

## 覆盖边界

- 终端边界收敛时新增的组合压力输入暴露了既有 CMD 限制（2026-10-02，Windows 11 / PowerShell 5.1.26100.8115 / Node 24.13.0）：参数中先有嵌入双引号，后有 `& %ACPIRA_LITERAL_TEST% !bang! ^` 这样的单个参数时，`%*` shim 的第二次解析会截断参数并尝试执行其余文本。迁移前后的 `spawn_spec` 输出逐字节相同，原生执行均复现失败；双重转义可修好该 shim，但会给使用 `%~1` 的普通批处理传入额外的 `^"`。本次结构收敛保留既有规则；该缺口尚未修复，后续需明确区分转发型 shim 与自行读取参数的批处理，不能全局增加转义层数。九种内置登录参数不触发此组合。
- SSH 离线测试覆盖进程、参数、环境、文件系统和协议握手；未完成每家厂商的真实浏览器 OAuth，也不替代两种 IDE 内的按钮、终端交互和浏览器回跳验收。
- Windows arm64 的打包与目标定义已审查，当前原生执行主机为 x64；需 arm64 主机补测。
- MCP 的本机路径解析和配置合并已验证；各厂商 Windows 版本如何启动其接收的 stdio MCP command，尚未逐家运行真实 CLI 验收。
- 未获得 Developer Mode / 符号链接权限时，文件硬链接受同卷限制，且源文件被原子替换后不会自动跟随；目录 junction 不受文件硬链接的这项限制。现有账号文件保留，不强制覆盖账号私有配置。
- 共享盘实际权限、重定向 Known Folders、企业进程限制 / 杀毒软件、禁用 PowerShell 或 WSL interop 的自定义终端仍需对应环境验收。
- Desktop Commander 在 Windows 上的进程识别仍报告 `unknown`；可执行文件 / 配置存在性不冒充已连接或已配对。
- 修改目前在工作区，未发布到 Marketplace，已安装的 1.8.0 不会自动获得本轮修复。

## 发版 CI 的短路径回归（2026-10-02）

GitHub Windows runner 的临时目录包含 `RUNNER~1`。Node 22 的普通 `fs.realpathSync` 保留短路径，Rust `std::fs::canonicalize` 将其展开为 `runneradmin`，导致 Node 路径对照与 Pi 父目录信任用例失败。Windows 11 / Node 24.13.0 上的新建隔离目录同样复现：普通 realpath 保留 `ACPIRA~1.1-S`，native realpath 展开为长名称。`platform::paths::canonical_for_cli` 按组件解析符号链接 / junction，并保留普通路径的大小写和 8.3 拼写；Pi 信任与原生会话 cwd 重试共用该边界。新增回归对照 Node 的短路径、大小写、父目录折叠与 junction 输出。

修复自验：macOS Rust 工作区 623 项通过；macOS 与 Windows GNU 目标的全部 target Clippy 通过；Windows 11 / Node 24.13.0 上 144 项库测试通过，1 项辅助进程入口忽略。新增用例显式调用 Windows 短路径 API，覆盖短名称、大小写、父目录折叠和 junction 的真实 Node realpath 对照。该结果仍不替代修复版本的 MSVC 发布 CI。

## Claude 认证状态与静默重试（2026-10-07）

Windows 11 / claude-agent-acp 0.84.0 / Claude Code 2.1.284 的真实会话记录出现 7 次 `401 API key is invalid`。用户级 `ANTHROPIC_API_KEY` 仍被 CLI 采用，未保存 Acpira 账号时的「未登录」文案不代表缺少 CLI 凭据。适配器忽略 `api_retry` 中的认证警告，持续等待期间界面没有错误。修复详见 `agent-quirks.md`：明确未认证时在启动阶段拦截，连续 401 结束回合，账号空列表改为「尚未添加账号」。

原生 Windows x64 验收通过：GNU 目标编译的临时 smoke 程序调用已安装的 `.cmd` 适配器，隔离配置与凭据目录、仅在测试进程中清除认证环境，未发送 prompt 即进入 `auth_required`；离线 Node fixture 回放连续 401 后，运行状态结束且已有输出保留。未安装替换用户的扩展，未改动用户级环境变量，未发送真实模型请求；不替代正式 MSVC 发布构建。
