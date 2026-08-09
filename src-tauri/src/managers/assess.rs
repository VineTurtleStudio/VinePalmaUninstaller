use crate::managers::icons;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

/// APT 包数据库：基于 /var/lib/dpkg/status 与 extended_states 的真实数据。
/// 用于计算风险等级与判断是否"用户级软件"。
#[derive(Debug, Default)]
pub struct AptDb {
    /// 包名 → Priority (required/important/standard/optional/extra)
    priority: HashMap<String, String>,
    /// Essential: yes 的包
    essential: HashSet<String>,
    /// 包名 → 被多少个已安装包依赖（反向依赖数）
    rdepends: HashMap<String, usize>,
    /// Auto-Installed: 1 的包（作为依赖自动安装，非用户手动安装）
    auto_installed: HashSet<String>,
}

static APT_DB: OnceLock<Mutex<Option<AptDb>>> = OnceLock::new();

fn apt_db() -> std::sync::MutexGuard<'static, Option<AptDb>> {
    APT_DB.get_or_init(|| Mutex::new(None)).lock().unwrap()
}

impl AptDb {
    /// 从 dpkg 数据构建（带缓存；包列表变化后由上层调用 reload）。
    pub fn load() -> Self {
        if let Some(db) = apt_db().as_ref() {
            return db.clone_shallow();
        }
        let db = Self::build();
        *apt_db() = Some(db.clone_shallow());
        db
    }

    pub fn reload() -> Self {
        let db = Self::build();
        *apt_db() = Some(db.clone_shallow());
        db
    }

    /// 浅克隆（只拷贝用于查询的 map，避免重复持有同一数据）。
    fn clone_shallow(&self) -> Self {
        AptDb {
            priority: self.priority.clone(),
            essential: self.essential.clone(),
            rdepends: self.rdepends.clone(),
            auto_installed: self.auto_installed.clone(),
        }
    }

    fn build() -> Self {
        let status = std::fs::read_to_string("/var/lib/dpkg/status").unwrap_or_default();
        let mut db = AptDb::default();

        // 反向依赖计数：只统计"已安装"的包对依赖的引用
        let mut installed: Vec<(String, Vec<String>)> = Vec::new();

        for para in status.split("\n\n") {
            let mut pkg = String::new();
            let mut prio = String::new();
            let mut ess = false;
            let mut status_line = String::new();
            let mut deps: Vec<String> = Vec::new();
            let mut cur_field = String::new();

            for line in para.lines() {
                if line.starts_with(' ') {
                    // 续行：追加到上一字段
                    if !cur_field.is_empty() {
                        let v = cur_field.clone();
                        match v.as_str() {
                            "Depends" | "Pre-Depends" | "Recommends" => {
                                let last = deps.last_mut();
                                if let Some(l) = last {
                                    l.push(' ');
                                    l.push_str(line.trim());
                                }
                            }
                            _ => {}
                        }
                    }
                    continue;
                }
                if let Some(rest) = line.strip_prefix("Package:") {
                    pkg = rest.trim().to_string();
                    cur_field = "Package".into();
                } else if let Some(rest) = line.strip_prefix("Status:") {
                    status_line = rest.trim().to_string();
                    cur_field = "Status".into();
                } else if let Some(rest) = line.strip_prefix("Priority:") {
                    prio = rest.trim().to_string();
                    cur_field = "Priority".into();
                } else if let Some(rest) = line.strip_prefix("Essential:") {
                    ess = rest.trim() == "yes";
                    cur_field = "Essential".into();
                } else if let Some(rest) = line.strip_prefix("Depends:") {
                    deps.extend(parse_dep_field(rest.trim()));
                    cur_field = "Depends".into();
                } else if let Some(rest) = line.strip_prefix("Pre-Depends:") {
                    deps.extend(parse_dep_field(rest.trim()));
                    cur_field = "Pre-Depends".into();
                } else if let Some(rest) = line.strip_prefix("Recommends:") {
                    deps.extend(parse_dep_field(rest.trim()));
                    cur_field = "Recommends".into();
                }
            }

            if pkg.is_empty() || !status_line.starts_with("install ok installed") {
                continue;
            }
            // 同一包可能有多个段落（多架构/多版本），优先级取最强、essential 任一为 yes
            let prio_rank = |p: &str| match p {
                "required" => 5,
                "important" => 4,
                "standard" => 3,
                "optional" => 2,
                "extra" => 1,
                _ => 0,
            };
            if !prio.is_empty() {
                let entry = db.priority.entry(pkg.clone()).or_insert_with(|| prio.clone());
                if prio_rank(&prio) > prio_rank(entry) {
                    *entry = prio.clone();
                }
            }
            if ess {
                db.essential.insert(pkg.clone());
            }
            installed.push((pkg.clone(), deps));
        }

        // 计数：pkg 被多少已安装包依赖（同包多段落去重：每包只算一次反向依赖）
        let mut seen_dependents: HashMap<String, HashSet<String>> = HashMap::new();
        for (owner, deps) in &installed {
            for d in deps {
                seen_dependents.entry(d.clone()).or_default().insert(owner.clone());
            }
        }
        for (name, owners) in seen_dependents {
            db.rdepends.insert(name, owners.len());
        }

        // Auto-Installed 集合
        let ext = std::fs::read_to_string("/var/lib/apt/extended_states").unwrap_or_default();
        let mut cur = String::new();
        for line in ext.lines() {
            if let Some(rest) = line.strip_prefix("Package:") {
                cur = rest.trim().to_string();
            } else if line.trim().starts_with("Auto-Installed: 1") && !cur.is_empty() {
                db.auto_installed.insert(cur.clone());
            }
        }

        db
    }

    /// 反向依赖数。
    pub fn rdepends_of(&self, name: &str) -> usize {
        self.rdepends.get(name).copied().unwrap_or(0)
    }

    pub fn is_essential(&self, name: &str) -> bool {
        self.essential.contains(name)
    }

    fn priority_of(&self, name: &str) -> &str {
        self.priority.get(name).map(|s| s.as_str()).unwrap_or("optional")
    }

    /// 计算风险等级。基于真实数据而非名字猜测。
    pub fn risk(&self, source: &str, name: &str, gui: bool) -> &'static str {
        if source != "apt" {
            return "low";
        }
        // Essential 或 required 优先级的包 = 系统生死攸关
        if self.is_essential(name) || self.priority_of(name) == "required" {
            return "critical";
        }
        // GUI 应用是用户直接使用的前端，风险低
        if gui {
            return "low";
        }
        // important 优先级 = 系统关键组件
        if self.priority_of(name) == "important" {
            return "high";
        }
        // 反向依赖越多，越不能删
        let n = self.rdepends_of(name);
        if n >= 8 {
            return "high";
        }
        if n >= 1 {
            return "medium";
        }
        // 无依赖 + 非 essential => 叶子包，风险低
        "low"
    }

    /// 是否"用户级软件"（默认模式显示）。
    /// 判定：非 apt 来源都是用户级；apt 来源 = 有 GUI 桌面入口（用户安装的应用或系统自带的应用），
    /// 系统库、命令行工具、语言包、内核组件等一律隐藏。
    pub fn is_user_level(&self, source: &str, name: &str, gui: bool, essential: bool) -> bool {
        if source != "apt" {
            return true;
        }
        if essential || self.is_essential(name) || self.priority_of(name) == "required" {
            return false;
        }
        // 只有带图形界面入口（.desktop）的包才出现在默认模式，
        // 避免 aria2/git/curl/语言包/库等命令行工具混入。
        gui
    }
}

/// 解析依赖字段：拆出依赖包名（去掉版本约束与架构后缀），含 '|' 的替代也计入。
fn parse_dep_field(field: &str) -> Vec<String> {
    let mut out = Vec::new();
    for item in field.split(',') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        for alt in item.split('|') {
            let alt = alt.trim();
            let name = alt
                .split('(')
                .next()
                .map(|s| s.trim())
                .unwrap_or("")
                .split(':')
                .next()
                .map(|s| s.trim())
                .unwrap_or("");
            if !name.is_empty() {
                out.push(name.to_string());
            }
        }
    }
    out
}

/// 包是否拥有 GUI 入口（.desktop 文件）。
fn owns_desktop(pkg: &str) -> bool {
    let list = std::fs::read_to_string(format!("/var/lib/dpkg/info/{}.list", pkg)).unwrap_or_default();
    list.lines().any(|l| l.trim().ends_with(".desktop"))
}

/// 计算包的风险等级与是否 GUI 应用。
/// source: apt / flatpak / snap / docker / appimage / residue
/// name: 包名（apt 用真实包名，其它用显示名）
pub fn assess(source: &str, name: &str) -> (String, bool) {
    match source {
        "apt" => {
            let db = AptDb::load();
            let gui = owns_desktop(name) || icons::exec_has_desktop(name);
            (db.risk(source, name, gui).to_string(), gui)
        }
        _ => ("low".to_string(), true),
    }
}

/// 判断软件包是否应显示在"默认模式"（只展示用户级软件）。
pub fn is_user_level(source: &str, name: &str, essential: bool) -> bool {
    match source {
        "apt" => {
            let db = AptDb::load();
            let gui = owns_desktop(name) || icons::exec_has_desktop(name);
            db.is_user_level(source, name, gui, essential)
        }
        _ => true,
    }
}

/// 触发 dpkg 数据库重载（卸载后调用，避免返回旧缓存）。
pub fn reload_apt_db() {
    let _ = AptDb::reload();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_dep_field_basic() {
        let deps = parse_dep_field("libc6 (>= 2.39), libgcc-s1 (>= 4.2) | libgcc-s2");
        assert!(deps.contains(&"libc6".to_string()));
        assert!(deps.contains(&"libgcc-s1".to_string()));
        assert!(deps.contains(&"libgcc-s2".to_string()));
    }

    #[test]
    fn db_reverse_deps_counts() {
        let db = AptDb::build();
        // libc6 被极多包依赖
        assert!(db.rdepends_of("libc6") > 10, "libc6 反向依赖应很多");
        // 用户手工装的 hermes 软件不应有太多反向依赖
        assert!(db.rdepends_of("hermes-agent-cn-desktop") <= 1, "hermes 不应被系统依赖");
    }

    #[test]
    fn essential_and_required_are_critical() {
        let db = AptDb::build();
        assert_eq!(db.risk("apt", "base-files", false), "critical");
        assert_eq!(db.risk("apt", "libc6", false), "critical");
    }

    #[test]
    fn user_software_is_low_risk_and_user_level() {
        let db = AptDb::build();
        // 用户手工安装、无反向依赖的 GUI 软件 => 低风险 + 用户级
        assert_eq!(db.risk("apt", "hermes-agent-cn-desktop", true), "low");
        assert!(db.is_user_level("apt", "hermes-agent-cn-desktop", true, false));
        // 系统核心 => 非用户级
        assert!(!db.is_user_level("apt", "libc6", false, true));
    }

    #[test]
    fn non_apt_is_user_level_and_low() {
        let (risk, gui) = assess("flatpak", "org.example.App");
        assert_eq!(risk, "low");
        assert!(gui);
        assert!(is_user_level("snap", "spotify", false));
    }
}
