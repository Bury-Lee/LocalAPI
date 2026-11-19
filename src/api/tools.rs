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
