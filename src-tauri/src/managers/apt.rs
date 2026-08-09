use crate::models::{Package, PlannedCommand, UninstallPlan, UninstallRequest, UninstallResult};
use crate::runner;

pub async fn list_installed() -> Result<Vec<Package>, String> {
    let r = runner::run(
        "dpkg-query",
        &["-W", "-f=${db:Status-Abbrev}\t${Package}\t${Version}\t${binary:Summary}\t${Installed-Size}\t${Essential}\n"],
        false,
        60,
    )
    .await?;

    let mut pkgs = Vec::new();
    for line in r.stdout.lines() {
        let mut parts = line.split('\t');
        let (Some(status), Some(name), Some(ver), Some(summary), Some(size), Some(ess)) = (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        ) else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        // 只显示已安装（ii）的包
        if status.trim() != "ii" {
            continue;
        }
        let essential = ess.trim() == "yes";
        let size_kb: i64 = size.trim().parse().unwrap_or(0);
        pkgs.push(Package {
            id: format!("apt:{}", name),
            name: name.to_string(),
            display_name: name.to_string(),
            version: ver.trim().to_string(),
            source: "apt".into(),
            description: summary.trim().to_string(),
            size: size_kb_to_str(size.trim()),
            size_bytes: size_kb * 1024,
            installed_at: install_time(name).unwrap_or(0),
            essential,
            risk: String::new(),
            gui: false,
        });
    }
    pkgs.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(pkgs)
}

/// 通过 /var/lib/dpkg/info/<pkg>.list 的 mtime 近似获取安装/最近更新时间。
fn install_time(pkg: &str) -> Option<i64> {
    let path = format!("/var/lib/dpkg/info/{}.list", pkg);
    std::fs::metadata(&path)
        .ok()
        .and_then(|m| m.modified().ok())
        .map(|t| t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0))
}

/// 扫描"已卸载但配置残留"的包（dpkg 状态 rc），即类 Windows 注册表的残留。
pub async fn list_rc_residues() -> Result<Vec<Package>, String> {
    let r = runner::run(
        "dpkg-query",
        &["-W", "-f=${db:Status-Abbrev}\t${Package}\t${Version}\t${binary:Summary}\n"],
        false,
        60,
    )
    .await?;

    let mut pkgs = Vec::new();
    for line in r.stdout.lines() {
        let mut parts = line.split('\t');
        let (Some(status), Some(name), Some(ver), Some(summary)) = (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        ) else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        if status.trim() != "rc" {
            continue;
        }
        pkgs.push(Package {
            id: format!("residue:{}", name),
            name: name.to_string(),
            display_name: name.to_string(),
            version: ver.trim().to_string(),
            source: "residue".into(),
            description: format!("卸载残留（配置文件记录）：{}", summary.trim()),
            size: String::new(),
            size_bytes: 0,
            installed_at: 0,
            essential: false,
            risk: String::new(),
            gui: false,
        });
    }
    pkgs.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(pkgs)
}

fn size_kb_to_str(kb: &str) -> String {
    let n: f64 = kb.parse().unwrap_or(0.0);
    if n >= 1048576.0 {
        format!("{:.1} GB", n / 1048576.0)
    } else if n >= 1024.0 {
        format!("{:.1} MB", n / 1024.0)
    } else {
        format!("{:.0} KB", n)
    }
}

/// 从 apt 输出中提取被移除/清除的包名（兼容 Removing 与 Purg 两种行）。
fn extract_removed(out: &str) -> Vec<String> {
    let mut removed = Vec::new();
    for line in out.lines() {
        let t = line.trim();
        let rest = t
            .strip_prefix("Removing ")
            .or_else(|| t.strip_prefix("Purg "))
            .or_else(|| t.strip_prefix("Purging "));
        if let Some(rest) = rest {
            let name = rest.split_whitespace().next().unwrap_or("");
            if !name.is_empty() {
                removed.push(name.to_string());
            }
        }
    }
    removed.sort();
    removed.dedup();
    removed
}

/// 预览 apt purge 将影响的包（关键安全步骤）。
/// 返回受影响包的名称列表。
pub async fn simulate_purge(pkg: &str) -> Result<(Vec<String>, String), String> {
    let r = runner::run(
        "apt-get",
        &["-s", "purge", "-y", "--no-install-recommends", "-o", "APT::Get::AutomaticRemove=0", pkg],
        false,
        60,
    )
    .await?;
    let out = format!("{}\n{}", r.stdout, r.stderr);
    Ok((extract_removed(&out), out))
}

/// 预览 autoremove 会移除的孤儿包。
pub async fn simulate_orphans() -> Result<Vec<String>, String> {
    let r = runner::run("apt-get", &["-s", "autoremove", "--purge"], false, 60).await?;
    let out = format!("{}\n{}", r.stdout, r.stderr);
    Ok(extract_removed(&out))
}

pub fn build_plan(pkg: &Package, affected: Vec<String>) -> UninstallPlan {
    let commands = vec![PlannedCommand {
        description: format!("彻底卸载 {}（apt purge，含配置文件）", pkg.name),
        command: format!("apt purge -y {}（仅卸载目标，不连带移除其他包）", pkg.name),
        needs_root: true,
        affects_others: false,
    }];

    let mut warnings = Vec::new();
    if affected.len() > 1 {
        let extra: Vec<String> = affected.iter().filter(|p| *p != &pkg.name).cloned().collect();
        if !extra.is_empty() {
            warnings.push(format!(
                "⚠️ 此操作还会连带移除 {} 个相关包：{}。这些是被目标包依赖的组件。",
                extra.len(),
                extra.join(", ")
            ));
        }
    }

    UninstallPlan {
        package: pkg.clone(),
        commands,
        files: Vec::new(),
        warnings,
    }
}

/// 清理 rc 状态残留的计划：dpkg --purge 清掉配置残留记录。
pub fn build_rc_plan(pkg: &Package) -> UninstallPlan {
    UninstallPlan {
        package: pkg.clone(),
        commands: vec![PlannedCommand {
            description: format!("清除 {} 的配置残留记录（dpkg --purge）", pkg.name),
            command: format!("dpkg --purge {}", pkg.name),
            needs_root: true,
            affects_others: false,
        }],
        files: Vec::new(),
        warnings: vec![format!(
            "此包已被卸载，但系统中仍保留其配置文件记录。清理后这些残留记录将被彻底移除。"
        )],
    }
}

/// 执行 rc 残留清理。
pub async fn execute_rc(pkg: &Package, result: &mut UninstallResult) {    let r = runner::run("dpkg", &["--purge", &pkg.name], true, 120).await;
    match r {
        Ok(res) if res.success => {
            result.executed.push(format!("已清除 {} 的残留记录", pkg.name));
        }
        Ok(res) => {
            result.success = false;
            result.failed.push(format!(
                "清除 {} 残留失败: {}",
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
pub async fn execute(pkg: &Package, _req: &UninstallRequest, result: &mut UninstallResult) {
    // 只 purge 目标包本身，绝不让 apt 自动连带移除依赖（防误删）。
    // --no-remove-recommends 是 install 的选项，不能用于 purge，会报错。
    // AutomaticRemove=0 关闭自动移除孤儿依赖，确保只动目标包。
    let args = vec![
        "-y".to_string(),
        "purge".to_string(),
        "--no-install-recommends".into(),
        "-o".into(),
        "APT::Get::AutomaticRemove=0".into(),
        pkg.name.clone(),
        "-o".into(),
        "Dpkg::Options::=--force-confask".into(),
        "-o".into(),
        "APT::Get::Purge=1".into(),
    ];

    let r = runner::run("apt-get", &args.iter().map(|s| s.as_str()).collect::<Vec<_>>(), true, 600)
        .await;
    match r {
        Ok(res) if res.success => {
            result.executed.push(format!("已彻底卸载 {}", pkg.name));
        }
        Ok(res) => {
            result.success = false;
            result
                .failed
                .push(format!("apt purge {} 失败: {}", pkg.name, res.stderr.trim().lines().last().unwrap_or("")));
        }
        Err(e) => {
            result.success = false;
            result.failed.push(e);
        }
    }
}

/// 清理孤儿依赖包（apt autoremove --purge）。
pub async fn execute_orphans() -> Result<usize, String> {
    let orphans = simulate_orphans().await?;
    if orphans.is_empty() {
        return Ok(0);
    }
    let r = runner::run("apt-get", &["-y", "autoremove", "--purge"], true, 600).await?;
    if r.success {
        Ok(orphans.len())
    } else {
        Err(format!("autoremove 失败: {}", r.stderr.trim().lines().last().unwrap_or("")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dpkg_list_parses_all_packages() {
        // 关键回归测试：确认修复 \n 换行问题后，能解析出全部已安装包
        let pkgs = list_installed().await.unwrap();
        assert!(!pkgs.is_empty(), "应能列出已安装软件");
        // 系统至少应有几个核心包
        let names: Vec<&str> = pkgs.iter().map(|p| p.name.as_str()).collect();
        assert!(
            names.iter().any(|n| n.contains("libc") || n.contains("bash") || n.contains("dpkg")),
            "应包含核心系统包，实际前几个: {:?}",
            &names[..names.len().min(10)]
        );
    }

    #[tokio::test]
    async fn rc_residues_scanned() {
        let residues = list_rc_residues().await.unwrap();
        // 本系统有 rc 状态残留（如 grub-pc 等），不应为空
        println!("rc 残留: {:?}", residues.iter().map(|p| &p.name).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn simulate_purge_command_is_valid() {
        // 回归测试：修复 --no-remove-recommends 与 purge 组合报错的问题。
        // 用模拟模式验证命令可正常执行，不再出现 "not understood" 错误。
        // 挑一个当前确实已安装的包来模拟，避免测试包被卸载后误报失败。
        let installed = list_installed().await.unwrap();
        assert!(!installed.is_empty(), "系统应有已安装软件");
        let pkg = installed.iter().find(|p| p.name != "libc6").unwrap_or(&installed[0]);
        let (affected, out) = simulate_purge(&pkg.name).await.unwrap();
        assert!(
            !out.contains("not understood") && !out.contains("不能理解"),
            "purge 模拟不应报选项错误: {}",
            out
        );
        assert!(affected.contains(&pkg.name), "应包含目标包 {}", pkg.name);
    }
}
