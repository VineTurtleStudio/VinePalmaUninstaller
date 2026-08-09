use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Package {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub version: String,
    pub source: String,
    pub description: String,
    pub size: String,
    /// 数值大小（字节），用于排序；0 表示未知
    pub size_bytes: i64,
    /// 安装时间（epoch 秒），用于排序；0 表示未知
    pub installed_at: i64,
    pub essential: bool,
    /// 风险等级：low / medium / high / critical
    pub risk: String,
    /// 是否有图形界面入口（.desktop），用于默认模式过滤
    pub gui: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlannedCommand {
    pub description: String,
    pub command: String,
    pub needs_root: bool,
    pub affects_others: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlannedFile {
    pub path: String,
    pub kind: String,
    pub category: String,
    pub exists: bool,
    pub selected: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct UninstallPlan {
    pub package: Package,
    pub commands: Vec<PlannedCommand>,
    pub files: Vec<PlannedFile>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UninstallRequest {
    pub id: String,
    pub keep_user_config: bool,
    pub keep_system_config: bool,
    pub clean_orphans: bool,
    pub selected_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UninstallResult {
    pub success: bool,
    pub message: String,
    pub executed: Vec<String>,
    pub failed: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SystemInfo {
    pub distro: String,
    pub version: String,
    pub arch: String,
    pub kernel: String,
    pub desktop: String,
}
