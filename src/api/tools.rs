use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

static RequestSeq: AtomicU64 = AtomicU64::new(0);

/// 生成短请求 id，形如 `req-1794841872-7`；日志与响应头 `x-request-id` 共用。
pub fn next_request_id() -> String {
    let seq = RequestSeq.fetch_add(1, Ordering::Relaxed) + 1;
    format!("req-{}-{}", now_secs(), seq)
}

static HistorySeq: AtomicU64 = AtomicU64::new(0);

/// 会话历史 id：毫秒时间戳 + 进程内序号，按时间递增且同一毫秒内不冲突。
pub fn next_history_id() -> u64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let seq = HistorySeq.fetch_add(1, Ordering::Relaxed) % 1000;
    millis.saturating_mul(1000).saturating_add(seq)
}
