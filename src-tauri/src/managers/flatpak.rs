use crate::models::{Package, PlannedCommand, UninstallPlan, UninstallRequest, UninstallResult};
use crate::runner;

pub async fn list_installed() -> Result<Vec<Package>, String> {
    let r = runner::run(
        "flatpak",
        &["list", "--app", "--columns=application,name,version,size", "--noninteractive"],
        false,
        60,
    )
    .await?;
    let mut pkgs = Vec::new();
    for line in r.stdout.lines() {
        let mut parts = line.split('\t');
        let (Some(appid), Some(name), version, size) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let appid = appid.trim();
        if appid.is_empty() {
            continue;
        }
        pkgs.push(Package {
            id: format!("flatpak:{}", appid),
            name: appid.to_string(),
            display_name: name.trim().to_string(),
            version: version.unwrap_or("").trim().to_string(),
            source: "flatpak".into(),
            description: appid.to_string(),
            size: size.unwrap_or("").trim().to_string(),
            size_bytes: parse_size(size),
            installed_at: app_dir_time(appid),
            essential: false,
            risk: String::new(),
            gui: true,
        });
    }
    pkgs.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(pkgs)
}

/// flatpak 安装目录 mtime 作为安装时间近似。
fn app_dir_time(appid: &str) -> i64 {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    for dir in [
        format!("/var/lib/flatpak/app/{}", appid),
        format!("{}/.local/share/flatpak/app/{}", home, appid),
    ] {
        if let Ok(m) = std::fs::metadata(&dir) {
            if let Ok(t) = m.modified() {
                if let Ok(d) = t.duration_since(std::time::UNIX_EPOCH) {
                    return d.as_secs() as i64;
                }
            }
        }
    }
    0
}

/// flatpak 的 size 列是 "1,2 MB" 这类带千位分隔符的字符串。
fn parse_size(size: Option<&str>) -> i64 {
    let Some(s) = size else { return 0 };
    let s = s.trim().replace(',', "");
    let s = s.replace(',', "");
    let lower = s.to_lowercase();
    let (num, mult) = if lower.ends_with("kb") {
        (&s[..s.len() - 2], 1024f64)
    } else if lower.ends_with("mb") {
        (&s[..s.len() - 2], 1024f64 * 1024.0)
    } else if lower.ends_with("gb") {
        (&s[..s.len() - 2], 1024f64 * 1024.0 * 1024.0)
    } else {
        (s.trim(), 1.0)
    };
    let n: f64 = num.trim().parse().unwrap_or(0.0);
    (n * mult) as i64
}

pub fn build_plan(pkg: &Package) -> UninstallPlan {
    let commands = vec![PlannedCommand {
        description: format!("卸载 {}（flatpak）", pkg.display_name),
        command: format!("flatpak uninstall -y {}", pkg.name),
        needs_root: false,
        affects_others: false,
    }];

    UninstallPlan {
        package: pkg.clone(),
        commands,
        files: Vec::new(),
        warnings: Vec::new(),
    }
}

pub async fn execute(pkg: &Package, _req: &UninstallRequest, result: &mut UninstallResult) {
    let r = runner::run("flatpak", &["uninstall", "-y", "--noninteractive", &pkg.name], false, 300).await;
    match r {
        Ok(res) if res.success => {
            result.executed.push(format!("已卸载 {}", pkg.display_name));
        }
        Ok(res) => {
            result.success = false;
            result.failed.push(format!(
                "flatpak uninstall {} 失败: {}",
                pkg.name,
                res.stderr.trim().lines().last().unwrap_or("")
            ));
        }
        Err(e) => {
            result.success = false;
            result.failed.push(e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_flatpak_size() {
        assert_eq!(parse_size(Some("1,234 MB")), (1234.0 * 1024.0 * 1024.0) as i64);
        assert_eq!(parse_size(Some("10 MB")), (10.0 * 1024.0 * 1024.0) as i64);
        assert_eq!(parse_size(Some("—")), 0);
    }
}
