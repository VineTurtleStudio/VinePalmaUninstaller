use crate::managers::{apm, appimage, apt, assess, cleanup, docker, flatpak, icons, snap};
use crate::models::{Package, SystemInfo, UninstallPlan, UninstallRequest, UninstallResult};

/// 扫描所有来源的已安装软件。
/// mode: "default" 只返回用户级软件；"advanced" 返回全部。
#[tauri::command]
pub async fn list_packages(mode: String) -> Result<Vec<Package>, String> {
    // 每次扫描都重载 dpkg 数据缓存，确保重装后的软件也能被识别
    assess::reload_apt_db();
    icons::clear_cache();
    let mut all = Vec::new();
    let mut errors = Vec::new();

    match apt::list_installed().await {
        Ok(mut p) => all.append(&mut p),
        Err(e) => errors.push(e),
    }
    if let Ok(mut p) = flatpak::list_installed().await {
        all.append(&mut p);
    }
    if let Ok(mut p) = snap::list_installed().await {
        all.append(&mut p);
    }
    if let Ok(mut p) = docker::list_installed().await {
        all.append(&mut p);
    }
    if let Ok(mut p) = appimage::list_installed().await {
        all.append(&mut p);
    }
    if let Ok(mut p) = apm::list_installed().await {
        all.append(&mut p);
    }

    // 给每个包标注风险等级与 GUI 标记
    for p in all.iter_mut() {
        let (risk, gui) = assess::assess(&p.source, &p.name);
        p.risk = risk;
        p.gui = gui;
    }

    // 默认模式：智能过滤，只保留用户级软件
    let advanced = mode == "advanced";
    if !advanced {
        all.retain(|p| assess::is_user_level(&p.source, &p.name, p.essential));
    }

    let _ = errors;
    Ok(all)
}

/// 扫描"卸载残留"（已卸载但留下配置记录的包）。
#[tauri::command]
pub async fn list_residues() -> Result<Vec<Package>, String> {
    apt::list_rc_residues().await
}

/// 生成卸载计划（含预览：明确列出会被移除的包内容，绝无意外）。
#[tauri::command]
pub async fn get_uninstall_plan(id: String) -> Result<UninstallPlan, String> {
    // 通过 id 反查包信息（服务端重新扫描，不信任前端传入的任意路径）
    let pkgs = list_packages("advanced".to_string()).await?;
    let pkg = pkgs
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| format!("未找到软件: {}", id))?;
    build_plan_for(pkg).await
}

/// 服务端生成卸载/清理计划。这是"可删除路径"的唯一出处：
/// 前端只能从该计划里勾选，执行阶段用它做白名单校验（绝不含任意路径）。
async fn build_plan_for(pkg: &Package) -> Result<UninstallPlan, String> {
    let (prefix, name) = pkg.id.split_once(':').unwrap_or(("apt", &pkg.id));
    let name = name.to_string();

    let mut plan = match prefix {
        "flatpak" => flatpak::build_plan(pkg),
        "snap" => snap::build_plan(pkg),
        "docker" => docker::build_plan(pkg),
        "manual" => appimage::build_plan(pkg),
        "apm" => apm::build_plan(pkg),
        _ => {
            // apt：先 simulate 预览，确认不影响其它软件
            let (affected, _detail) = apt::simulate_purge(&name).await.unwrap_or_else(|_| (vec![name.clone()], String::new()));
            apt::build_plan(pkg, affected)
        }
    };

    // 追加残留清理候选
    plan.files.extend(plan_residue_files(prefix, pkg));

    Ok(plan)
}

/// 每种来源的残留清理候选文件。
fn plan_residue_files(prefix: &str, pkg: &Package) -> Vec<crate::models::PlannedFile> {
    match prefix {
        "flatpak" => cleanup::flatpak_residue(pkg),
        "snap" => cleanup::snap_residue(pkg),
        "manual" | "apm" => cleanup::manual_residue(pkg),
        // Docker 镜像没有按包名的宿主残留目录，避免生成 ~/.config/<repo:tag> 等错误路径
        "docker" => Vec::new(),
        _ => cleanup::apt_residue(pkg),
    }
}

/// 生成残留清理计划（rc 状态包）。
#[tauri::command]
pub async fn get_residue_plan(id: String) -> Result<UninstallPlan, String> {
    let residues = apt::list_rc_residues().await?;
    let pkg = residues
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| format!("未找到残留: {}", id))?;
    Ok(apt::build_rc_plan(pkg))
}

/// 预览 apt 孤儿包（可选的 autoremove 清理）。
#[tauri::command]
pub async fn preview_orphans() -> Result<Vec<String>, String> {
    apt::simulate_orphans().await
}

/// 清理孤儿依赖包。返回清理数量。
#[tauri::command]
pub async fn clean_orphans() -> Result<usize, String> {
    apt::execute_orphans().await
}

/// 获取软件图标绝对路径（找不到返回 null）。
#[tauri::command]
pub async fn get_package_icon(source: String, name: String) -> Result<Option<String>, String> {
    Ok(icons::resolve_package_icon(&source, &name))
}

/// 获取系统信息（发行版 / 架构 / 内核 / 桌面环境）。
/// 跨发行版识别，仅用于界面展示。
#[tauri::command]
pub fn get_system_info() -> Result<SystemInfo, String> {
    Ok(system_info())
}

fn system_info() -> SystemInfo {
    let os_rel = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let mut name = String::new();
    let mut pretty = String::new();
    let mut version = String::new();
    for line in os_rel.lines() {
        if let Some(r) = line.strip_prefix("NAME=") {
            name = clean_quoted(r);
        } else if let Some(r) = line.strip_prefix("PRETTY_NAME=") {
            pretty = clean_quoted(r);
        } else if let Some(r) = line.strip_prefix("VERSION_ID=") {
            version = clean_quoted(r);
        }
    }
    let mut distro = if !pretty.is_empty() { pretty } else { name };
    let fallback = match std::fs::read_to_string("/etc/redhat-release") {
        Ok(rel) => rel.trim().to_string(),
        Err(_) => String::new(),
    };
    if distro.is_empty() && !fallback.is_empty() {
        distro = fallback;
    }
    if distro.is_empty() {
        distro = "Linux".to_string();
    }

    let arch = std::env::consts::ARCH.to_string();
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    let desktop = [("XDG_CURRENT_DESKTOP", "XDG_CURRENT_DESKTOP"), ("DESKTOP_SESSION", "DESKTOP_SESSION")]
        .iter()
        .find_map(|(k, _)| std::env::var(k).ok().filter(|v| !v.is_empty()))
        .unwrap_or_default();

    SystemInfo {
        distro,
        version,
        arch,
        kernel,
        desktop,
    }
}

fn clean_quoted(s: &str) -> String {
    s.trim().trim_matches('"').trim_matches('\'').to_string()
}

/// 执行卸载。
#[tauri::command]
pub async fn execute_uninstall(
    req: UninstallRequest,
) -> Result<UninstallResult, String> {
    let pkgs = list_packages("advanced".to_string()).await?;
    let pkg = pkgs
        .iter()
        .find(|p| p.id == req.id)
        .ok_or_else(|| format!("未找到软件: {}", req.id))?;

    let (prefix, _name) = pkg.id.split_once(':').unwrap_or(("apt", &pkg.id));
    let mut result = UninstallResult {
        success: true,
        message: String::new(),
        executed: Vec::new(),
        failed: Vec::new(),
    };

    match prefix {
        "flatpak" => flatpak::execute(pkg, &req, &mut result).await,
        "snap" => snap::execute(pkg, &req, &mut result).await,
        "docker" => docker::execute(pkg, &req, &mut result).await,
        "manual" => appimage::execute(pkg, &req, &mut result).await,
        "apm" => apm::execute(pkg, &req, &mut result).await,
        _ => apt::execute(pkg, &req, &mut result).await,
    }

    // 清理残留文件（用户已确认的）。以"服务端重新生成的本包计划"为白名单：
    // 前端只能勾选计划里列出的路径，杜绝提交任意系统路径造成误删。
    if !req.selected_files.is_empty() {
        let allowed: std::collections::HashSet<String> = build_plan_for(pkg)
            .await?
            .files
            .into_iter()
            .map(|f| f.path)
            .collect();
        let keep_user = req.keep_user_config;
        let keep_system = req.keep_system_config;
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
        let user_filtered: Vec<String> = req
            .selected_files
            .iter()
            .filter(|p| allowed.contains(*p))
            .filter(|p| {
                if keep_user && (p.starts_with(&home) || p.starts_with("/home/")) {
                    return false;
                }
                if keep_system && (p.starts_with("/etc") || p.starts_with("/var") || p.starts_with("/opt")) {
                    return false;
                }
                true
            })
            .cloned()
            .collect();
        cleanup::remove_selected(&user_filtered, &mut result).await;
    }

    // 可选：清理 apt 孤儿依赖
    if req.clean_orphans && prefix == "apt" {
        let orphans = apt::simulate_orphans().await.unwrap_or_default();
        if !orphans.is_empty() {
            let r = crate::runner::run("apt-get", &["-y", "autoremove", "--purge"], true, 600).await;
            match r {
                Ok(res) if res.success => {
                    result.executed.push(format!("已清理 {} 个孤儿依赖包", orphans.len()));
                }
                Ok(res) => {
                    result.failed.push(format!("孤儿依赖清理失败: {}", res.stderr.trim()));
                }
                Err(e) => result.failed.push(e),
            }
        }
    }

    if result.failed.is_empty() {
        result.success = true;
        result.message = "卸载完成".into();
    } else {
        result.message = format!("卸载未完全成功，{} 项失败", result.failed.len());
    }
    assess::reload_apt_db();
    Ok(result)
}

/// 执行残留清理（rc 状态包）。
#[tauri::command]
pub async fn execute_residue_cleanup(id: String) -> Result<UninstallResult, String> {
    let residues = apt::list_rc_residues().await?;
    let pkg = residues
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| format!("未找到残留: {}", id))?;

    let mut result = UninstallResult {
        success: true,
        message: String::new(),
        executed: Vec::new(),
        failed: Vec::new(),
    };
    apt::execute_rc(pkg, &mut result).await;

    if result.failed.is_empty() {
        result.success = true;
        result.message = "残留清理完成".into();
    } else {
        result.message = format!("清理未完全成功，{} 项失败", result.failed.len());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn default_mode_filters_system_packages() {
        let def = list_packages("default".to_string()).await.unwrap();
        let adv = list_packages("advanced".to_string()).await.unwrap();
        // 高级模式包含全部，默认模式明显更少（系统库/内核/依赖被过滤）
        assert!(
            def.len() < adv.len() / 2,
            "默认模式应显著少于高级模式: default={} advanced={}",
            def.len(),
            adv.len()
        );
        // 用户自己的软件必须在默认模式中可见
        assert!(
            def.iter().any(|p| p.name == "hermes-agent-cn-desktop"),
            "用户安装的 hermes 软件应在默认模式可见"
        );
        // 系统核心（libc6）不应在默认模式出现
        assert!(
            !def.iter().any(|p| p.name == "libc6"),
            "libc6 是系统核心，不应在默认模式出现"
        );
    }

    #[tokio::test]
    async fn advanced_mode_marks_risk() {
        let adv = list_packages("advanced".to_string()).await.unwrap();
        assert!(adv.iter().all(|p| !p.risk.is_empty()), "高级模式所有包都应有风险等级");
        assert!(
            adv.iter().any(|p| p.risk == "critical"),
            "应存在 critical 风险包"
        );
        assert!(
            adv.iter().any(|p| p.risk == "high"),
            "应存在 high 风险包"
        );
    }
}

    #[tokio::test]
    async fn packages_have_size_and_install_time() {
        let adv = list_packages("advanced".to_string()).await.unwrap();
        // hermes 是用户软件，应有大小与安装时间
        let hermes = adv.iter().find(|p| p.name == "hermes-agent-cn-desktop").expect("hermes 存在");
        assert!(hermes.installed_at > 0, "hermes 应有安装时间");
        // libc6 应有大小
        let libc = adv.iter().find(|p| p.name == "libc6").expect("libc6 存在");
        assert!(libc.size_bytes > 0, "libc6 应有数值大小: {}", libc.size_bytes);
    }
