<script setup>
import { ref, computed, onMounted, watch } from "vue";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";

const packages = ref([]);
const residues = ref([]);
const activeTab = ref("installed"); // installed | residues
const loading = ref(false);
const resLoading = ref(false);
const search = ref("");
const sourceFilter = ref("all");
const viewMode = ref("default"); // default | advanced
const sortKey = ref("name"); // name | size | time | risk
const sortDir = ref(1); // 1 asc, -1 desc
const selected = ref(null);
const plan = ref(null);
const planLoading = ref(false);
const uninstalling = ref(false);
const result = ref(null);
const orphans = ref([]);
const showOrphans = ref(false);
const orphanLoading = ref(false);
const toast = ref(null);
let toastTimer = null;

function showToast(msg, kind = "ok") {
  toast.value = { msg, kind };
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (toast.value = null), 3000);
}

const iconMap = ref({});

async function loadIcon(pkg) {
  if (iconMap.value[pkg.id] !== undefined) return;
  try {
    const path = await invoke("get_package_icon", { source: pkg.source, name: pkg.name });
    iconMap.value = {
      ...iconMap.value,
      [pkg.id]: path ? convertFileSrc(path) : null,
    };
  } catch {
    iconMap.value = { ...iconMap.value, [pkg.id]: null };
  }
}

const sources = computed(() => {
  const map = {};
  for (const p of packages.value) {
    map[p.source] = (map[p.source] || 0) + 1;
  }
  return map;
});

const riskRank = (r) => ({"critical": 4, high: 3, medium: 2, low: 1}[r] || 0);

const filtered = computed(() => {
  const q = search.value.trim().toLowerCase();
  const list = packages.value.filter((p) => {
    if (sourceFilter.value !== "all" && p.source !== sourceFilter.value) return false;
    if (!q) return true;
    return (
      p.name.toLowerCase().includes(q) ||
      p.display_name.toLowerCase().includes(q) ||
      p.description.toLowerCase().includes(q)
    );
  });
  const dir = sortDir.value;
  switch (sortKey.value) {
    case "size":
      list.sort((a, b) => dir * ((a.size_bytes || 0) - (b.size_bytes || 0)));
      break;
    case "time":
      list.sort((a, b) => dir * ((a.installed_at || 0) - (b.installed_at || 0)));
      break;
    case "risk":
      list.sort((a, b) => dir * (riskRank(a.risk) - riskRank(b.risk)));
      break;
    default:
      list.sort((a, b) => dir * a.display_name.localeCompare(b.display_name, "zh"));
  }
  return list;
});

function setSort(key) {
  if (sortKey.value === key) {
    sortDir.value = -sortDir.value; // 再次点击反转
  } else {
    sortKey.value = key;
    sortDir.value = key === "name" ? 1 : -1; // 非名称默认倒序（最近安装/最大/高风险在前）
  }
}

function sortArrow(key) {
  if (sortKey.value !== key) return "";
  return sortDir.value === 1 ? "↑" : "↓";
}

let iconBatchTimer = null;
watch(filtered, (list) => {
  clearTimeout(iconBatchTimer);
  iconBatchTimer = setTimeout(() => {
    for (const p of list.slice(0, 200)) {
      loadIcon(p);
    }
  }, 80);
});

async function refresh() {
  loading.value = true;
  try {
    packages.value = await invoke("list_packages", { mode: viewMode.value });
    iconMap.value = {};
  } catch (e) {
    alert("扫描失败: " + e);
  }
  loading.value = false;
}

const modeTip = {
  default: "仅显示用户级软件，系统库、内核和依赖默认隐藏。",
  advanced: "显示全部软件并标注风险等级，请谨慎操作。"
};

function riskLabel(r) {
  return { low: "低风险", medium: "中风险", high: "高风险", critical: "核心" }[r] || r;
}

function switchMode(mode) {
  if (mode === viewMode.value) return;
  viewMode.value = mode;
  sourceFilter.value = "all";
  refresh();
}

async function refreshResidues() {
  resLoading.value = true;
  try {
    residues.value = await invoke("list_residues");
    iconMap.value = {};
  } catch (e) {
    alert("扫描残留失败: " + e);
  }
  resLoading.value = false;
}

async function openPlan(pkg) {
  selected.value = pkg;
  plan.value = null;
  result.value = null;
  planLoading.value = true;
  try {
    if (pkg.source === "residue") {
      plan.value = await invoke("get_residue_plan", { id: pkg.id });
    } else {
      plan.value = await invoke("get_uninstall_plan", { id: pkg.id });
    }
  } catch (e) {
    alert("生成卸载计划失败: " + e);
  }
  planLoading.value = false;
}

async function toggleOrphans() {
  showOrphans.value = !showOrphans.value;
  if (showOrphans.value && orphans.value.length === 0) {
    orphanLoading.value = true;
    try {
      orphans.value = await invoke("preview_orphans");
    } catch (e) {
      orphans.value = [];
    }
    orphanLoading.value = false;
  }
}

async function runOrphanCleanup() {
  if (!orphans.value.length) return;
  uninstalling.value = true;
  try {
    await invoke("clean_orphans");
    await refresh();
    alert(`已清理 ${orphans.value.length} 个孤儿依赖包`);
    orphans.value = await invoke("preview_orphans");
  } catch (e) {
    alert("清理孤儿依赖失败: " + e);
  }
  uninstalling.value = false;
}

const toggleFile = (f) => {
  f.selected = !f.selected;
};

async function doUninstall() {
  if (!plan.value) return;
  uninstalling.value = true;
  result.value = null;
  const selectedFiles = plan.value.files.filter((f) => f.selected).map((f) => f.path);
  try {
    if (plan.value.package.source === "residue") {
      result.value = await invoke("execute_residue_cleanup", { id: plan.value.package.id });
    } else {
      result.value = await invoke("execute_uninstall", {
        req: {
          id: plan.value.package.id,
          keep_user_config: false,
          keep_system_config: false,
          clean_orphans: plan.value.cleanOrphans || false,
          selected_files: selectedFiles,
        },
      });
    }
    if (plan.value.package.source === "residue") {
      await refreshResidues();
    } else {
      await refresh();
      await refreshResidues();
    }
    // 卸载成功：提示 + 返回列表，避免停在"确认卸载"按钮让用户误以为没成功
    if (result.value && result.value.success) {
      showToast(`✓ ${plan.value.package.display_name} 已卸载完成`);
      selected.value = null;
      plan.value = null;
    }
  } catch (e) {
    result.value = { success: false, message: e, executed: [], failed: [e] };
  }
  uninstalling.value = false;
}

const sourceLabel = {
  apt: "APT",
  flatpak: "Flatpak",
  snap: "Snap",
  docker: "Docker",
  appimage: "AppImage",
  apm: "APM",
  residue: "残留",
};

const sourceColor = {
  apt: "#3b82f6",
  flatpak: "#8b5cf6",
  snap: "#ef4444",
  docker: "#0ea5e9",
  appimage: "#f59e0b",
  apm: "#22c55e",
  residue: "#f97316",
};

const sourceOrder = ["apt", "flatpak", "snap", "docker", "appimage", "apm", "residue"];

function srcLabel(src) {
  return sourceLabel[src] || src;
}

function srcColor(src) {
  return sourceColor[src] || "#9ca3af";
}

const orderedSources = computed(() => {
  const list = Object.entries(sources.value);
  list.sort((a, b) => {
    const ia = sourceOrder.indexOf(a[0]);
    const ib = sourceOrder.indexOf(b[0]);
    const ra = ia === -1 ? 99 : ia;
    const rb = ib === -1 ? 99 : ib;
    if (ra !== rb) return ra - rb;
    return a[0].localeCompare(b[0]);
  });
  return list;
});

const systemInfo = ref(null);

async function loadSystemInfo() {
  try {
    systemInfo.value = await invoke("get_system_info");
  } catch {
    systemInfo.value = null;
  }
}

function formatSize(s) {
  return s || "—";
}

function formatTime(t) {
  if (!t) return "—";
  const d = new Date(t * 1000);
  const now = Date.now();
  const diff = now - t * 1000;
  if (diff < 60 * 1000) return "刚刚";
  if (diff < 60 * 60 * 1000) return `${Math.floor(diff / 60000)} 分钟前`;
  if (diff < 24 * 60 * 60 * 1000) return `${Math.floor(diff / 3600000)} 小时前`;
  if (diff < 7 * 24 * 60 * 60 * 1000) return `${Math.floor(diff / 86400000)} 天前`;
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

function catLabel(c) {
  return {
    config: "配置",
    cache: "缓存",
    data: "数据",
    system: "系统",
    appimage: "程序",
    desktop: "启动项",
  }[c] || c;
}

onMounted(() => {
  refresh();
  refreshResidues();
  loadSystemInfo();
});

watch(residues, (list) => {
  clearTimeout(iconBatchTimer);
  iconBatchTimer = setTimeout(() => {
    for (const p of list.slice(0, 200)) {
      loadIcon(p);
    }
  }, 80);
});
</script>

<template>
  <div class="app">
    <header>
      <div class="logo">🗑️</div>
      <div class="title">
        <h1>VinePalma 卸载器</h1>
        <span>彻底卸载 · 绝不误删</span>
      </div>
      <div class="actions">
        <button class="btn-ghost" @click="toggleOrphans">
          {{ showOrphans ? "隐藏" : "孤儿依赖" }}
        </button>
        <button class="btn-ghost" @click="activeTab === 'installed' ? refresh() : refreshResidues()"
          :disabled="loading || resLoading">
          {{ loading || resLoading ? "扫描中…" : "刷新" }}
        </button>
      </div>
    </header>

    <div class="toolbar">
      <div class="toolbar-row top">
        <div class="mode-toggle">
          <button class="mode-btn" :class="{ active: viewMode === 'default' }" @click="switchMode('default')">
            默认模式
          </button>
          <button class="mode-btn" :class="{ active: viewMode === 'advanced' }" @click="switchMode('advanced')">
            高级模式
          </button>
        </div>
        <div class="tabs">
          <button class="tab" :class="{ active: activeTab === 'installed' }" @click="activeTab = 'installed'">
            已安装软件
          </button>
          <button class="tab" :class="{ active: activeTab === 'residues' }" @click="activeTab = 'residues'">
            卸载残留 <span v-if="residues.length" class="tab-badge">{{ residues.length }}</span>
          </button>
        </div>
        <input v-model="search" class="search" placeholder="搜索软件名称…" />
        <select class="sort-select" v-model="sortKey" @change="setSort($event.target.value)">
          <option value="name">名称排序</option>
          <option value="time">安装时间</option>
          <option value="size">占用大小</option>
          <option value="risk">风险等级</option>
        </select>
        <button class="sort-dir" @click="sortDir = -sortDir" :title="sortDir === 1 ? '当前升序' : '当前降序'">
          {{ sortDir === 1 ? "升序 ↑" : "降序 ↓" }}
        </button>
        <span class="count-badge" v-if="activeTab === 'installed'">{{ filtered.length }} 个</span>
        <span v-if="systemInfo" class="sys-badge">{{ systemInfo.distro }} · {{ systemInfo.arch }}</span>
      </div>
      <div class="toolbar-row filters-row" v-if="activeTab === 'installed'">
        <div class="filters">
          <button
            v-for="[src, count] in orderedSources"
            :key="src"
            class="filter-chip"
            :class="{ active: sourceFilter === src }"
            :style="{ borderColor: srcColor(src) }"
            @click="sourceFilter = sourceFilter === src ? 'all' : src"
          >
            {{ srcLabel(src) }} · {{ count }}
          </button>
          <button
            class="filter-chip"
            :class="{ active: sourceFilter === 'all' }"
            @click="sourceFilter = 'all'"
          >
            全部
          </button>
        </div>
      </div>
    </div>

    <div class="mode-banner">
      {{ modeTip[viewMode] }}
    </div>

    <main class="layout">
      <div class="pkg-list" :class="{ advanced: viewMode === 'advanced' }" v-show="!selected">
        <div class="table-head">
          <span class="col-icon"></span>
          <span class="col-name sortable" @click="setSort('name')">名称 {{ sortArrow('name') }}</span>
          <span class="col-ver">版本</span>
          <span class="col-src">来源</span>
          <span class="col-size sortable" @click="setSort('size')">大小 {{ sortArrow('size') }}</span>
          <span class="col-time sortable" @click="setSort('time')">安装时间 {{ sortArrow('time') }}</span>
          <span class="col-risk sortable" v-if="viewMode === 'advanced'" @click="setSort('risk')">风险 {{ sortArrow('risk') }}</span>
          <span class="col-op"></span>
        </div>

        <div v-if="activeTab === 'installed'" class="list-body" :class="{ loading: loading }">
          <div v-if="loading" class="empty">正在扫描系统中已安装的软件…</div>
          <div v-else-if="filtered.length === 0" class="empty">没有找到软件</div>
          <div
            v-for="p in filtered"
            :key="p.id"
            class="row"
            @dblclick="openPlan(p)"
          >
            <span class="col-icon">
              <img v-if="iconMap[p.id]" :src="iconMap[p.id]" class="app-icon" @error="iconMap[p.id] = null" />
            </span>
            <span class="col-name" :title="p.description">
              <span class="name-txt">{{ p.display_name }}</span>
              <small v-if="p.essential" class="essential">核心</small>
            </span>
            <span class="col-ver">{{ p.version }}</span>
            <span class="col-src">
              <span class="src-badge" :style="{ color: srcColor(p.source) }">
                {{ srcLabel(p.source) }}
              </span>
            </span>
            <span class="col-size">{{ formatSize(p.size) }}</span>
            <span class="col-time">{{ formatTime(p.installed_at) }}</span>
            <span class="col-risk" v-if="viewMode === 'advanced'">
              <span class="risk" :class="'risk-' + p.risk">{{ riskLabel(p.risk) }}</span>
            </span>
            <span class="col-op">
              <button class="btn-uninstall" @click.stop="openPlan(p)">卸载</button>
            </span>
          </div>
        </div>

        <div v-else class="list-body" :class="{ loading: resLoading }">
          <div v-if="resLoading" class="empty">正在扫描卸载残留…</div>
          <div v-else-if="residues.length === 0" class="empty">
            <div class="clean-check">✅ 没有卸载残留</div>
            <div class="empty-sub">系统中未发现已卸载但留有配置记录的项目</div>
          </div>
          <div
            v-for="p in residues"
            :key="p.id"
            class="row"
            @dblclick="openPlan(p)"
          >
            <span class="col-icon"></span>
            <span class="col-name" :title="p.description">
              <span class="name-txt">{{ p.display_name }}</span>
              <small class="residue-badge">残留</small>
            </span>
            <span class="col-ver">{{ p.version }}</span>
            <span class="col-src">
              <span class="src-badge" style="color: #f97316">残留</span>
            </span>
            <span class="col-size">—</span>
            <span class="col-time">{{ formatTime(p.installed_at) }}</span>
            <span class="col-op">
              <button class="btn-uninstall" @click.stop="openPlan(p)">清理</button>
            </span>
          </div>
        </div>
      </div>

      <section class="detail" v-if="selected">
        <div class="detail-head">
          <button class="btn-back" @click="selected = null; plan = null">← 返回列表</button>
          <h2>{{ plan ? plan.package.display_name : selected.display_name }}</h2>
        </div>

        <div v-if="planLoading" class="empty">正在分析将影响的内容…</div>

        <template v-else-if="plan">
          <div class="risk-banner" v-if="plan.package.risk === 'critical' || plan.package.risk === 'high'">
            ⚠️ 风险提示：<strong>{{ riskLabel(plan.package.risk) }}</strong> —
            此软件属于系统核心组件，卸载可能导致系统异常或无法开机，请谨慎确认。
          </div>
          <div class="warn-box" v-if="plan.warnings.length">
            <div v-for="w in plan.warnings" :key="w" class="warn-item">🔴 {{ w }}</div>
          </div>

          <div class="section-title">将执行的操作</div>
          <div class="cmd-list">
            <div v-for="c in plan.commands" :key="c.description" class="cmd-item">
              <span class="cmd-desc">{{ c.description }}</span>
              <code>{{ c.command }}</code>
              <span v-if="c.needs_root" class="root-badge">需要授权</span>
            </div>
          </div>

          <template v-if="plan.files.length">
            <div class="section-title">
              残留文件清理
              <small>（勾选的要删除；未勾选将保留）</small>
            </div>
            <div class="file-list">
              <label v-for="f in plan.files" :key="f.path" class="file-item">
                <input type="checkbox" :checked="f.selected" @change="toggleFile(f)" />
                <span class="file-cat" :class="f.category">{{ catLabel(f.category) }}</span>
                <code>{{ f.path }}</code>
              </label>
            </div>
          </template>

          <div class="orphan-box" v-if="selected.source === 'apt'">
            <label class="file-item">
              <input type="checkbox" v-model="plan.cleanOrphans" />
              <span class="file-cat">🔗</span>
              <span>同时清理不再需要的孤儿依赖包（会先预览）</span>
            </label>
          </div>

          <div class="result-box" v-if="result">
            <div :class="result.success ? 'ok' : 'fail'">
              <strong>{{ result.success ? "✓ 卸载完成" : "✗ 卸载未完全成功" }}</strong>
              <div v-for="(e, i) in result.executed" :key="'e' + i" class="res-line ok-line">· {{ e }}</div>
              <div v-for="(f, i) in result.failed" :key="'f' + i" class="res-line fail-line">· {{ f }}</div>
            </div>
          </div>

          <div class="detail-actions">
            <button class="btn-danger" :disabled="uninstalling" @click="doUninstall">
              {{ uninstalling ? "执行中…（可能弹出授权窗口）" : "确认卸载" }}
            </button>
            <button class="btn-ghost" @click="selected = null; plan = null" :disabled="uninstalling">
              取消
            </button>
          </div>
        </template>
      </section>
    </main>

    <div class="orphan-panel" v-if="showOrphans">
      <div class="orphan-head">
        <strong>孤儿依赖包预览</strong>
        <button class="btn-ghost small" @click="showOrphans = false">×</button>
      </div>
      <div v-if="orphanLoading" class="empty">加载中…</div>
      <div v-else-if="orphans.length === 0" class="no-residue">系统很干净，没有孤儿依赖</div>
      <div v-else>
        <p class="orphan-note">这些包不再被任何软件依赖。清理它们通常安全，但建议仅在确实不需要时清理。</p>
        <div class="orphan-tags">
          <span v-for="o in orphans" :key="o" class="orphan-tag">{{ o }}</span>
        </div>
        <button
          class="btn-danger small"
          :disabled="uninstalling"
          @click="runOrphanCleanup"
        >
          清理孤儿依赖
        </button>
      </div>
    </div>

    <transition name="toast">
      <div class="toast" :class="toast.kind" v-if="toast">{{ toast.msg }}</div>
    </transition>
  </div>
</template>

<style>
* { margin: 0; padding: 0; box-sizing: border-box; }
:root {
  color-scheme: dark;
  --bg: #1a1d23;
  --panel: #23272f;
  --panel2: #2a2f3a;
  --border: #343a47;
  --text: #e6e8ee;
  --muted: #aab2c2;
  --accent: #4f8cff;
  --danger: #ff5252;
}
body {
  background: var(--bg);
  color: var(--text);
  font-family: "Segoe UI", "Noto Sans SC", system-ui, sans-serif;
  font-size: 14px;
  overflow: hidden;
}
#app { height: 100vh; }
.app { height: 100vh; display: flex; flex-direction: column; }

header {
  display: flex; align-items: center; gap: 12px;
  padding: 12px 20px;
  background: var(--panel);
  border-bottom: 1px solid var(--border);
}
.logo { font-size: 26px; }
.title h1 { font-size: 16px; font-weight: 600; }
.title span { font-size: 12px; color: var(--muted); }
.actions { margin-left: auto; display: flex; gap: 8px; }

.toolbar {
  display: flex; flex-direction: column; gap: 8px;
  padding: 10px 20px;
  background: var(--panel);
  border-bottom: 1px solid var(--border);
}
.toolbar-row {
  display: flex; align-items: center; gap: 12px;
  min-width: 0;
}
.filters-row { flex-wrap: wrap; }
.toolbar-row.top { flex-wrap: wrap; }
.tabs { display: flex; gap: 4px; flex-shrink: 0; }
.mode-toggle {
  display: flex; gap: 4px; flex-shrink: 0;
  background: var(--bg); border: 1px solid var(--border);
  border-radius: 8px; padding: 3px;
}
.mode-btn {
  background: transparent; color: var(--muted);
  border: none; border-radius: 6px;
  padding: 6px 14px; cursor: pointer; font-size: 13px;
}
.mode-btn.active { background: var(--accent); color: #fff; }
.mode-banner {
  background: var(--panel); border-bottom: 1px solid var(--border);
  color: var(--muted); font-size: 12px; padding: 6px 20px;
}
.tab {
  background: transparent; color: var(--muted);
  border: 1px solid transparent; border-radius: 6px;
  padding: 7px 14px; cursor: pointer; font-size: 13px;
}
.tab:hover { color: var(--text); }
.tab.active {
  background: var(--panel2); color: var(--text);
  border-color: var(--border);
}
.tab-badge {
  background: var(--danger); color: #fff;
  border-radius: 8px; padding: 1px 6px; font-size: 11px; margin-left: 2px;
}
.search {
  flex: 0 0 220px;
  background: var(--bg); border: 1px solid var(--border);
  color: var(--text); border-radius: 6px; padding: 7px 12px;
  outline: none;
}
.search:focus { border-color: var(--accent); }
.sort-select {
  background: var(--bg); color: var(--text);
  border: 1px solid var(--border); border-radius: 6px;
  padding: 7px 8px; font-size: 12px; outline: none;
  flex-shrink: 0;
}
.sort-select:focus { border-color: var(--accent); }
.sort-dir {
  background: var(--panel2); color: var(--muted);
  border: 1px solid var(--border); border-radius: 6px;
  padding: 7px 10px; font-size: 12px; cursor: pointer; flex-shrink: 0;
}
.sort-dir:hover { color: var(--text); border-color: var(--accent); }
.count-badge {
  color: var(--muted); font-size: 12px; margin-left: auto; flex-shrink: 0;
}
.sys-badge {
  color: var(--muted); font-size: 12px; flex-shrink: 0;
  background: var(--panel2); border: 1px solid var(--border);
  border-radius: 14px; padding: 3px 12px;
}
.filters { display: flex; gap: 8px; flex-wrap: wrap; }
.filter-chip {
  background: var(--panel2); color: var(--text);
  border: 1px solid var(--border); border-radius: 14px;
  padding: 4px 12px; font-size: 12px; cursor: pointer;
}
.filter-chip.active { background: var(--accent); color: #fff; border-color: var(--accent); }

.layout { flex: 1; display: flex; overflow: hidden; }

.pkg-list { flex: 1; display: flex; flex-direction: column; overflow: hidden; }
.table-head, .row {
  display: grid;
  grid-template-columns: 32px 1fr 1.2fr 0.7fr 0.6fr 0.7fr 80px;
  gap: 8px; align-items: center;
  padding: 0 20px;
}
.pkg-list.advanced .table-head,
.pkg-list.advanced .row {
  grid-template-columns: 32px 1fr 1fr 0.6fr 0.55fr 0.7fr 0.6fr 80px;
}
.table-head {
  padding-top: 10px; padding-bottom: 8px;
  color: var(--muted); font-size: 12px;
  border-bottom: 1px solid var(--border);
}
.table-head .sortable { cursor: pointer; user-select: none; white-space: nowrap; }
.table-head .sortable:hover { color: var(--text); }
.list-body { flex: 1; overflow-y: auto; }
.row {
  height: 42px; cursor: pointer; border-bottom: 1px solid #1f232b;
  font-size: 13px;
}
.row:hover { background: var(--panel2); }
.col-icon { display: flex; align-items: center; justify-content: center; }
.app-icon { width: 24px; height: 24px; object-fit: contain; }
.col-name { display: flex; align-items: center; gap: 8px; min-width: 0; }
.name-txt {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}
.col-name small { color: var(--muted); flex-shrink: 0; }
.col-ver { overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
.col-time { overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
.essential {
  background: #3d3a1f; color: #e0c86a;
  font-size: 10px; padding: 1px 6px; border-radius: 8px;
}
.residue-badge {
  background: rgba(249, 115, 22, 0.15); color: #fb923c;
  font-size: 10px; padding: 1px 6px; border-radius: 8px;
}
.risk {
  font-size: 10px; padding: 1px 6px; border-radius: 8px;
  border: 1px solid;
}
.risk-low { background: rgba(74, 222, 128, 0.12); color: #4ade80; border-color: rgba(74,222,128,0.4); }
.risk-medium { background: rgba(250, 204, 21, 0.12); color: #facc15; border-color: rgba(250,204,21,0.4); }
.risk-high { background: rgba(251, 146, 60, 0.12); color: #fb923c; border-color: rgba(251,146,60,0.4); }
.risk-critical { background: rgba(239, 68, 68, 0.15); color: #f87171; border-color: rgba(239,68,68,0.5); }
.src-badge { font-weight: 600; font-size: 11px; }
.empty {
  padding: 40px; text-align: center; color: var(--muted);
}
.empty-sub { font-size: 12px; color: var(--muted); margin-top: 6px; }
.clean-check { font-size: 20px; }
.btn-uninstall {
  background: transparent; color: var(--danger);
  border: 1px solid var(--danger); border-radius: 5px;
  padding: 4px 14px; cursor: pointer; font-size: 12px;
}
.btn-uninstall:hover { background: var(--danger); color: #fff; }

.btn-ghost {
  background: var(--panel2); color: var(--text);
  border: 1px solid var(--border); border-radius: 6px;
  padding: 7px 14px; cursor: pointer; font-size: 13px;
}
.btn-ghost:hover { border-color: var(--accent); }
.btn-ghost.small { padding: 3px 10px; font-size: 12px; }
.btn-danger {
  background: var(--danger); color: #fff;
  border: none; border-radius: 6px;
  padding: 9px 24px; cursor: pointer; font-size: 14px; font-weight: 600;
}
.btn-danger:hover { filter: brightness(1.1); }
.btn-danger.small { padding: 6px 16px; font-size: 13px; }
.btn-danger:disabled { opacity: 0.5; cursor: wait; }

.detail {
  flex: 1; padding: 16px 24px; overflow-y: auto;
  border-left: 1px solid var(--border);
}
.detail-head { display: flex; align-items: center; gap: 16px; margin-bottom: 16px; }
.detail-head h2 { font-size: 17px; }
.btn-back {
  background: var(--panel2); color: var(--text);
  border: 1px solid var(--border); border-radius: 6px;
  padding: 6px 12px; cursor: pointer; font-size: 13px;
}
.warn-box {
  background: rgba(255, 82, 82, 0.12);
  border: 1px solid var(--danger); border-radius: 8px;
  padding: 12px 16px; margin-bottom: 16px;
}
.warn-item { color: #ffb4b4; font-size: 13px; margin: 2px 0; }
.risk-banner {
  background: rgba(239, 68, 68, 0.15);
  border: 1px solid #f87171; border-radius: 8px;
  padding: 12px 16px; margin-bottom: 16px;
  color: #ffd0d0; font-size: 13px;
}
.section-title {
  font-weight: 600; margin: 16px 0 8px; font-size: 13px; color: var(--muted);
}
.section-title small { font-weight: 400; color: var(--muted); }
.cmd-list, .file-list {
  background: var(--panel); border: 1px solid var(--border);
  border-radius: 8px; padding: 6px 12px;
}
.cmd-item {
  display: flex; align-items: center; gap: 10px;
  padding: 8px 0; border-bottom: 1px solid #2c3140;
}
.cmd-item:last-child { border-bottom: none; }
.cmd-desc { flex: 1; }
.cmd-item code {
  color: #7ec8ff; font-size: 12px;
  background: var(--bg); padding: 3px 8px; border-radius: 4px;
}
.root-badge {
  background: #5b3b1f; color: #ffb45b;
  font-size: 11px; padding: 2px 8px; border-radius: 8px;
}
.file-item {
  display: flex; align-items: center; gap: 10px;
  padding: 8px 0; cursor: pointer;
  border-bottom: 1px solid #2c3140;
}
.file-item:last-child { border-bottom: none; }
.file-item code { color: var(--muted); font-size: 12px; word-break: break-all; }
.file-cat {
  font-size: 10px; padding: 2px 8px; border-radius: 8px;
  background: var(--panel2); color: var(--muted); flex-shrink: 0;
}
.file-cat.config { color: #7ec8ff; }
.file-cat.cache { color: #ffd166; }
.file-cat.data { color: #9be29b; }
.file-cat.system { color: #ff8a8a; }
.file-cat.appimage { color: #f59e0b; }
.no-residue { color: #9be29b; padding: 10px; font-size: 13px; }
.orphan-box { margin-top: 12px; background: var(--panel); border: 1px solid var(--border); border-radius: 8px; padding: 4px 12px; }
.detail-actions { margin-top: 20px; display: flex; gap: 12px; }
.result-box {
  margin-top: 16px; background: var(--panel); border: 1px solid var(--border);
  border-radius: 8px; padding: 12px 16px;
}
.result-box .ok { color: #9be29b; }
.result-box .fail { color: #ffb4b4; }
.res-line { font-size: 12px; margin-top: 4px; }
.ok-line { color: #9be29b; }
.fail-line { color: #ffb4b4; }

.orphan-panel {
  position: fixed; right: 16px; bottom: 16px;
  width: 380px; max-height: 60vh; overflow-y: auto;
  background: var(--panel); border: 1px solid var(--border);
  border-radius: 10px; padding: 14px 16px;
  box-shadow: 0 8px 30px rgba(0,0,0,0.5);
  z-index: 50;
}
.orphan-head { display: flex; justify-content: space-between; align-items: center; margin-bottom: 8px; }
.orphan-note { font-size: 12px; color: var(--muted); margin-bottom: 8px; }
.orphan-tags { display: flex; flex-wrap: wrap; gap: 6px; margin-bottom: 12px; }
.orphan-tag {
  background: var(--panel2); border: 1px solid var(--border);
  border-radius: 6px; padding: 3px 8px; font-size: 11px;
  font-family: monospace;
}

.toast {
  position: fixed; top: 16px; left: 50%; transform: translateX(-50%);
  padding: 10px 22px; border-radius: 8px;
  font-size: 14px; font-weight: 600;
  background: #14532d; color: #dcfce7;
  border: 1px solid #16a34a;
  box-shadow: 0 6px 24px rgba(0,0,0,0.5);
  z-index: 100;
}
.toast.fail {
  background: #450a0a; color: #fecaca;
  border-color: #ef4444;
}
.toast-enter-active, .toast-leave-active { transition: all 0.25s ease; }
.toast-enter-from, .toast-leave-to { opacity: 0; transform: translateX(-50%) translateY(-8px); }
</style>
