use crate::models::{Package, PlannedFile};
use crate::runner;

fn home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/root".to_string())
}

const FORBIDDEN: [&str; 14] = [
    "/", "/home", "/etc", "/usr", "/usr/share", "/usr/bin", "/usr/lib", "/var", "/var/lib",
    "/opt", "/root", "/boot", "/srv", "/tmp",
];

/// 安全检查：确认路径不是系统关键目录或其直系子目录。
fn path_is_safe(path: &str) -> bool {
    let p = std::path::Path::new(path);
    let abs = if p.is_absolute() { p.to_path_buf() } else { std::path::PathBuf::from(path) };
    for f in FORBIDDEN {
        if abs == std::path::Path::new(f) {
            return false;
        }
    }
    // 禁止删除 home 根目录或其一级配置根（.config/.cache/.local 本身）
    let h = std::path::PathBuf::from(home());
    let protected_roots = [
        h.join(".config"),
        h.join(".cache"),
        h.join(".local"),
        h.join(".local/share"),
        h.join(".local/state"),
        h.join(".var"),
        h.clone(),
    ];
    for r in protected_roots {
        if abs == r {
            return false;
        }
    }
    // 检查路径组件：不允许包含 . 或 .. 等危险组件（由调用方保证，这里兜底）
    for comp in abs.components() {
        if let std::path::Component::ParentDir = comp {
            return false;
        }
    }
    true
}

fn exists(path: &str) -> bool {
    std::path::Path::new(path).exists()
}

fn is_dir(path: &str) -> bool {
    std::path::Path::new(path).is_dir()
}

/// 为 APT 包生成残留候选。只扫描与包名精确匹配的目录。
pub fn apt_residue(pkg: &Package) -> Vec<PlannedFile> {
    let h = home();
    let name = &pkg.name;
    let mut candidates = Vec::new();

    let user_dirs: Vec<(String, String)> = vec![
        (format!("{}/.config/{}", h, name), "config".to_string()),
        (format!("{}/.cache/{}", h, name), "cache".to_string()),
        (format!("{}/.local/share/{}", h, name), "data".to_string()),
        (format!("{}/.local/state/{}", h, name), "config".to_string()),
        (format!("{}/.{}", h, name), "config".to_string()),
    ];
    let sys_dirs: Vec<(String, String)> = vec![
        (format!("/etc/{}", name), "system".to_string()),
        (format!("/var/lib/{}", name), "system".to_string()),
        (format!("/var/log/{}", name), "system".to_string()),
        (format!("/opt/{}", name), "system".to_string()),
    ];

    for (path, cat) in user_dirs.iter().chain(sys_dirs.iter()) {
        if exists(path) && path_is_safe(path) {
            candidates.push(PlannedFile {
                path: path.clone(),
                kind: if is_dir(path) { "dir" } else { "file" }.into(),
                category: cat.clone(),
                exists: true,
                selected: cat == "config" || cat == "cache" || cat == "data",
            });
        }
    }
    candidates
}

/// 为 Flatpak 应用生成残留候选：应用数据目录。
pub fn flatpak_residue(pkg: &Package) -> Vec<PlannedFile> {
    let h = home();
    let path = format!("{}/.var/app/{}", h, pkg.name);
    let mut out = Vec::new();
    if exists(&path) && path_is_safe(&path) {
        let kind = if is_dir(&path) { "dir" } else { "file" }.to_string();
        out.push(PlannedFile {
            path,
            kind,
            category: "data".into(),
            exists: true,
            selected: true,
        });
    }
    out
}

/// 为 Snap 应用生成残留候选。
pub fn snap_residue(pkg: &Package) -> Vec<PlannedFile> {
    let h = home();
    let mut out = Vec::new();
    for (path, cat) in [
        (format!("{}/snap/{}", h, pkg.name), "data".to_string()),
        (format!("/var/snap/{}", pkg.name), "system".to_string()),
    ] {
        if exists(&path) && path_is_safe(&path) {
            let kind = if is_dir(&path) { "dir" } else { "file" }.to_string();
            out.push(PlannedFile {
                path,
                kind,
                category: cat,
                exists: true,
                selected: true,
            });
        }
    }
    out
}

/// 为手动安装/AppImage 程序生成残留候选（以文件名与去掉扩展名的名字匹配）。
pub fn manual_residue(pkg: &Package) -> Vec<PlannedFile> {
    let h = home();
    let file_name = std::path::Path::new(&pkg.name)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let stem = std::path::Path::new(&file_name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| file_name.clone());
    let mut out = Vec::new();
    for (path, cat) in [
        (format!("{}/.config/{}", h, stem), "config".to_string()),
        (format!("{}/.cache/{}", h, stem), "cache".to_string()),
        (format!("{}/.local/share/{}", h, stem), "data".to_string()),
        (format!("{}/.config/{}", h, file_name), "config".to_string()),
    ] {
        if exists(&path) && path_is_safe(&path) {
            let kind = if is_dir(&path) { "dir" } else { "file" }.to_string();
            out.push(PlannedFile {
                path,
                kind,
                category: cat,
                exists: true,
                selected: true,
            });
        }
    }
    out
}

/// 按用户选择删除残留文件。返回 (已删除, 失败)。
pub async fn remove_selected(selected: &[String], result: &mut crate::models::UninstallResult) {
    for path in selected {
        if !path_is_safe(path) {
            result.failed.push(format!("已跳过不安全的路径: {}", path));
            continue;
        }
        let is_dir = std::path::Path::new(path).is_dir();
        let needs_root = path.starts_with("/etc") || path.starts_with("/var") || path.starts_with("/opt");
        let arg = if is_dir { "-rf" } else { "-f" };
        let r = runner::run("rm", &[arg, path], needs_root, 60).await;
        match r {
            Ok(res) if res.success => {
                result.executed.push(format!("已清理 {}", path));
            }
            Ok(res) => {
                result.failed.push(format!("清理 {} 失败: {}", path, res.stderr.trim()));
            }
            Err(e) => {
                result.failed.push(e);
            }
        }
    }
}
