# LocalAPI

把网页版大模型（DeepSeek / 通义千问 / 豆包 / ChatGPT）包装成本地 **OpenAI 兼容 API** 的小网关。
不需要官方 API Key，复用浏览器里的登录态，适合个人实验和本地工具接入。

```
OpenAI 客户端（opencode / pi / curl）
        │  HTTP，OpenAI 协议
        ▼
LocalAPI（Rust + axum）      默认 127.0.0.1:5688
        │  stdin/stdout，行 JSON-RPC
        ▼
cjs/*-driver.cjs（Node，无第三方依赖）
        │  CDP over WebSocket
        ▼
Edge / Chrome  ──DOM──▶ chat.deepseek.com 等网页
```

## 1. 能力与边界

支持：

- `GET /v1/models`
- `POST /v1/chat/completions`（非流式 + SSE 流式）
- `tools` 参数：转成提示词让网页版输出 `tool_call`，再解析成 OpenAI 的 `tool_calls`
- 多站点驱动：DeepSeek、千问、豆包、ChatGPT，按配置切换
- 多浏览器 profile：不同账号各用一份登录态
- 命令行：`run` 启动、`init` 初始化配置、`clean` 清空运行数据

限制：

- 依赖网页 DOM，站点改版可能导致发送或提取回复失败
- 网页端只适合串行，默认 `MaxConcurrent = 1`
- 流式是“假流式”：等网页回答完成后，再按字符切片发 SSE
- `usage` 恒为 0（网页版拿不到 token 统计）
- 只把最后一条消息发给网页版，历史由网页端自己保留
- 没有鉴权，默认只监听 `127.0.0.1`，不要直接暴露到公网

## 2. 环境要求

- Windows（开发测试环境）；理论支持 macOS/Linux，未验证
- Node.js 18+（驱动脚本，无 npm 依赖）
- Chrome 或 Edge（自动探测，可用 `ChromePath` / `DSWEB_CHROME` 指定）
- Rust 工具链（自行编译时）

## 3. 初始化与启动

在项目根目录执行（`.data`、`cjs/` 都是相对路径，运行目录会影响查找）：

```powershell
cargo build --release
.\target\release\LocalAPI.exe init     # 生成默认 config.json（可省略）
.\target\release\LocalAPI.exe run
```

或者用 Makefile 打包到 `dist`（exe + 驱动脚本）：

```powershell
make          # target\release\LocalAPI.exe + cjs\ → dist\
make run      # 在 dist 目录启动
```

`dist` 目录结构：

```
dist\
  LocalAPI.exe
  cjs\
    deepseek-driver.cjs
    qwen-driver.cjs / doubao-driver.cjs / gpt-driver.cjs
    lib\driver-core.cjs
    sites\*.cjs
  config.json        # 若根目录存在会自动复制
```

看到下面这行即启动成功：

```
INFO gateway listening addr=127.0.0.1:5688 site=deepseek ...
```

首次使用先登录（会弹出独立的 Edge/Chrome 窗口）：

```powershell
curl.exe http://127.0.0.1:5688/login
```

在窗口里登录并勾选“保持登录”，登录成功（检测到聊天输入框）后返回 JSON。
Cookie 存在 `Base` 目录下的 profile 里，下次启动不用再登录。

验证：

```powershell
curl.exe http://127.0.0.1:5688/v1/models
curl.exe http://127.0.0.1:5688/status
```

## 4. 命令行

```
LocalAPI.exe [COMMAND]
```

| 命令 | 说明 |
|---|---|
| `run`（默认） | 启动 HTTP 网关；不带子命令等价于 `run` |
| `init` | 生成默认 `config.json`，默认写项目根 |
| `clean` | 清空运行数据（浏览器 profile、Cookie、校准数据） |

### init

```powershell
LocalAPI.exe init                        # 项目根 config.json
LocalAPI.exe init --global               # %APPDATA%\dsh-web-gateway\config.json
LocalAPI.exe init --path D:\cfg\a.json   # 指定路径
LocalAPI.exe init --force                # 覆盖已有配置
```

目标优先级：`--path` > `--global` > `DSWEB_CONFIG` > 项目根。默认不覆盖已有文件。
`init` / `clean` 也接受首字母大写写法（`run` 另有别名 `serve`）。

### clean

```powershell
LocalAPI.exe clean --dry-run   # 只打印将要删除的目录
LocalAPI.exe clean             # 删除 Base（profile/Cookie/校准）
LocalAPI.exe clean --logs      # 同时删除日志目录
```

安全保护：拒绝清理项目根目录、包含项目的上级目录或磁盘根目录。
执行前请先停掉网关和该 profile 的浏览器，否则文件可能被占用。

## 5. 配置

查找顺序：

1. `DSWEB_CONFIG` 指定的文件
2. 项目根 `config.json`
3. `%APPDATA%\dsh-web-gateway\config.json`

都没找到时使用默认值。环境变量优先级最高，适合临时切换站点或驱动。

字段（默认值）：

| 字段 | 默认值 | 说明 |
|---|---|---|
| `Host` | `127.0.0.1:5688` | 监听地址 |
| `LogLeastLevel` | `""` | 日志级别；空则读 `RUST_LOG`，否则 `info` |
| `LogDir` | `logs` | 日志目录，按日期写 `YYYY-MM-DD.log` |
| `Driver` | `cjs/deepseek-driver.cjs` | 驱动入口脚本 |
| `Site` | `""` | 站点标识；空则从驱动文件名推断（默认 deepseek） |
| `Base` | `.data` | 运行数据目录：profile、Cookie、校准 |
| `Headless` | `true` | 请求用无头浏览器；登录接口始终有头 |
| `TimeoutSecs` | `240` | 等待网页回答的秒数 |
| `LoginWaitSecs` | `300` | `/login` 等待人工登录的秒数 |
| `MaxConcurrent` | `1` | 同时占用浏览器的请求数 |
| `ChromePath` | `null` | 浏览器可执行文件；空则自动探测 |
| `Proxy` | `""` | 浏览器代理，例如 `http://127.0.0.1:7897` |
| `IgnoreTools` | `false` | 为 true 时忽略请求里的 `tools` |
| `SpeedControl` | `null` | 保留字段，当前未使用 |

换站点示例：

```powershell
$env:DSWEB_SITE='qwen'
$env:DSWEB_DRIVER='cjs/qwen-driver.cjs'
.\target\release\LocalAPI.exe run
```

## 6. 接口

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/health` | 健康检查 |
| GET | `/control` | 浏览器控制台：站点、登录入口、状态链接 |
| GET | `/status` | 驱动、浏览器、登录态、模型徽标 |
| GET | `/login` | 打开有头浏览器等待人工登录 |
| POST | `/ping` | 检查模型是否出现在驱动列表 |
| GET | `/v1/models` | OpenAI 模型列表 |
| POST | `/v1/chat/completions` | OpenAI 对话补全 |

### `GET /login`

参数均可省略：

- `site` / `vendor`：`deepseek` / `qwen` / `doubao` / `gpt`，默认当前站点
- `profile`：浏览器 profile 名，默认 `default`
- `wait`：本次等待秒数，默认 `LoginWaitSecs`
- `driver`：自定义驱动脚本路径

```powershell
curl.exe "http://127.0.0.1:5688/login?site=qwen&profile=default&wait=300"
```

普通 `/login` 不会清空旧登录态。要换账号：停网关 → 删掉对应 profile 目录 → 再登录。

### `GET /v1/models`

当前站点决定返回哪一组模型：

| 站点 | 模型 id |
|---|---|
| deepseek | `deepseek-chat`、`deepseek-reasoner` |
| qwen | `qwen3.7-plus`、`qwen3.8-max` |
| doubao | `doubao-pro`、`doubao-think`、`doubao-expert` |
| gpt | `auto`、`gpt-5-5`、`gpt-5-4-thinking`、`gpt-5-3-mini`、`gpt-5-mini` |

模型列表优先向驱动查询，失败时使用内置兜底列表。

### `POST /v1/chat/completions`

```powershell
curl.exe -s http://127.0.0.1:5688/v1/chat/completions `
  -H "Content-Type: application/json" `
  -d "{\"model\":\"deepseek-chat\",\"messages\":[{\"role\":\"user\",\"content\":\"只回答两个字：你好\"}]}"
```

`stream: true` 时返回标准 SSE：

```
data: {"object":"chat.completion.chunk",...}
data: [DONE]
```

响应头带 `x-request-id`，日志里同一 id 可用于对请求。

### `POST /ping`

```powershell
curl.exe -s http://127.0.0.1:5688/ping -H "Content-Type: application/json" -d "[]"
curl.exe -s http://127.0.0.1:5688/ping -H "Content-Type: application/json" -d "{\"models\":[\"deepseek-chat\"]}"
```

## 7. 接入 opencode

编辑 `~/.config/opencode/opencode.jsonc`：

```jsonc
{
  "provider": {
    "localapi": {
      "name": "LocalAPI (本地网页网关)",
      "npm": "@ai-sdk/openai-compatible",
      "options": {
        "baseURL": "http://127.0.0.1:5688/v1",
        "apiKey": "sk-local-placeholder"
      },
      "models": {
        "deepseek-chat": { "name": "DeepSeek 快速", "tool_call": true },
        "deepseek-reasoner": { "name": "DeepSeek 专家", "tool_call": true }
      }
    }
  }
}
```

## 8. 环境变量

| 变量 | 说明 |
|---|---|
| `DSWEB_CONFIG` | 配置文件路径 |
| `DSWEB_ADDR` / `DSWEB_HOST` | 监听地址 |
| `DSWEB_LOG_LEVEL` / `RUST_LOG` | 日志级别 |
| `DSWEB_LOG_DIR` | 日志目录 |
| `DSWEB_DRIVER` | 驱动脚本 |
| `DSWEB_SITE` / `DS_WEB_SITE` | 站点标识 |
| `DSWEB_BASE` / `DS_WEB_BASE` | 运行数据目录 |
| `DSWEB_HEADLESS` | `1` 无头，`0` 有头 |
| `DSWEB_TIMEOUT_SECS` | 等待回答秒数 |
| `DSWEB_LOGIN_WAIT_SECS` | 登录等待秒数 |
| `DSWEB_MAX_CONCURRENT` | 并发数 |
| `DSWEB_CHROME` / `DS_WEB_CHROME` | 浏览器路径 |
| `DSWEB_PROXY` / `DS_WEB_PROXY` | 浏览器代理 |
| `DSWEB_IGNORE_TOOLS` | `1` 忽略 tools |

## 9. 数据目录

| 路径 | 内容 |
|---|---|
| `Base\profiles\<site>-<profile>\` | 站点 profile：Cookie、localStorage、缓存（DeepSeek 为 `profiles\<profile>\`） |
| `Base\calibration.json` | DeepSeek 专家模式校准数据（首次请求生成） |
| `logs\YYYY-MM-DD.log` | 按日期滚动日志 |
| `config.json` | 配置文件 |

`clean` 默认删除 `Base`，等于清空所有登录态和缓存；`--logs` 才删日志。

## 10. 常见问题

| 现象 | 处理 |
|---|---|
| `502 ... not logged in` | 打开 `/login` 重新登录；确认勾选保持登录 |
| 端口被占用 / `bind failed` | 改 `Host`，或关掉占用端口的进程 |
| 找不到浏览器 | 设置 `ChromePath` 或 `DSWEB_CHROME` |
| ChatGPT/豆包地区受限 | 配置 `Proxy`（如 `http://127.0.0.1:7897`）后重新登录 |
| 驱动脚本不存在 | 确认 `Driver` 指向 `cjs/*-driver.cjs`，且从项目根或 `dist` 启动 |
| 回复一直为空 | 先看 `/status`，再查 `logs`；网页改版时驱动选择器可能失效 |

### 停止

```powershell
Get-Process LocalAPI -ErrorAction SilentlyContinue | Stop-Process -Force
```

后台运行时日志在 `logs\`；`/login` 打开的浏览器窗口可手动关闭。
