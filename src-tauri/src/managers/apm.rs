use crate::models::{Package, PlannedCommand, UninstallPlan, UninstallRequest, UninstallResult};
use crate::runner;
use std::collections::HashMap;

/// APM（Amber Package Manager）扫描。
///
/// APM 应用分两类：
/// - 宿主级（com.alibabainc.dingtalk / org.videolan.vlc 等）：已登记进宿主 dpkg，
///   由 apt 管理器负责，这里不再重复。
/// - 容器级（motrix / com.163.music.spark / net.hmcl.huangyuhui / vine-palma-player 等）：
///   只登记在 APM 容器内的 dpkg status，宿主 dpkg 看不到，因此单独扫描。
///
/// 容器级应用的 .desktop 由 amber-pm-desktop-fix 符号链接到
/// /usr/local/share/applications，Exec 形如 `apm run <pkg> <path> %U`，
/// 通过它可以把桌面入口与容器 dpkg 中的安装信息对应起来。
const CONTAINER_STATUS: &str = "/var/lib/apm/apm/files/ace-env/var/lib/dpkg/status";
const CONTAINER_INFO: &str = "/var/lib/apm/apm/files/ace-env/var/lib/dpkg/info";
const HOST_DESKTOP_DIR: &str = "/usr/local/share/applications";

struct ApmMeta {
    version: String,
    size_kb: i64,
    description: String,
}

/// 解析容器 dpkg status，返回"已安装"的包信息。
fn container_installed() -> HashMap<String, ApmMeta> {
    let mut map = HashMap::new();
    let Ok(content) = std::fs::read_to_string(CONTAINER_STATUS) else {
        return map;
    };
    for para in content.split("\n\n") {
        let mut pkg = String::new();
        let mut ok = false;
        let mut version = String::new();
        let mut size_kb: i64 = 0;
        let mut description = String::new();
        for line in para.lines() {
            if let Some(rest) = line.strip_prefix("Package:") {
                pkg = rest.trim().to_string();
            } else if let Some(rest) = line.strip_prefix("Status:") {
                ok = rest.trim().starts_with("install ok installed");
            } else if let Some(rest) = line.strip_prefix("Version:") {
                version = rest.trim().to_string();
            } else if let Some(rest) = line.strip_prefix("Installed-Size:") {
                size_kb = rest.trim().parse().unwrap_or(0);
            } else if let Some(rest) = line.strip_prefix("Description:") {
                description = rest.trim().to_string();
            }
        }
        if ok && !pkg.is_empty() {
            map.insert(pkg, ApmMeta { version, size_kb, description });
        }
    }
    map
}

/// 从 .desktop 内容中提取 (Name, Exec, Icon)。Exec 为空则返回 None。
fn parse_desktop(content: &str) -> Option<(String, String, String)> {
    let mut name = String::new();
    let mut exec = String::new();
    let mut icon = String::new();
    for line in content.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Name=") {
            if name.is_empty() {
                name = rest.trim().to_string();
            }
        } else if let Some(rest) = line.strip_prefix("Exec=") {
            exec = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("Icon=") {
            icon = rest.trim().to_string();
        }
    }
    if exec.is_empty() {
        None
    } else {
        Some((name, exec, icon))
    }
}

/// 从 `Exec=apm run <pkg> <path>...` 中提取包名。
fn apm_pkg_from_exec(exec: &str) -> Option<String> {
    let mut parts = exec.split_whitespace();
    if parts.next()? != "apm" {
        return None;
    }
    if parts.next()? != "run" {
        return None;
    }
    parts.next().map(|s| s.to_string())
}

/// 安装时间：用容器内 dpkg info 的 .list 文件 mtime 近似。
fn install_time(pkg: &str) -> i64 {
    std::fs::metadata(format!("{}/{}.list", CONTAINER_INFO, pkg))
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn size_kb_to_str(kb: i64) -> String {
    let n = kb as f64;
    if n >= 1048576.0 {
        format!("{:.1} GB", n / 1048576.0)
    } else if n >= 1024.0 {
        format!("{:.1} MB", n / 1024.0)
    } else {
        format!("{:.0} KB", n)
    }
}

/// 扫描容器级 APM 应用（有 .desktop 启动入口的）。
pub async fn list_installed() -> Result<Vec<Package>, String> {
    let installed = container_installed();
    let mut pkgs = Vec::new();
    let mut seen = std::collections::HashSet::new();

    if let Ok(entries) = std::fs::read_dir(HOST_DESKTOP_DIR) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().map(|x| x.to_string_lossy().to_string()) != Some("desktop".into()) {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&p) else {
                continue;
            };
            let Some((name, exec, _icon)) = parse_desktop(&content) else {
                continue;
            };
            let Some(pkg_name) = apm_pkg_from_exec(&exec) else {
                continue;
            };
            // 只处理容器内安装的包；宿主级由 apt 负责
            let Some(meta) = installed.get(&pkg_name) else {
                continue;
            };
            if !seen.insert(pkg_name.clone()) {
                continue;
            }
            let size_bytes = meta.size_kb * 1024;
            pkgs.push(Package {
                id: format!("apm:{}", pkg_name),
                name: pkg_name.clone(),
                display_name: if name.is_empty() { pkg_name.clone() } else { name },
                version: meta.version.clone(),
                source: "apm".into(),
                description: if meta.description.is_empty() {
                    format!("APM 应用：{}", pkg_name)
                } else {
                    meta.description.clone()
                },
                size: size_kb_to_str(meta.size_kb),
                size_bytes,
                installed_at: install_time(&pkg_name),
                essential: false,
                risk: String::new(),
                gui: true,
            });
        }
    }
    pkgs.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    Ok(pkgs)
}

pub fn build_plan(pkg: &Package) -> UninstallPlan {
    let commands = vec![PlannedCommand {
        description: format!("卸载 {}（apm remove，含容器内数据）", pkg.display_name),
        command: format!("apm remove -y --purge {}", pkg.name),
        needs_root: true,
        affects_others: false,
    }];

    UninstallPlan {
        package: pkg.clone(),
        commands,
        files: Vec::new(),
        warnings: vec![
            "此软件通过 APM（Amber Package Manager）安装，卸载会移除其桌面启动项与容器内安装的数据。".to_string(),
        ],
    }
}

pub async fn execute(pkg: &Package, _req: &UninstallRequest, result: &mut UninstallResult) {
    let r = runner::run("apm", &["remove", "-y", "--purge", &pkg.name], true, 600).await;
    match r {
        Ok(res) if res.success => {
            result.executed.push(format!("已卸载 {}", pkg.display_name));
        }
        Ok(res) => {
            result.success = false;
            result
                .failed
                .push(format!("apm remove {} 失败: {}", pkg.name, res.stderr.trim().lines().last().unwrap_or("")));
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

    #[tokio::test]
    async fn apm_list_finds_container_apps() {
        if !std::path::Path::new(CONTAINER_STATUS).exists() {
            eprintln!("系统未安装 APM 容器，跳过测试");
            return;
        }
        let pkgs = list_installed().await.unwrap();
        let names: Vec<&str> = pkgs.iter().map(|p| p.name.as_str()).collect();
        eprintln!("APM 应用: {:?}", names);
        assert!(
            names.iter().any(|n| n.contains("motrix") || n.contains("spark") || n.contains("vine") || n.contains("hmcl")),
            "应能扫描到 APM 容器应用: {:?}",
            names
        );
    }

    #[test]
    fn exec_parsing() {
        assert_eq!(apm_pkg_from_exec("apm run motrix /opt/Motrix/motrix %U"), Some("motrix".into()));
        assert_eq!(
            apm_pkg_from_exec("apm run com.163.music.spark /opt/apps/com.163.music.spark/files/start.sh"),
            Some("com.163.music.spark".into())
        );
        assert_eq!(apm_pkg_from_exec("/usr/bin/foo %U"), None);
    }
}
