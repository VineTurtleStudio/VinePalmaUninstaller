use crate::models::{Package, PlannedCommand, UninstallPlan, UninstallRequest, UninstallResult};
use crate::runner;
use std::path::Path;

const APPIMAGE_HINT_DIRS: [&str; 4] = ["~/Applications", "/opt", "/usr/local/bin", "~/bin"];

/// 扫描手动安装的软件：AppImage 文件 + 指向它们的 .desktop 文件。
/// 手动安装的软件没有包管理器登记，只能通过桌面入口与可执行文件识别。
pub async fn list_installed() -> Result<Vec<Package>, String> {
    let mut found: Vec<(String, String, String)> = Vec::new(); // (exec_path, display_name, desktop_file)

    // 1. 扫描常见目录中的 AppImage 文件
    for d in APPIMAGE_HINT_DIRS {
        let path = expand_tilde(d);
        if let Ok(entries) = std::fs::read_dir(&path) {
            for e in entries.flatten() {
                let p = e.path();
                if let Some(ext) = p.extension() {
                    if ext.to_string_lossy().eq_ignore_ascii_case("appimage") {
                        let name = p
                            .file_stem()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_else(|| p.to_string_lossy().to_string());
                        found.push((p.to_string_lossy().to_string(), name, String::new()));
                    }
                }
            }
        }
    }

    // 2. 扫描 .desktop 文件，提取 Exec 指向手动安装程序的
    let desktop_dirs = [
        expand_tilde("~/.local/share/applications"),
        "/usr/local/share/applications".to_string(),
    ];
    for dir in desktop_dirs {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.extension().map(|x| x.to_string_lossy()) != Some("desktop".into()) {
                    continue;
                }
                if let Some((name, exec, _icon)) = parse_desktop(&p) {
                    let exec = exec.trim_matches('"');
                    // 只处理手动安装目标（AppImage 或常见手动安装目录）
                    if is_manual_exec(&exec) {
                        found.push((exec.to_string(), name, p.to_string_lossy().to_string()));
                    }
                }
            }
        }
    }

    // 3. 汇总去重（按 exec 路径）
    let mut seen = std::collections::HashSet::new();
    let mut pkgs = Vec::new();
    for (exec, name, desktop) in found {
        if !seen.insert(exec.clone()) {
            continue;
        }
        let display_name = Path::new(&exec)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| name.clone());
        let meta = std::fs::metadata(&exec).ok();
        let mtime = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let size_bytes = meta.map(|m| m.len() as i64).unwrap_or(0);
        pkgs.push(Package {
            id: format!("manual:{}", exec),
            name: exec.clone(),
            display_name: display_name.clone(),
            version: "手动安装".into(),
            source: "appimage".into(),
            description: format!("AppImage/手动安装程序：{}", exec),
            size: file_size_str(&exec),
            size_bytes,
            installed_at: mtime,
            essential: false,
            risk: String::new(),
            gui: true,
        });
        let _ = desktop;
    }
    Ok(pkgs)
}

fn is_manual_exec(exec: &str) -> bool {
    if exec.to_lowercase().ends_with(".appimage") {
        return true;
    }
    let expanded = expand_tilde("~");
    let candidates = ["/opt/", &format!("{}/Applications/", expanded), &format!("{}/bin/", expanded), "/usr/local/bin/"];
    candidates.iter().any(|c| exec.starts_with(c))
}

fn parse_desktop(path: &Path) -> Option<(String, String, String)> {
    let content = std::fs::read_to_string(path).ok()?;
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
    if name.is_empty() || exec.is_empty() {
        return None;
    }
    Some((name, exec, icon))
}

fn expand_tilde(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        format!("{}/{}", home(), rest)
    } else {
        p.to_string()
    }
}

fn home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/root".to_string())
}

fn file_size_str(path: &str) -> String {
    std::fs::metadata(path)
        .ok()
        .map(|m| {
            let n = m.len() as f64;
            if n >= 1024.0 * 1024.0 * 1024.0 {
                format!("{:.1} GB", n / 1024.0 / 1024.0 / 1024.0)
            } else if n >= 1024.0 * 1024.0 {
                format!("{:.1} MB", n / 1024.0 / 1024.0)
            } else {
                format!("{:.0} KB", n / 1024.0)
            }
        })
        .unwrap_or_default()
}

pub fn build_plan(pkg: &Package) -> UninstallPlan {
    let desktop_files = find_desktop_files(&pkg.name);
    let mut commands = Vec::new();
    let mut files = Vec::new();

    let exec_exists = std::path::Path::new(&pkg.name).exists();
    files.push(crate::models::PlannedFile {
        path: pkg.name.clone(),
        kind: if exec_exists {
            let kind = if std::path::Path::new(&pkg.name).is_dir() { "dir" } else { "file" };
            kind.to_string()
        } else {
            "file".into()
        },
        category: "appimage".into(),
        exists: exec_exists,
        selected: true,
    });

    for df in &desktop_files {
        files.push(crate::models::PlannedFile {
            path: df.clone(),
            kind: "file".into(),
            category: "desktop".into(),
            exists: true,
            selected: true,
        });
    }

    if !desktop_files.is_empty() {
        commands.push(PlannedCommand {
            description: "删除对应的 .desktop 启动项".into(),
            command: format!("rm {}", desktop_files.join(" ")),
            needs_root: desktop_files.iter().any(|f| f.starts_with("/usr") || f.starts_with("/opt")),
            affects_others: false,
        });
    }
    commands.push(PlannedCommand {
        description: format!("删除 AppImage 文件 {}", Path::new(&pkg.name).file_name().unwrap_or_default().to_string_lossy()),
        command: format!("rm \"{}\"", pkg.name),
        needs_root: pkg.name.starts_with("/opt") || pkg.name.starts_with("/usr"),
        affects_others: false,
    });

    UninstallPlan {
        package: pkg.clone(),
        commands,
        files,
        warnings: Vec::new(),
    }
}

fn find_desktop_files(exec_path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let dirs = [
        expand_tilde("~/.local/share/applications"),
        "/usr/local/share/applications".to_string(),
        "/usr/share/applications".to_string(),
    ];
    for dir in dirs {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let p = e.path();
                if let Ok(content) = std::fs::read_to_string(&p) {
                    if content.lines().any(|l| l.trim().starts_with("Exec=") && l.contains(exec_path)) {
                        out.push(p.to_string_lossy().to_string());
                    }
                }
            }
        }
    }
    out
}

pub async fn execute(pkg: &Package, req: &UninstallRequest, result: &mut UninstallResult) {
    // 只删除用户在计划中勾选的路径；未勾选的一律保留（与界面提示一致）
    let want = |p: &str| req.selected_files.iter().any(|f| f == p);

    // 删除 .desktop 启动项
    let desktop_files = find_desktop_files(&pkg.name);
    for df in &desktop_files {
        if !want(df) {
            continue;
        }
        let needs_root = df.starts_with("/usr") || df.starts_with("/opt");
        let r = runner::run("rm", &["-f", df], needs_root, 30).await;
        match r {
            Ok(res) if res.success => result.executed.push(format!("已删除 {}", df)),
            Ok(res) => result.failed.push(format!("删除 {} 失败: {}", df, res.stderr.trim())),
            Err(e) => result.failed.push(e),
        }
    }
    // 删除 AppImage 文件
    if want(&pkg.name) {
        let needs_root = pkg.name.starts_with("/opt") || pkg.name.starts_with("/usr");
        let r = runner::run("rm", &["-f", &pkg.name], needs_root, 60).await;
        match r {
            Ok(res) if res.success => {
                result.executed.push(format!("已删除 {}", pkg.name));
            }
            Ok(res) => {
                result.success = false;
                result.failed.push(format!("删除 {} 失败: {}", pkg.name, res.stderr.trim()));
            }
            Err(e) => {
                result.success = false;
                result.failed.push(e);
            }
        }
    }
}
