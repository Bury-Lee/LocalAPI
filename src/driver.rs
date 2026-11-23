//! 与 Node 版 `cjs/*-driver.cjs` 的通信层。
//!
//! 协议是「一行一个 JSON」的 RPC（不是 LSP 那种 `Content-Length` 分帧）：
//! - 请求：`{"id":1,"method":"streamAsk","params":{...}}`
//! - 应答：`{"id":1,"ok":true,"result":{...}}`
//! - 事件：`{"event":"stream-end","streamId":"s1",...}`（无 id，异步推送）
//!
//! 底层是推送模型，上层想要 `Future`，所以用三张表

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{oneshot, watch, Mutex as AsyncMutex};
use tracing::{debug, info, warn};

// driver 解析出来的工具调用：形如 {"type":"tool_call","name":"...","args":{...}}
// 注意：cjs 驱动返回的键是 `arguments`（JSON 字符串），旧版才是 `args`，两个都要收。
#[derive(Clone, Debug, Deserialize)]
pub struct DriverToolCall {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub args: Option<Value>,
    #[serde(default)]
    pub arguments: Option<Value>,
}

impl DriverToolCall {
    /// 取参数：优先 `args`，退回 `arguments`（字符串或对象都可能）。
    pub fn args_value(&self) -> Option<&Value> {
        self.args.as_ref().or(self.arguments.as_ref())
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct StreamEnd {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub result: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    // 驱动用的是驼峰 toolCalls；serde 默认不会自动匹配，必须显式 rename
    #[serde(default, rename = "toolCalls")]
    pub tool_calls: Vec<DriverToolCall>,
}

/// DS网页端的特殊性,其模型切换是通过点击一个"深度思考"的功能实现,因此需要使用录制的方式来进行切换.
/// 专家模式的“校准数据”：driver 回放这串点击来切模型。
/// 首次启动写到 `<base>/calibration.json`，driver 自己读（专门给deepseek 驱动用）。
const CALIBRATION: &str = r#"{
  "deepseek-reasoner": [
    { "tag": "div", "cls": "dfb78875", "text": "专家模式", "aria": "", "role": "" }
  ]
}"#;

/// 启动驱动时下发的运行时参数（不经过环境变量）。
#[derive(Clone, Debug, Default)]
pub struct DriverOptions {
    /// 浏览器可执行文件（绝对路径），空则驱动自动探测
    pub chrome_path: Option<String>,
    /// 浏览器代理，空则不用
    pub proxy: String,
}

/// 相对路径一律按程序根目录展开，避免 Node/浏览器子进程解析不一致。
fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        crate::app_root::Root().join(path)
    }
}

fn is_deepseek(driver_path: &Path) -> bool {
    driver_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.starts_with("deepseek"))
        .unwrap_or(false)
}

pub struct Driver {
    // 保留句柄：tokio 的 Child 默认不会 kill_on_drop，丢了就成孤儿进程；
    // 同时这也是超时/退出时 kill 的入口。
    child: Mutex<Child>,
    // 多个请求可能同时写同一个管道，必须串行化；因为跨 .await 持锁，只能用 tokio 的异步 Mutex
    stdin: AsyncMutex<ChildStdin>,
    // 请求 id 用原子量
    seq: AtomicU64,
    // std::sync::Mutex 足够：临界区里没有 .await，比 tokio::Mutex 便宜
    pending: Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>,
    // streamId -> 正在 await 最终结果的调用方
    streams: Mutex<HashMap<String, oneshot::Sender<StreamEnd>>>,
    // stream-end 可能比 wait_stream 先到，先存这里，避免结果丢失（竞态补偿）
    early: Mutex<HashMap<String, StreamEnd>>,
    // watch 适合“状态变化广播”：用 borrow() 读当前值，send(true) 唤醒等待者
    ready_tx: watch::Sender<bool>,
    ready_rx: watch::Receiver<bool>,
    // stdout 读到 EOF 时置 false，让等待方立刻失败，而不是干等到超时
    alive: AtomicBool,
}

impl Driver {
    pub async fn start(
        driver_path: &Path,
        base: &Path,
        options: &DriverOptions,
    ) -> Result<Arc<Driver>, String> {
        // 配置里的相对路径按程序根目录补全：浏览器/Node 子进程对相对路径的解析
        // 不一定跟网关一致（尤其 `--user-data-dir`），必须传绝对路径。
        let driver_path = absolute_path(driver_path);
        let base = absolute_path(base);
        // 只有 deepseek 需要校准数据；其它驱动的 calibration.json 没有消费方
        if is_deepseek(&driver_path) {
            std::fs::create_dir_all(&base).map_err(|e| format!("mkdir base: {e}"))?;
            let calib = base.join("calibration.json");
            if !calib.exists() {
                let _ = std::fs::write(&calib, CALIBRATION);
            }
        }
        // 三段管道：stdin 发请求，stdout 收应答/事件，stderr 转发日志
        let mut cmd = Command::new("node");
        cmd.arg(&driver_path)
            .current_dir(crate::app_root::Root())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("spawn node（检查 Node 是否在 PATH）: {e}"))?;
        info!(
            pid = ?child.id(),
            driver = %driver_path.display(),
            base = %base.display(),
            "driver spawned"
        );
        // take() 把管道所有权从 Child 手里拿走（对应字段变成 None），之后由 Driver 持有
        let stdin = child.stdin.take().ok_or("no child stdin")?;
        let stdout = child.stdout.take().ok_or("no child stdout")?;
        let stderr = child.stderr.take().ok_or("no child stderr")?;
        // watch::channel 初值 false；读取任务收到 ready 事件时 send(true)
        let (ready_tx, ready_rx) = watch::channel(false);
        let driver = Arc::new(Driver {
            child: Mutex::new(child),
            stdin: AsyncMutex::new(stdin),
            seq: AtomicU64::new(0),
            pending: Mutex::new(HashMap::new()),
            streams: Mutex::new(HashMap::new()),
            early: Mutex::new(HashMap::new()),
            ready_tx,
            ready_rx,
            alive: AtomicBool::new(true),
        });
        // 共享 Arc：读取任务需要回调 dispatch，而 Driver 又要留给调用方使用
        tokio::spawn(read_stdout(driver.clone(), stdout));
        tokio::spawn(read_stderr(driver.clone(), stderr));
        // 启动失败时主动收回node 进程
        match driver.wait_ready(Duration::from_secs(25)).await {
            Ok(()) => {
                // 浏览器路径/代理通过 RPC 下发
                let config = json!({
                    "config": {
                        "chromePath": options.chrome_path.clone().unwrap_or_default(),
                        "proxy": options.proxy.clone(),
                    }
                });
                if let Err(err) = driver
                    .rpc("config", config, Duration::from_secs(5))
                    .await
                {
                    warn!(error = %err, "driver config rpc failed");
                }
                Ok(driver)
            }
            Err(e) => {
                if let Ok(mut c) = driver.child.lock() {
                    let _ = c.start_kill();
                }
                Err(e)
            }
        }
    }

    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    // 轮询而不是单纯等 watch 通知：顺便检查进程是否已死，能更快报错
    pub async fn wait_ready(&self, dur: Duration) -> Result<(), String> {
        let deadline = tokio::time::Instant::now() + dur;
        loop {
            // borrow() 读 watch 的当前值（不阻塞），send(true) 后这里立刻可见
            if *self.ready_rx.borrow() {
                return Ok(());
            }
            if !self.is_alive() {
                return Err("driver exited before ready（看 stderr 日志：路径/Node/浏览器）".to_string());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err("driver not ready (timeout)".to_string());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    // 发一条 RPC 并等对应 id 的应答；id 是请求/响应之间唯一的关联键
    pub async fn rpc(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        if !self.is_alive() {
            return Err("driver is not running".to_string());
        }
        // 先登记 oneshot 再写管道：应答可能来得非常快，顺序反了会丢
        let id = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        // 协议以换行分帧，所以写入时一定要补 \n
        let line = json!({ "id": id, "method": method, "params": params }).to_string();
        {
            let mut stdin = self.stdin.lock().await;
            if let Err(e) = stdin.write_all(format!("{line}\n").as_bytes()).await {
                self.pending.lock().unwrap().remove(&id);
                return Err(format!("write to driver: {e}"));
            }
        }
        // timeout 包住的是本地等待，不会取消 driver 侧的任务；超时后清理登记防泄漏
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(res)) => res,
            Ok(Err(_)) => Err("driver dropped the request".to_string()),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                Err(format!("driver rpc timeout: {method}"))
            }
        }
    }

    // streamAsk 只返回 streamId，最终文本靠后续 stream-end 事件送达
    pub async fn wait_stream(&self, stream_id: &str, timeout: Duration) -> Result<StreamEnd, String> {
        // 竞态：事件可能比本函数更早到达，dispatch 会暂存在 early
        if let Some(end) = self.early.lock().unwrap().remove(stream_id) {
            return Ok(end);
        }
        let (tx, rx) = oneshot::channel();
        self.streams
            .lock()
            .unwrap()
            .insert(stream_id.to_string(), tx);
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(end)) => Ok(end),
            Ok(Err(_)) => Err("driver dropped the stream".to_string()),
            Err(_) => {
                self.streams.lock().unwrap().remove(stream_id);
                Err("waiting for the reply timed out".to_string())
            }
        }
    }

    // 先让 driver 走自己的 shutdown（它会顺带关掉浏览器），再 kill 兜底
    pub async fn shutdown(&self) {
        if !self.is_alive() {
            return;
        }
        let _ = self
            .rpc("shutdown", json!({}), Duration::from_secs(3))
            .await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        if let Ok(mut c) = self.child.lock() {
            let _ = c.start_kill();
        }
    }

    // 分发一行 JSON：带 id 的是 RPC 应答，否则按 event 处理
    fn dispatch(&self, value: Value) {
        if let Some(id) = value.get("id").and_then(|v| v.as_u64()) {
            if let Some(tx) = self.pending.lock().unwrap().remove(&id) {
                let out = if value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                    Ok(value.get("result").cloned().unwrap_or(Value::Null))
                } else {
                    Err(value
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("driver error")
                        .to_string())
                };
                let _ = tx.send(out);
            }
            return;
        }
        let event = value.get("event").and_then(|v| v.as_str()).unwrap_or("");
        match event {
            "ready" => {
                let _ = self.ready_tx.send(true);
            }
            // 流式结果的终点：有等待者就直接交付，没有就说明来早了，存进 early
            "stream-end" => {
                let sid = match value.get("streamId").and_then(|v| v.as_str()) {
                    Some(s) => s.to_string(),
                    None => return,
                };
                // 反序列化失败就退化成默认值，至少让等待方拿到一个 ok=false
                let end: StreamEnd = serde_json::from_value(value).unwrap_or_default();
                let tx = self.streams.lock().unwrap().remove(&sid);
                match tx {
                    Some(tx) => {
                        let _ = tx.send(end);
                    }
                    None => {
                        self.early.lock().unwrap().insert(sid, end);
                    }
                }
            }
            "progress" | "login-progress" => {
                let detail = value
                    .get("detail")
                    .or_else(|| value.get("url"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if !detail.is_empty() {
                    debug!(event, detail, "driver event");
                }
            }
            _ => {}
        }
    }

    // 进程异常退出：把所有等待者直接唤醒为错误，避免请求一直挂到超时
    fn fail_all(&self, reason: &str) {
        self.alive.store(false, Ordering::SeqCst);
        let mut pending = self.pending.lock().unwrap();
        for (_, tx) in pending.drain() {
            let _ = tx.send(Err(reason.to_string()));
        }
        let mut streams = self.streams.lock().unwrap();
        for (_, tx) in streams.drain() {
            let _ = tx.send(StreamEnd {
                ok: false,
                error: Some(reason.to_string()),
                ..Default::default()
            });
        }
    }
}

// 唯一的 stdout 读取者：BufReader::lines 按 \n 切行，一行一个 JSON
async fn read_stdout(driver: Arc<Driver>, stdout: tokio::process::ChildStdout) {
    let mut lines = BufReader::new(stdout).lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                if let Ok(value) = serde_json::from_str::<Value>(line) {
                    driver.dispatch(value);
                }
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }
    driver.fail_all("driver exited");
    warn!("driver process exited, all pending requests rejected");
}

// driver 把自身日志（如 [qwen] launching ...）写在 stderr，这里原样转发到 tracing
async fn read_stderr(_driver: Arc<Driver>, stderr: tokio::process::ChildStderr) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if !line.trim().is_empty() {
            info!(target: "driver", "{}", line.trim());
        }
    }
}

// 懒启动 + 失效重建。用 tokio::Mutex 是因为 get() 里要 await 启动过程，std 锁不能跨 await 持有
pub struct DriverManager {
    inner: AsyncMutex<Option<Arc<Driver>>>,
    driver_path: PathBuf,
    base: PathBuf,
    options: DriverOptions,
}

impl DriverManager {
    pub fn new(
        driver_path: PathBuf,
        base: PathBuf,
        options: DriverOptions,
    ) -> Arc<DriverManager> {
        Arc::new(DriverManager {
            inner: AsyncMutex::new(None),
            driver_path,
            base,
            options,
        })
    }

    // 存活就复用；进程挂了或还没启动就在这里重建，调用方不用关心
    pub async fn get(&self) -> Result<Arc<Driver>, String> {
        let mut guard = self.inner.lock().await;
        if let Some(driver) = guard.as_ref() {
            if driver.is_alive() {
                return Ok(driver.clone());
            }
        }
        let driver =
            Driver::start(&self.driver_path, &self.base, &self.options).await?;
        *guard = Some(driver.clone());
        Ok(driver)
    }

    pub async fn shutdown(&self) {
        let mut guard = self.inner.lock().await;
        if let Some(driver) = guard.take() {
            driver.shutdown().await;
        }
    }
}
