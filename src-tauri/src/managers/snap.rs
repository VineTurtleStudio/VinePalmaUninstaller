use crate::models::{Package, PlannedCommand, UninstallPlan, UninstallRequest, UninstallResult};
use crate::runner;

pub async fn list_installed() -> Result<Vec<Package>, String> {
    let r = runner::run("snap", &["list", "--unicode=never"], false, 60).await?;
    let mut pkgs = Vec::new();
    for (idx, line) in r.stdout.lines().enumerate() {
        if idx == 0 {
            continue; // 表头
        }
        let mut it = line.split_whitespace();
        let (Some(name), Some(ver)) = (it.next(), it.next()) else {
            continue;
        };
        pkgs.push(Package {
            id: format!("snap:{}", name),
            name: name.to_string(),
            display_name: name.to_string(),
            version: ver.to_string(),
            source: "snap".into(),
            description: "Snap 应用".into(),
            size: String::new(),
            size_bytes: snap_file_size(name).unwrap_or(0),
            installed_at: snap_install_time(name),
            essential: false,
            risk: String::new(),
            gui: true,
        });
    }
    Ok(pkgs)
}

/// 读取 /var/lib/snapd/snaps/<name>_<rev>.snap 的 mtime 与大小。
fn snap_file(name: &str) -> Option<std::path::PathBuf> {
    let dir = "/var/lib/snapd/snaps";
    let entries = std::fs::read_dir(dir).ok()?;
    let mut best: Option<(i64, std::path::PathBuf)> = None;
    for e in entries.flatten() {
        let p = e.path();
        let fname = p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        if !fname.starts_with(&format!("{}_", name)) || !fname.ends_with(".snap") {
            continue;
        }
        if let Ok(m) = std::fs::metadata(&p) {
            let t = m.modified().ok().and_then(|x| x.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0);
            if best.as_ref().map(|(bt, _)| t > *bt).unwrap_or(true) {
                best = Some((t, p.clone()));
            }
        }
    }
    best.map(|(_, p)| p)
}

fn snap_install_time(name: &str) -> i64 {
    snap_file(name)
        .and_then(|p| std::fs::metadata(&p).ok())
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn snap_file_size(name: &str) -> Option<i64> {
    snap_file(name)
        .and_then(|p| std::fs::metadata(&p).ok())
        .map(|m| m.len() as i64)
}

pub fn build_plan(pkg: &Package) -> UninstallPlan {
    UninstallPlan {
        package: pkg.clone(),
        commands: vec![PlannedCommand {
            description: format!("卸载 {}（snap remove，含数据）", pkg.name),
            command: format!("snap remove --purge {}", pkg.name),
            needs_root: true,
            affects_others: false,
        }],
        files: Vec::new(),
        warnings: Vec::new(),
    }
}

pub async fn execute(pkg: &Package, _req: &UninstallRequest, result: &mut UninstallResult) {
    let r = runner::run("snap", &["remove", "--purge", &pkg.name], true, 300).await;
    match r {
        Ok(res) if res.success => {
            result.executed.push(format!("已卸载 {}", pkg.name));
        }
        Ok(res) => {
            result.success = false;
            result
                .failed
                .push(format!("snap remove {} 失败: {}", pkg.name, res.stderr.trim().lines().last().unwrap_or("")));
        }
        Err(e) => {
            result.success = false;
            result.failed.push(e);
        }
    }
}
