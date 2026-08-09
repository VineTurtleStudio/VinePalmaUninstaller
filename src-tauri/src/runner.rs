use std::process::Stdio;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

#[derive(Debug, Clone)]
pub struct CmdResult {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

pub fn is_root() -> bool {
    // 通过读取进程 uid 判断是否以 root 运行
    std::fs::read_to_string("/proc/self/status")
        .map(|s| s.lines().any(|l| l.starts_with("Uid:") && l.split_whitespace().nth(1) == Some("0")))
        .unwrap_or(false)
}

/// 执行命令。root=true 时自动用 pkexec 提权（若当前非 root）。
pub async fn run(cmd: &str, args: &[&str], root: bool, timeout_secs: u64) -> Result<CmdResult, String> {
    let need_priv = root && !is_root();

    let mut command = Command::new(if need_priv { "pkexec" } else { cmd });
    if need_priv {
        command.arg(cmd);
        for a in args {
            command.arg(a);
        }
    } else {
        for a in args {
            command.arg(a);
        }
    }

    let result = timeout(
        Duration::from_secs(timeout_secs),
        command
            .env("LC_ALL", "C")
            .env("LANG", "C")
            .stdin(Stdio::null())
            .output(),
    )
    .await
    .map_err(|_| format!("命令执行超时: {} {}", cmd, args.join(" ")))?
    .map_err(|e| format!("无法启动命令 {}: {}", cmd, e))?;

    Ok(CmdResult {
        success: result.status.success(),
        stdout: String::from_utf8_lossy(&result.stdout).to_string(),
        stderr: String::from_utf8_lossy(&result.stderr).to_string(),
    })
}
