use crate::models::{Package, PlannedCommand, UninstallPlan, UninstallRequest, UninstallResult};
use crate::runner;

pub async fn list_installed() -> Result<Vec<Package>, String> {
    let r = runner::run(
        "docker",
        &["images", "--format", "{{.Repository}}:{{.Tag}}\t{{.ID}}\t{{.Size}}\t{{.CreatedAt}}"],
        false,
        60,
    )
    .await?;
    let mut pkgs = Vec::new();
    for line in r.stdout.lines() {
        let mut parts = line.split('\t');
        let (Some(repo_tag), _id, Some(size), Some(created)) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let repo_tag = repo_tag.trim();
        if repo_tag.is_empty() || repo_tag.ends_with("<none>") {
            continue;
        }
        let (repo, tag) = repo_tag.rsplit_once(':').unwrap_or((repo_tag, "latest"));
        pkgs.push(Package {
            id: format!("docker:{}", repo_tag),
            name: repo_tag.to_string(),
            display_name: repo.to_string(),
            version: tag.to_string(),
            source: "docker".into(),
            description: "Docker 镜像".into(),
            size: size.trim().to_string(),
            size_bytes: parse_size(size),
            installed_at: parse_created(created),
            essential: false,
            risk: String::new(),
            gui: true,
        });
    }
    pkgs.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(pkgs)
}

/// 解析 docker 镜像大小，如 "1.2GB"、"512MB"、"25.6kB"。
fn parse_size(s: &str) -> i64 {
    let s = s.trim().to_lowercase();
    let (num, mult) = if s.ends_with("gb") {
        (&s[..s.len() - 2], 1024f64 * 1024.0 * 1024.0)
    } else if s.ends_with("mb") {
        (&s[..s.len() - 2], 1024f64 * 1024.0)
    } else if s.ends_with("kb") {
        (&s[..s.len() - 2], 1024f64)
    } else {
        (s.as_str(), 1.0)
    };
    let n: f64 = num.parse().unwrap_or(0.0);
    (n * mult) as i64
}

/// 解析 docker 的 CreatedAt 时间字符串，如 "2026-07-28 18:48:39 +0800 CST"。
fn parse_created(s: &str) -> i64 {
    let date = s.trim().split_whitespace().next().unwrap_or("");
    let mut it = date.split('-');
    let (y, m, d) = (
        it.next().and_then(|x| x.parse::<i64>().ok()),
        it.next().and_then(|x| x.parse::<i64>().ok()),
        it.next().and_then(|x| x.parse::<i64>().ok()),
    );
    let (Some(y), Some(m), Some(d)) = (y, m, d) else { return 0 };
    // 简单的公历→epoch 换算（忽略时区与闰秒，足够排序用）
    let days = days_from_civil(y, m, d);
    days * 86400
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// 查询某个镜像被哪些容器（含停止的）引用。
async fn referencing_containers(image: &str) -> Vec<String> {
    let r = runner::run(
        "docker",
        &["ps", "-a", "--format", "{{.Names}}\t{{.Image}}"],
        false,
        30,
    )
    .await;
    match r {
        Ok(res) => res
            .stdout
            .lines()
            .filter_map(|l| {
                let mut it = l.split('\t');
                let (name, img) = (it.next(), it.next());
                if let (Some(name), Some(img)) = (name, img) {
                    if img.trim() == image {
                        return Some(name.trim().to_string());
                    }
                }
                None
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

pub fn build_plan(pkg: &Package) -> UninstallPlan {
    UninstallPlan {
        package: pkg.clone(),
        commands: vec![PlannedCommand {
            description: format!("删除 Docker 镜像 {}", pkg.name),
            command: format!("docker rmi -f {}", pkg.name),
            needs_root: false,
            affects_others: false,
        }],
        files: Vec::new(),
        warnings: Vec::new(),
    }
}

pub async fn execute(pkg: &Package, _req: &UninstallRequest, result: &mut UninstallResult) {
    // 先删除引用该镜像的容器（含停止的）
    let containers = referencing_containers(&pkg.name).await;
    for c in &containers {
        let r = runner::run("docker", &["rm", "-f", c], false, 120).await;
        match r {
            Ok(res) if res.success => {
                result.executed.push(format!("已删除容器 {}", c));
            }
            Ok(res) => {
                result.failed.push(format!("删除容器 {} 失败: {}", c, res.stderr.trim()));
            }
            Err(e) => {
                result.failed.push(e);
            }
        }
    }
    let r = runner::run("docker", &["rmi", "-f", &pkg.name], false, 120).await;
    match r {
        Ok(res) if res.success => {
            result.executed.push(format!("已删除镜像 {}", pkg.name));
        }
        Ok(res) => {
            result.success = false;
            result.failed.push(format!("docker rmi {} 失败: {}", pkg.name, res.stderr.trim()));
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
    fn parse_docker_size() {
        assert_eq!(parse_size("1.2GB"), (1.2 * 1024.0 * 1024.0 * 1024.0) as i64);
        assert_eq!(parse_size("512MB"), (512.0 * 1024.0 * 1024.0) as i64);
        assert_eq!(parse_size("25.6kB"), (25.6 * 1024.0) as i64);
    }

    #[test]
    fn parse_docker_created() {
        let t = parse_created("2026-07-28 18:48:39 +0800 CST");
        assert!(t > 1750000000, "应为 2026 年中的时间戳: {}", t);
    }
}
