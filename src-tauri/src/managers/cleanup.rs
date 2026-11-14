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

fn push_variant(v: &mut Vec<String>, s: &str) {
    let t = s.trim();
    if !t.is_empty() && !v.iter().any(|x| x == t) {
        v.push(t.to_string());
    }
}

/// 生成用于"宽松匹配"的包名变体：
/// 包名 / 显示名 / 文件名 / 去扩展名 / 连字符-下划线等价 / 大小写不敏感。
/// 用于识别为避免默认路径冲突而“单独存放”的软件目录（如 `~/.config/VinePalma` 对包名 `vine-palma`）。
fn name_variants(pkg: &Package) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    push_variant(&mut v, &pkg.name);
    push_variant(&mut v, &pkg.display_name);
    if let Some(f) = std::path::Path::new(&pkg.name).file_name() {
        let f = f.to_string_lossy();
        push_variant(&mut v, &f);
        if let Some(st) = std::path::Path::new(f.as_ref()).file_stem() {
            push_variant(&mut v, &st.to_string_lossy());
        }
    }
    // 核心变体再派生：lowercase + 连字符/下划线互换，让大小写/分隔符不一致也能命中
    let base: Vec<String> = v.clone();
    for s in &base {
        if s.chars().any(|c| c.is_uppercase()) {
            push_variant(&mut v, &s.to_lowercase());
        }
        push_variant(&mut v, &s.replace('-', "_"));
        push_variant(&mut v, &s.replace('_', "-"));
    }
    v
}

/// 跳过已在列表中的路径，避免重复候选。
fn append_unique(list: &mut Vec<PlannedFile>, extra: Vec<PlannedFile>) {
    let existing: std::collections::HashSet<String> =
        list.iter().map(|f| f.path.clone()).collect();
    for f in extra {
        if !existing.contains(&f.path) {
            list.push(f);
        }
    }
}

/// 判断一个目录/文件名与包名变体集合的关系，返回 (是否精确匹配, 是否仅包含匹配)。
/// 纯函数，便于单元测试。
fn name_relation(fname: &str, v_lower: &[String]) -> (bool, bool) {
    let lower = fname.to_lowercase();
    let exact = v_lower.iter().any(|n| *n == lower);
    // “包含”匹配：至少 3 个字符才参与，避免过短的泛化名误报
    let fuzzy = !exact
        && fname.chars().count() >= 3
        && v_lower.iter().any(|n| !n.is_empty() && (n.contains(&lower) || lower.contains(n)));
    (exact, fuzzy)
}

/// 扫描“为避免系统默认路径冲突而单独存放”的软件目录，作为残留清理候选。
///
/// 覆盖常见独立存放位置：`~/.config`、`~/.cache`、`~/.local/share`、`~/.local/state`、
/// `~/Applications`（AppImage 常见目录）、`/opt`。
///
/// 严格程度：
/// - 目录名与包名变体“大小写不敏感精确”匹配 → 认定为该软件自己的目录，默认勾选；
/// - 仅是“包含”关系（如包名 `google-chrome-stable` 命中目录 `google-chrome`）→
///   追加为候选但**默认不勾选**，绝不误删无关软件。
pub fn scan_isolated_dirs(pkg: &Package) -> Vec<PlannedFile> {
    let h = home();
    let mut out: Vec<PlannedFile> = Vec::new();
    let variants = name_variants(pkg);
    let v_lower: Vec<String> = variants.iter().map(|s| s.to_lowercase()).collect();

    let bases: Vec<(String, &str)> = vec![
        (format!("{}/.config", h), "config"),
        (format!("{}/.cache", h), "cache"),
        (format!("{}/.local/share", h), "data"),
        (format!("{}/.local/state", h), "config"),
        (format!("{}/Applications", h), "appimage"),
        ("/opt".to_string(), "system"),
    ];

    for (base, cat) in bases {
        let Ok(entries) = std::fs::read_dir(&base) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            let fname = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if fname.is_empty() {
                continue;
            }
            let (exact, fuzzy) = name_relation(&fname, &v_lower);
            if !exact && !fuzzy {
                continue;
            }
            let path = p.to_string_lossy().to_string();
            if !path_is_safe(&path) {
                continue;
            }
            let kind = if is_dir(&path) { "dir" } else { "file" }.to_string();
            out.push(PlannedFile {
                path,
                kind,
                category: cat.to_string(),
                exists: true,
                selected: exact,
            });
        }
    }
    out
}

/// 为 APT 包生成残留候选。先做包名精确匹配，再补“单独存放”目录的识别。
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
    append_unique(&mut candidates, scan_isolated_dirs(pkg));
    candidates
}

/// 为 Flatpak 应用生成残留候选：应用数据目录。
pub fn flatpak_residue(pkg: &Package) -> Vec<PlannedFile> {
    let h = home();
    let mut out = Vec::new();
    // 应用数据目录（--user 安装）与系统安装目录
    for path in [
        format!("{}/.var/app/{}", h, pkg.name),
        format!("/var/lib/flatpak/app/{}", pkg.name),
    ] {
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
    }
    append_unique(&mut out, scan_isolated_dirs(pkg));
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
    append_unique(&mut out, scan_isolated_dirs(pkg));
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
    append_unique(&mut out, scan_isolated_dirs(pkg));
    out
}

/// 为 APM 容器应用生成残留候选：容器内应用目录 + 桌面启动项 + 用户数据目录。
pub fn apm_residue(pkg: &Package) -> Vec<PlannedFile> {
    let mut out = manual_residue(pkg);

    // 桌面启动项：Exec=apm run <pkg> 对应的 .desktop 文件
    let h = home();
    for dir in [
        format!("{}/.local/share/applications", h),
        "/usr/local/share/applications".to_string(),
    ] {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.extension().map(|x| x.to_string_lossy().to_string()) != Some("desktop".into()) {
                    continue;
                }
                let Ok(content) = std::fs::read_to_string(&p) else { continue };
                let is_target = content.lines().any(|l| {
                    let t = l.trim();
                    t.starts_with("Exec=")
                        && t.split_whitespace().nth(1) == Some("run")
                        && t.split_whitespace().nth(2) == Some(&pkg.name)
                });
                if !is_target {
                    continue;
                }
                let path = p.to_string_lossy().to_string();
                if path_is_safe(&path) {
                    out.push(PlannedFile {
                        path,
                        kind: "file".into(),
                        category: "desktop".into(),
                        exists: true,
                        selected: true,
                    });
                }
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Package;

    fn pkg(name: &str, display: &str) -> Package {
        Package {
            id: format!("apt:{}", name),
            name: name.to_string(),
            display_name: display.to_string(),
            version: String::new(),
            source: "apt".into(),
            description: String::new(),
            size: String::new(),
            size_bytes: 0,
            installed_at: 0,
            essential: false,
            risk: String::new(),
            gui: true,
        }
    }

    #[test]
    fn name_variants_cover_separator_and_case() {
        let variants = name_variants(&pkg("vine-palma-player", "vine-palma-player"));
        assert!(variants.contains(&"vine-palma-player".to_string()));
        assert!(variants.contains(&"vine_palma_player".to_string()));
    }

    #[test]
    fn name_variants_appimage_path() {
        // 手动安装的 AppImage：name 是完整路径，display_name 是文件名
        let p = Package {
            id: "manual:/home/u/Applications/FooBar.AppImage".into(),
            name: "/home/u/Applications/FooBar.AppImage".into(),
            display_name: "FooBar.AppImage".into(),
            version: String::new(),
            source: "appimage".into(),
            description: String::new(),
            size: String::new(),
            size_bytes: 0,
            installed_at: 0,
            essential: false,
            risk: String::new(),
            gui: true,
        };
        let variants = name_variants(&p);
        assert!(variants.contains(&"FooBar".to_string()));
        assert!(variants.contains(&"FooBar.AppImage".to_string()));
        assert!(variants.contains(&"foobar".to_string()));
    }

    #[test]
    fn name_relation_exact_vs_fuzzy() {
        let v = vec!["google-chrome-stable".to_string(), "vine-palma-player".to_string()];
        // 精确（忽略大小写）
        assert_eq!(name_relation("Google-Chrome-Stable", &v), (true, false));
        // 仅包含
        assert_eq!(name_relation("google-chrome", &v), (false, true));
        // 无关目录
        assert_eq!(name_relation("bash", &v), (false, false));
    }
}
