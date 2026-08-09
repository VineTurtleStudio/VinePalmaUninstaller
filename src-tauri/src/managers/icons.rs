use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::OnceLock;

static ICON_CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();

fn cache() -> &'static Mutex<HashMap<String, Option<String>>> {
    ICON_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 清空图标缓存（重装/刷新后调用，避免返回旧包名的过期图标结果）。
pub fn clear_cache() {
    if let Ok(mut guard) = cache().lock() {
        guard.clear();
    }
}

/// 全局 desktop 索引：Exec 可执行名 → 图标名。一次性构建并缓存（返回克隆，外层有包名缓存所以调用不频繁）。
fn desktop_index() -> HashMap<String, String> {
    static IDX: OnceLock<Mutex<Option<HashMap<String, String>>>> = OnceLock::new();
    let mut guard = IDX.get_or_init(|| Mutex::new(None)).lock().unwrap();
    if let Some(idx) = guard.as_ref() {
        return idx.clone();
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let dirs = [
        format!("{}/.local/share/applications", home),
        "/usr/local/share/applications".to_string(),
        "/usr/share/applications".to_string(),
        "/var/lib/flatpak/exports/share/applications".to_string(),
        "/var/lib/snapd/desktop/applications".to_string(),
    ];
    let mut idx = HashMap::new();
    for dir in dirs {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.extension().map(|x| x.to_string_lossy().to_string()) != Some("desktop".into()) {
                    continue;
                }
                let content = match std::fs::read_to_string(&p) {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                let mut icon = None;
                let mut exec = None;
                for line in content.lines() {
                    let line = line.trim();
                    if let Some(r) = line.strip_prefix("Exec=") {
                        exec = r.split_whitespace().next().map(|s| s.to_string());
                    } else if let Some(r) = line.strip_prefix("Icon=") {
                        icon = Some(r.trim().trim_matches('"').to_string());
                    }
                }
                if let (Some(exec), Some(icon)) = (exec, icon) {
                    if !exec.is_empty() && !icon.is_empty() {
                        let exec_base = std::path::Path::new(&exec)
                            .file_name()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_default();
                        if !exec_base.is_empty() && !idx.contains_key(&exec_base) {
                            idx.insert(exec_base, icon);
                        }
                    }
                }
            }
        }
    }
    *guard = Some(idx.clone());
    idx
}

/// 判断某个可执行名是否有对应的 .desktop 入口（用于 GUI 判定）。
pub fn exec_has_desktop(exec_base: &str) -> bool {
    let idx = desktop_index();
    idx.contains_key(exec_base)
        || idx.contains_key(&exec_base.replace('-', "_"))
        || idx.contains_key(&exec_base.replace('_', "-"))
}

/// 从 .desktop 文件中提取图标（绝对路径或图标名）。
fn desktop_icon(desktop_file: &str) -> Option<String> {
    let content = std::fs::read_to_string(desktop_file).ok()?;
    for line in content.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Icon=") {
            let icon = rest.trim().trim_matches('"');
            if !icon.is_empty() {
                return Some(icon.to_string());
            }
        }
    }
    None
}

/// 在图标主题目录中查找图标名对应的文件路径。
fn find_theme_icon(icon: &str) -> Option<String> {
    if icon.contains('/') {
        let p = std::path::Path::new(icon);
        if p.exists() {
            return Some(icon.to_string());
        }
        return None;
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let base_dirs = [
        format!("{}/.local/share/icons/hicolor", home),
        format!("{}/.local/share/icons", home),
        format!("{}/.icons/hicolor", home),
        "/usr/share/icons/hicolor".to_string(),
        "/usr/share/icons/Adwaita".to_string(),
        "/usr/share/icons/Papirus".to_string(),
        "/usr/share/icons/breeze".to_string(),
        "/usr/share/icons/ubuntu-mono-dark".to_string(),
        "/usr/share/icons/ubuntu-mono-light".to_string(),
        "/usr/share/pixmaps".to_string(),
    ];
    let sizes = ["256x256", "scalable", "128x128", "64x64", "48x48", "32x32", "24x24", "22x22", "16x16"];
    let subdirs = ["apps", "mimetypes", "places", "status"];
    let exts = ["png", "svg"];

    for base in &base_dirs {
        for size in &sizes {
            for sub in &subdirs {
                for extn in &exts {
                    let p = format!("{}/{}/{}/{}.{}", base, size, sub, icon, extn);
                    if std::path::Path::new(&p).exists() {
                        return Some(p);
                    }
                }
            }
        }
        // 兜底：base/icon.png 与 base/apps/icon.png、base/icon.svg
        for ext in &exts {
            let p1 = format!("{}/{}.{}", base, icon, ext);
            if std::path::Path::new(&p1).exists() {
                return Some(p1);
            }
            let p2 = format!("{}/apps/{}.{}", base, icon, ext);
            if std::path::Path::new(&p2).exists() {
                return Some(p2);
            }
        }
    }
    None
}

/// 图标名缓存。
pub fn resolve_package_icon(source: &str, pkg_name: &str) -> Option<String> {
    {
        let guard = cache().lock().unwrap();
        if let Some(cached) = guard.get(pkg_name) {
            return cached.clone();
        }
    }

    let result = match source {
        "apt" | "residue" => resolve_apt_icon(pkg_name),
        "flatpak" => resolve_flatpak_icon(pkg_name),
        "snap" => resolve_snap_icon(pkg_name),
        "appimage" => resolve_manual_icon(pkg_name),
        "apm" => resolve_apm_icon(pkg_name),
        _ => None,
    };

    cache().lock().unwrap().insert(pkg_name.to_string(), result.clone());
    result
}

fn resolve_apt_icon(pkg_name: &str) -> Option<String> {
    // 0. 优先使用包自身安装的图标（.list 文件里列的 .png/.svg，如 hermes-agent-cn-desktop.png）
    if let Some(owned) = package_owned_icon(pkg_name) {
        return Some(owned);
    }
    // 1. 通过全局 desktop 索引（Exec 可执行名匹配包名）
    let idx = desktop_index();
    let keys = [pkg_name, &pkg_name.replace('-', "_"), &pkg_name.replace('_', "-")];
    for key in &keys {
        if let Some(icon) = idx.get(*key) {
            if let Some(path) = find_theme_icon(icon) {
                return Some(path);
            }
        }
    }
    // 2. 查找 applications 目录下同名的 .desktop 文件
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let desktop_dirs = [
        format!("{}/.local/share/applications", home),
        "/usr/share/applications".to_string(),
        "/usr/local/share/applications".to_string(),
        "/var/lib/snapd/desktop/applications".to_string(),
    ];
    for dir in desktop_dirs {
        let d = std::path::Path::new(&dir);
        if let Ok(entries) = std::fs::read_dir(d) {
            for e in entries.flatten() {
                let p = e.path();
                if p.extension().map(|x| x.to_string_lossy().to_string()) != Some("desktop".into()) {
                    continue;
                }
                let fname = p
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                if fname == pkg_name || fname == pkg_name.replace('-', "_") || fname == pkg_name.replace('_', "-") {
                    if let Some(icon) = desktop_icon(&p.to_string_lossy()) {
                        if let Some(path) = find_theme_icon(&icon) {
                            return Some(path);
                        }
                        if let Some(path) = fuzzy_theme_icon(&icon) {
                            return Some(path);
                        }
                    }
                }
            }
        }
    }
    // 3. 尝试常见图标名（含模糊匹配：桌面文件写的 Icon 名可能与实际文件名不同）
    for candidate in [pkg_name, &pkg_name.replace('-', "_"), &pkg_name.replace('_', "-")] {
        if let Some(path) = find_theme_icon(candidate) {
            return Some(path);
        }
    }
    if let Some(path) = fuzzy_theme_icon(pkg_name) {
        return Some(path);
    }
    None
}

/// 从 dpkg 的 .list 文件中找到包自身安装的图标文件。
/// 优先 hicolor apps 下的 128x128 / scalable，其次是任意存在的图标。
fn package_owned_icon(pkg_name: &str) -> Option<String> {
    let list = std::fs::read_to_string(format!("/var/lib/dpkg/info/{}.list", pkg_name)).ok()?;
    let mut best: Option<(usize, String)> = None;
    for line in list.lines() {
        let p = line.trim();
        if !p.ends_with(".png") && !p.ends_with(".svg") {
            continue;
        }
        if !p.contains("/icons/") && !p.contains("/pixmaps/") {
            continue;
        }
        if !std::path::Path::new(p).exists() {
            continue;
        }
        let score = if p.contains("/apps/") { 4 } else { 1 }
            + if p.contains("128x128") || p.contains("scalable") { 4 }
            else if p.contains("256x256") { 3 }
            else if p.contains("64x64") { 2 }
            else { 1 }
            + if p.ends_with(".svg") { 1 } else { 0 };
        let cur = best.as_ref().map(|(s, _)| *s).unwrap_or(0);
        if score > cur {
            best = Some((score, p.to_string()));
        }
    }
    best.map(|(_, p)| p)
}

/// 模糊匹配主题图标：desktop 文件里 Icon=hermes，但实际图标文件是 hermes-agent-cn-desktop.png。
fn fuzzy_theme_icon(icon: &str) -> Option<String> {
    if icon.len() < 3 {
        return None;
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let base_dirs = [
        format!("{}/.local/share/icons/hicolor", home),
        format!("{}/.local/share/icons", home),
        "/usr/share/icons/hicolor".to_string(),
        "/usr/share/icons".to_string(),
        "/usr/share/pixmaps".to_string(),
    ];
    let exts = ["png", "svg", "xpm"];
    let mut best: Option<(usize, String)> = None;
    for base in &base_dirs {
        if let Ok(entries) = std::fs::read_dir(base) {
            for e in entries.flatten() {
                let p = e.path();
                if !p.is_dir() {
                    continue;
                }
                if let Ok(sizes) = std::fs::read_dir(&p) {
                    for s in sizes.flatten() {
                        let sp = s.path();
                        if !sp.is_dir() {
                            continue;
                        }
                        if let Ok(apps) = std::fs::read_dir(&sp) {
                            for a in apps.flatten() {
                                let ap = a.path();
                                let fname = ap
                                    .file_name()
                                    .map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or_default();
                                let stem = exts
                                    .iter()
                                    .find_map(|x| fname.strip_suffix(x))
                                    .unwrap_or(&fname);
                                if !ap.is_file()
                                    || !exts.iter().any(|x| fname.ends_with(x))
                                    || !(fname.contains(icon) || icon.contains(stem))
                                {
                                    continue;
                                }
                                let score = if fname.starts_with(icon) { 3 } else { 1 };
                                let cur = best.as_ref().map(|(s, _)| *s).unwrap_or(0);
                                if score > cur {
                                    best = Some((score, ap.to_string_lossy().to_string()));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    best.map(|(_, p)| p)
}

fn resolve_flatpak_icon(appid: &str) -> Option<String> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let base_dirs = [
        format!("{}/.local/share/flatpak/exports/share/icons/hicolor", home),
        "/var/lib/flatpak/exports/share/icons/hicolor".to_string(),
        "/usr/share/flatpak/exports/share/icons/hicolor".to_string(),
        "/var/lib/flatpak/app".to_string(),
    ];
    let sizes = ["128x128", "64x64", "256x256", "48x48", "scalable"];
    for base in &base_dirs {
        for size in &sizes {
            for (sub, ext) in [("apps", "png"), ("apps", "svg")] {
                let p = format!("{}/{}/{}/{}.{}", base, size, sub, appid, ext);
                if std::path::Path::new(&p).exists() {
                    return Some(p);
                }
            }
        }
    }
    None
}

fn resolve_snap_icon(snap_name: &str) -> Option<String> {
    let base = format!("/snap/{}/current/meta/gui", snap_name);
    if let Ok(entries) = std::fs::read_dir(&base) {
        for e in entries.flatten() {
            let p = e.path();
            let ext = p.extension().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
            if ext == "png" || ext == "svg" {
                return Some(p.to_string_lossy().to_string());
            }
        }
    }
    None
}

/// APM 应用图标：桌面入口 Icon= 给出的图标名，实际文件在 APM 容器内 hicolor 树中。
fn resolve_apm_icon(pkg_name: &str) -> Option<String> {
    let icon = apm_desktop_icon_name(pkg_name).unwrap_or_else(|| pkg_name.to_string());
    let container_icon_dirs = [
        "/var/lib/apm/apm/files/ace-env/usr/share/icons/hicolor",
        "/var/lib/apm/apm/files/ace-env/usr/share/icons",
    ];
    let sizes = ["256x256", "scalable", "128x128", "64x64", "48x48", "32x32", "24x24", "22x22", "16x16"];
    let subdirs = ["apps", "mimetypes"];
    let exts = ["png", "svg"];
    for base in &container_icon_dirs {
        for size in &sizes {
            for sub in &subdirs {
                for extn in &exts {
                    let p = format!("{}/{}/{}/{}.{}", base, size, sub, icon, extn);
                    if std::path::Path::new(&p).exists() {
                        return Some(p);
                    }
                }
            }
        }
        for ext in &exts {
            let p = format!("{}/apps/{}.{}", base, icon, ext);
            if std::path::Path::new(&p).exists() {
                return Some(p);
            }
        }
    }
    // 兜底：用主题查找（可能落在宿主图标目录）
    find_theme_icon(&icon).or_else(|| fuzzy_theme_icon(&icon))
}

/// 读取 /usr/local/share/applications 下 Exec 为 `apm run <pkg>` 的 desktop 的 Icon=。
fn apm_desktop_icon_name(pkg_name: &str) -> Option<String> {
    let dir = "/usr/local/share/applications";
    let entries = std::fs::read_dir(dir).ok()?;
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().map(|x| x.to_string_lossy().to_string()) != Some("desktop".into()) {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&p) else {
            continue;
        };
        let mut icon = None;
        let mut is_apm_pkg = false;
        for line in content.lines() {
            let line = line.trim();
            if let Some(r) = line.strip_prefix("Icon=") {
                icon = Some(r.trim().trim_matches('"').to_string());
            } else if let Some(r) = line.strip_prefix("Exec=") {
                is_apm_pkg = r.trim().starts_with("apm run ") && r.trim().split_whitespace().nth(2) == Some(pkg_name);
            }
        }
        if is_apm_pkg {
            return icon;
        }
    }
    None
}

fn resolve_manual_icon(exec_path: &str) -> Option<String> {
    // 手动安装：找 ~/.local/share/applications 中 Exec 指向它的 desktop 文件
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let dir = format!("{}/.local/share/applications", home);
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().map(|x| x.to_string_lossy().to_string()) != Some("desktop".into()) {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&p) {
                if content.contains(exec_path) {
                    if let Some(icon) = desktop_icon(&p.to_string_lossy()) {
                        return find_theme_icon(&icon);
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_icon_for_known_app() {
        // baidunetdisk 在该系统有 scalable/apps/baidunetdisk.svg
        let path = resolve_package_icon("apt", "baidunetdisk");
        assert!(path.is_some(), "baidunetdisk 应有图标");
        if let Some(p) = path {
            assert!(std::path::Path::new(&p).exists(), "图标文件应存在: {}", p);
        }
    }

    #[test]
    fn find_icon_returns_none_for_lib() {
        let path = resolve_package_icon("apt", "libc6");
        let _ = path; // 不强制，仅确认不 panic
    }

    #[test]
    fn theme_icon_scalable() {
        let p = find_theme_icon("baidunetdisk");
        assert!(p.is_some());
    }

    #[test]
    fn owned_icon_for_user_software() {
        // 用户自己开发的软件：desktop Icon=hermes，但包实际安装的图标是 hermes-agent-cn-desktop.png
        let path = resolve_package_icon("apt", "hermes-agent-cn-desktop");
        assert!(path.is_some(), "hermes-agent-cn-desktop 应能通过包自带图标解析");
        if let Some(p) = path {
            assert!(std::path::Path::new(&p).exists(), "图标文件应存在: {}", p);
            assert!(p.contains("hermes"), "应为 hermes 相关图标: {}", p);
        }
    }
}
