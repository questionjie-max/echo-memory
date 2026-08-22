// 集成测试：验证命令层用到的纯函数行为（独立于 DB 与 Tauri 运行时）。
use echo_memory_lib::analysis::ollama_base_url;
use echo_memory_lib::state::{heavy_job_limiter, JobLimiter};

#[test]
fn ollama_base_url_is_well_formed_regardless_of_env() {
    // 无论 OLLAMA_HOST 如何设置，解析结果都必须带 scheme 且可拼接 API 路径。
    let base = ollama_base_url();
    assert!(
        base.starts_with("http://") || base.starts_with("https://"),
        "Ollama 地址应带 scheme：{base}"
    );
    assert!(!base.ends_with('/'), "Ollama 地址不应以 / 结尾：{base}");
}

#[test]
fn heavy_job_limiter_capacity_stays_within_bounds() {
    // 全局闸门容量受 ECHO_MAX_HEAVY_JOBS 约束（1–8），避免配置错误导致无并发或无限并发。
    assert!((1..=8).contains(&heavy_job_limiter().max()));
    let single = JobLimiter::new(1);
    let permit = single.acquire();
    let waiter = std::thread::spawn(move || {
        let _permit = single.acquire();
        "acquired"
    });
    assert!(!waiter.is_finished(), "达到上限后新任务应排队等待");
    drop(permit);
    assert_eq!(waiter.join().unwrap(), "acquired");
}
