<script setup lang="ts">
import { ref, onMounted, watch } from "vue";
import { 
  LayoutDashboard, 
  Users, 
  Sun, 
  Moon, 
  ZapOff,
  Plus,
  Trash2,
  Activity,
  Globe,
  RefreshCw,
  CheckCircle2,
  AlertCircle,
  Download,
  Info,
  Minus,
  Maximize2,
  Minimize2,
  X,
  Puzzle
} from "lucide-vue-next";
import { useAppStore } from "./stores/app";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

const store = useAppStore();
const activeTab = ref('dashboard');
const detectedProxy = ref('Checking...');

// Syncing from backend
const syncData = async () => {
  try {
    // 1. Sync active status with real Antigravity DB first
    await invoke("reconcile_active_session").catch(e => console.warn(e));

    const accs = await invoke("sync_vault_entries");
    store.accounts = accs as any;
    
    const config = await invoke("load_booster_settings");
    store.proxyConfig = config as any;
    
    detectedProxy.value = await invoke("analyze_network_gate");
    
    // Check proxy status
    try {
      isProxyEnabled.value = await invoke("is_proxy_enabled");
    } catch (e) {
      console.warn("Failed to check proxy status:", e);
    }
  } catch (err) {
    store.addLog(`Sync error: ${err}`);
  }
};

const isProxyEnabled = ref(false);
const isTogglingProxy = ref(false);

const toggleProxy = async () => {
  if (isTogglingProxy.value) return; // Prevent double-click
  
  isTogglingProxy.value = true;
  try {
    if (isProxyEnabled.value) {
      store.addLog("正在禁用系统代理...");
      const result = await invoke<string>("disable_system_proxy");
      store.addLog("已禁用系统代理");
      console.log("Disable proxy result:", result);
      isProxyEnabled.value = false;
    } else {
      store.addLog("正在启用系统代理...");
      const result = await invoke<string>("enable_system_proxy");
      store.addLog("已启用系统代理");
      console.log("Enable proxy result:", result);
      isProxyEnabled.value = true;
    }
    // Refresh status after operation
    await syncData();
  } catch (err) {
    console.error("Proxy toggle error:", err);
    store.addLog(`代理切换失败: ${err}`);
    // Don't change the state if operation failed
  } finally {
    isTogglingProxy.value = false;
  }
};

const refreshQuota = async (id: string) => {
  try {
    store.addLog("Refreshing account usage...");
    await invoke("pulse_check_quota", { id });
    await syncData();
    store.addLog("Usage data updated.");
  } catch (err) {
    store.addLog(`Quota error: ${err}`);
  }
};

const chartData = ref<any>(null);
const chartTimeRange = ref<number>(1); // 1, 3, or 7 days
const isLoadingChart = ref(false);
const chartKey = ref(0); // 用于触发动画

const loadChartData = async () => {
  isLoadingChart.value = true;
  try {
    const displayMinutes = chartTimeRange.value * 24 * 60;
    const bucketMinutes = chartTimeRange.value === 1 ? 30 : (chartTimeRange.value === 3 ? 60 : 120);
    const newData = await invoke("get_usage_chart", { displayMinutes, bucketMinutes });
    
    // 性能优化 & 视觉体验：比对数据是否真正发生变化，避免相同数据重复触发动画
    const isDifferent = JSON.stringify(newData) !== JSON.stringify(chartData.value);
    chartData.value = newData;
    
    if (isDifferent) {
      chartKey.value++; // 只有数据变了才触发动画
    }
  } catch (err) {
    console.error("Failed to load chart data:", err);
  } finally {
    isLoadingChart.value = false;
  }
};

watch(chartTimeRange, () => {
  loadChartData();
});

// 格式化时间段显示
const formatBucketTooltip = (bucket: any) => {
  const startDate = new Date(bucket.start_time * 1000);
  const endDate = new Date(bucket.end_time * 1000);
  
  const formatTime = (date: Date) => {
    const month = String(date.getMonth() + 1).padStart(2, '0');
    const day = String(date.getDate()).padStart(2, '0');
    const hours = String(date.getHours()).padStart(2, '0');
    const minutes = String(date.getMinutes()).padStart(2, '0');
    return `${month}-${day} ${hours}:${minutes}`;
  };
  
  const timeRange = `${formatTime(startDate)} ~ ${formatTime(endDate)}`;
  const usageInfo = bucket.items.map((item: any) => 
    `${item.account_name} - ${item.model_name}: ${item.usage.toFixed(1)}%`
  ).join('\n');
  
  return `${timeRange}\n${usageInfo}`;
};

// 样式计算辅助函数
const calculateBarHeight = (items: any[], maxUsage: number): string => {
  const total = items.reduce((sum: number, item: any) => sum + item.usage, 0);
  // Ensure maxUsage is at least 1 to avoid division by zero (though backend handles this)
  const safeMax = Math.max(1, maxUsage); 
  const px = (total / safeMax) * 160;
  // Min 3px for visibility, Max 160px container height
  return Math.min(160, Math.max(3, px)) + 'px';
};

const calculateBarGradient = (items: any[]): string => {
  if (items.length === 0) return 'transparent';
  if (items.length === 1) return items[0].color;
  
  const stops = items.map((item: any, i: number) => {
    const prevHeight = items.slice(0, i).reduce((sum: number, it: any) => sum + it.usage, 0);
    const currHeight = prevHeight + item.usage;
    const totalHeight = items.reduce((sum: number, it: any) => sum + it.usage, 0);
    
    // Avoid division by zero
    if (totalHeight === 0) return `${item.color} 0% 0%`;

    const startPct = (prevHeight / totalHeight * 100).toFixed(1);
    const endPct = (currHeight / totalHeight * 100).toFixed(1);
    return `${item.color} ${startPct}% ${endPct}%`;
  }).join(', ');

  return `linear-gradient(to top, ${stops})`;
};


import { listen } from '@tauri-apps/api/event';

onMounted(async () => {
  await syncData();
  await loadChartData();
  
  // Listen for backend debug logs
  listen('debug-log', (event: any) => {
     const raw = event.payload as string;
     if (raw && raw.length < 500) {
        store.addLog(`[DEBUG] ${raw.replace(/\n/g, ' ')}`);
     } else {
        store.addLog(`[DEBUG] Log received (${raw.length} chars)`);
     }
  });
  
  // Listen for quota updates from auto-refresh
  listen('quota-updated', async () => {
    await syncData();
    await loadChartData();
  });
});

// Actions
const isSwitchingAccount = ref(false);
const switchingAccountId = ref<string | null>(null);

const switchAccount = async (id: string) => {
  if (isSwitchingAccount.value) return;
  
  isSwitchingAccount.value = true;
  switchingAccountId.value = id;
  
  try {
    store.addLog("正在切换账号...");
    const res = await invoke("switch_account", { id });
    await syncData();
    // Auto refresh usage on switch (Force await)
    await refreshQuota(id);
    store.addLog(`已登录账户: ${store.activeAccount?.name}`);
    if (res) {
      store.addLog(`同步日志: ${res}`);
    }
  } catch (err) {
    store.addLog(`登录失败: ${err}`);
  } finally {
    isSwitchingAccount.value = false;
    switchingAccountId.value = null;
  }
};



// Custom confirmation dialog state
const showDeleteConfirm = ref(false);
const deleteConfirmData = ref<{ id: string; name: string } | null>(null);

const removeAccount = async (id: string, name?: string) => {
  deleteConfirmData.value = { id, name: name || '该账号' };
  showDeleteConfirm.value = true;
};

const confirmDelete = async () => {
  if (!deleteConfirmData.value) return;
  
  const { id, name } = deleteConfirmData.value;
  showDeleteConfirm.value = false;
  
  try {
    await invoke("delete_account", { id });
    await syncData();
    store.addLog(`已移除账户: ${name}`);
  } catch (err) {
    store.addLog(`移除失败: ${err}`);
  }
  
  deleteConfirmData.value = null;
};

const cancelDelete = () => {
  showDeleteConfirm.value = false;
  deleteConfirmData.value = null;
};

// --- Extension Logic ---
interface ExtensionInfo {
  installed_version: string | null;
  latest_version: string;
  status: string;
}

const extStatus = ref("unknown"); // not_installed, outdated, installed
const extInfo = ref<ExtensionInfo | null>(null);
const isInstallingExt = ref(false);

const checkExtensionStatus = async () => {
    try {
        const status = await invoke<ExtensionInfo>("get_extension_status");
        extInfo.value = status;
        extStatus.value = status.status;
    } catch (e) {
        console.error("Ext check failed", e);
    }
}

// Actions
const showRestartConfirm = ref(false);
const restartConfirmType = ref('manual'); // manual, after_install

const triggerRestart = () => {
    restartConfirmType.value = 'manual';
    showRestartConfirm.value = true;
}

const confirmRestart = async () => {
    showRestartConfirm.value = false;
    try {
        await invoke("restart_antigravity");
        store.addLog("已发送重启命令");
    } catch(e) {
        store.addLog(`重启失败: ${e}`);
    }
}

const installExtension = async () => {
    isInstallingExt.value = true;
    try {
        const msg = await invoke("install_assistant_extension");
        store.addLog(`插件安装: ${msg}`);
        await checkExtensionStatus();
        
        // Show custom restart modal
        restartConfirmType.value = 'after_install';
        showRestartConfirm.value = true;
    } catch (e) {
        store.addLog(`插件安装失败: ${e}`);
    } finally {
        isInstallingExt.value = false;
    }
}

// Check on mount and periodically
onMounted(async () => {
    checkExtensionStatus();
    setInterval(checkExtensionStatus, 10000); // Check every 10s
});

const isLoggingIn = ref(false);
const handleAddAccount = async () => {
  if (isLoggingIn.value) return;
  isLoggingIn.value = true;
  
  try {
    store.addLog("启动 OAuth 登录流程...");
    const acc = await invoke("start_oauth_login");
    await syncData();
    // Force refresh quota for the new account
    if (acc && (acc as any).id) {
       await refreshQuota((acc as any).id);
    }
    store.addLog(`登录成功: ${(acc as any).name}`);
    activeTab.value = 'accounts'; // Auto switch back
  } catch (err) {
    store.addLog(`登录失败: ${err}`);
  } finally {
    isLoggingIn.value = false;
  }
};

const handleExportBackup = async () => {
  try {
    await invoke("export_backup");
    store.addLog("备份已导出");
  } catch (err) {
    store.addLog(`导出取消或失败: ${err}`);
  }
};

const handleImportBackup = async () => {
  try {
    const imported = await invoke("import_backup");
    if ((imported as any[])?.length > 0) {
      await syncData();
      store.addLog(`从备份恢复了 ${(imported as any[]).length} 个账号`);
      
      // 先切换到账号页面，让用户看到导入的列表
      activeTab.value = 'accounts';
      
      // 然后开始刷新，用户能看到界面上的刷新动画
      store.addLog("正在刷新所有账号配额...");
      await refreshAllQuotas();
    }
  } catch (err) {
    store.addLog(`导入取消或失败: ${err}`);
  }
};

const isRefreshingActive = ref(false);
const handleRefreshActive = async () => {
    if (!store.activeAccount || isRefreshingActive.value) return;
    isRefreshingActive.value = true;
    await refreshQuota(store.activeAccount.id);
    isRefreshingActive.value = false;
};

const isRefreshingAll = ref(false);
const refreshAllQuotas = async () => {
  if (isRefreshingAll.value) return;
  isRefreshingAll.value = true;
  
  store.addLog("正在批量刷新 (顺序执行)...");
  
  for (const acc of store.accounts) {
    try {
      store.addLog(`[${acc.name}] 正在获取数据...`);
      await invoke("pulse_check_quota", { id: acc.id });
    } catch (e) {
      store.addLog(`Error ${acc.name}: ${e}`);
    }
  }

  await syncData();
  store.addLog("批量刷新完成。");
  isRefreshingAll.value = false;
};

// 窗口控制
const appWindow = getCurrentWindow();
const isMaximized = ref(false);

const minimizeWindow = async () => {
  await appWindow.minimize();
};

const toggleMaximize = async () => {
  if (isMaximized.value) {
    await appWindow.unmaximize();
  } else {
    await appWindow.maximize();
  }
  // 更新状态
  isMaximized.value = await appWindow.isMaximized();
};

const closeWindow = async () => {
  await appWindow.close();
};

// 监听窗口最大化状态变化
onMounted(async () => {
  // 初始化状态
  isMaximized.value = await appWindow.isMaximized();
  
  // 监听窗口大小变化
  await appWindow.onResized(async () => {
    isMaximized.value = await appWindow.isMaximized();
  });
});
const showLogModal = ref(false);

const isImportingFromEditor = ref(false);
const handleImportFromAntigravity = async () => {
    if (isImportingFromEditor.value) return;
    isImportingFromEditor.value = true;
    try {
        store.addLog("尝试从编辑器导入账号...");
        const acc = await invoke("import_account_from_antigravity");
        await syncData();
        if (acc && (acc as any).id) {
            await refreshQuota((acc as any).id);
        }
        store.addLog(`导入成功: ${(acc as any).name}`);
        activeTab.value = 'accounts';
    } catch (e) {
        store.addLog(`导入失败: ${e}`);
    } finally {
        isImportingFromEditor.value = false;
    }
}
</script>

<template>
  <!-- 窗口控制按钮 - 悬浮在右上角 -->
  <div class="window-controls">
    <button class="window-button" @click="minimizeWindow" title="最小化">
      <Minus :size="16" />
    </button>
    <button class="window-button" @click="toggleMaximize" :title="isMaximized ? '还原' : '最大化'">
      <component :is="isMaximized ? Minimize2 : Maximize2" :size="16" />
    </button>
    <button class="window-button window-close" @click="closeWindow" title="关闭">
      <X :size="16" />
    </button>
  </div>

  <!-- Sidebar -->
  <aside class="sidebar">
    <div class="brand">
      <h1>Antigravity Booster</h1>
    </div>

    <nav class="nav-menu">
      <div class="nav-item" :class="{ active: activeTab === 'dashboard' }" @click="activeTab = 'dashboard'">
        <LayoutDashboard /> 仪表盘
      </div>
      <div class="nav-item" :class="{ active: activeTab === 'accounts' }" @click="activeTab = 'accounts'">
        <Users /> 账号管理
      </div>
      <div class="nav-item" :class="{ active: activeTab === 'extension' }" @click="activeTab = 'extension'">
        <Puzzle /> 编辑器插件
      </div>
      <div class="nav-item" :class="{ active: activeTab === 'settings' }" @click="activeTab = 'settings'">
        <Info /> 关于
      </div>
    </nav>

    <div class="bottom-actions">
      <!-- Active Account Quick View -->
      <div v-if="store.activeAccount" class="card" style="padding: 0.75rem; background: var(--surface-hover); border: none; margin-bottom: 0.5rem;">
        <div style="display: flex; align-items: center; gap: 0.5rem;">
          <CheckCircle2 :size="16" color="var(--success)" />
          <div style="font-size: 0.8125rem; font-weight: 700; overflow: hidden; white-space: nowrap; text-overflow: ellipsis;">
            {{ store.activeAccount.name }}
          </div>
        </div>
      </div>

      <div class="card" style="padding: 1rem; border: 1px dashed var(--accent);">
        <div style="display: flex; align-items: center; justify-content: space-between; margin-bottom: 0.75rem;">
          <span style="font-size: 0.7rem; font-weight: 800; color: var(--text-secondary);">Antigravity 代理</span>
          <span class="badge" :class="isTogglingProxy ? 'badge-warning' : (isProxyEnabled ? 'badge-success' : 'badge-error')">
            {{ isTogglingProxy ? '处理中' : (isProxyEnabled ? '已启用' : '未启用') }}
          </span>
        </div>
        <button 
          class="btn btn-primary" 
          style="width: 100%;" 
          @click="toggleProxy"
          :disabled="isTogglingProxy">
          <component :is="isTogglingProxy ? RefreshCw : (isProxyEnabled ? ZapOff : Globe)" :size="18" :class="{ 'spin': isTogglingProxy }" />
          {{ isTogglingProxy ? '请稍候...' : (isProxyEnabled ? '禁用代理' : '启用代理') }}
        </button>
      </div>

      
      
      <button class="btn btn-ghost" @click="store.toggleTheme">
        <component :is="store.theme === 'light' ? Moon : Sun" :size="20" />
        {{ store.theme === 'light' ? '深色模式' : '浅色模式' }}
      </button>
    </div>
  </aside>

  <!-- Main Content -->
  <main class="main-area">
    <!-- Header removed for cleaner UI -->

    <div class="content-body">
      <!-- Dashboard -->
      <div v-if="activeTab === 'dashboard'" class="animate-fade-in">
        <div style="display: flex; justify-content: space-between; align-items: baseline; margin-bottom: 2rem;">
          <div>
            <h2 style="margin-bottom: 0.25rem;">资源配额</h2>
            <p style="color: var(--text-secondary); font-size: 0.875rem;">实时监控性能状态与配额消耗。</p>
          </div>
          <button v-if="store.activeAccount" 
                  class="btn btn-ghost" 
                  style="font-size: 0.75rem; border: 1px solid var(--border);" 
                  :disabled="isRefreshingActive"
                  @click="handleRefreshActive">
            <RefreshCw :size="14" style="margin-right: 0.25rem;" :class="{ 'spin': isRefreshingActive }" /> 刷新用量
          </button>
        </div>
        
        <div class="grid" style="grid-template-columns: 1.5fr 1fr;">
          <!-- Quota monitor -->
          <div class="card">
            <div style="display: flex; align-items: center; gap: 0.75rem; margin-bottom: 1.5rem;">
              <Activity :size="20" color="var(--accent)" />
              <div style="display: flex; align-items: center; gap: 0.5rem;">
                <h3 style="font-size: 1rem; margin: 0;">{{ store.activeAccount ? store.activeAccount.name : '资源分配情况' }}</h3>
                <span v-if="store.activeAccount?.account_type && !['gemini', 'free', 'standard'].includes(store.activeAccount.account_type.toLowerCase())" 
                      class="badge-capsule">
                  {{ store.activeAccount.account_type }}
                </span>
              </div>
            </div>


            
            <div v-if="store.activeAccount" style="margin-top: 0.5rem;">
              <div v-for="q in store.activeAccount.quota?.models" :key="q.name" style="margin-bottom: 1.5rem;">
                <div style="display: flex; justify-content: space-between; font-size: 0.8125rem; margin-bottom: 0.6rem;">
                  <span style="font-weight: 700; color: var(--text-primary); text-transform: uppercase;">{{ q.name }}</span>
                  <span :style="{ color: q.percentage < 20 ? 'var(--error)' : 'var(--accent)' }" style="font-weight: 800;">
                    {{ q.percentage }}%
                  </span>
                </div>
                <!-- Only show progress bar if it's a model with percentage -->
                <div v-if="q.name.indexOf('等级') === -1" style="height: 6px; width: 100%; background: var(--surface-hover); border-radius: 3px; overflow: hidden;">
                  <div :style="{ width: q.percentage + '%', background: q.percentage < 20 ? 'var(--error)' : 'var(--accent)' }" 
                    style="height: 100%; transition: width 1.5s cubic-bezier(0.16, 1, 0.3, 1), background 0.3s;"></div>
                </div>
                <div style="display: flex; justify-content: space-between; font-size: 0.7rem; color: var(--text-secondary); margin-top: 0.5rem;">
                  <span>重置时间</span>
                  <span>{{ q.reset_time.replace(/(\d+)\s*小时/, (m, hStr) => {
                      const h = parseInt(hStr);
                      if (h >= 24) {
                          const days = Math.floor(h / 24);
                          const remH = h % 24;
                          return remH > 0 ? `${days}天${remH}小时` : `${days}天`;
                      }
                      return m;
                  }) }}</span>
                </div>
              </div>
            </div>
            <div v-else style="padding: 2rem 1rem;">
              <!-- If there are accounts but none active, show list to switch -->
              <div v-if="store.accounts.length > 0">
                <div style="text-align: center; margin-bottom: 1.5rem;">
                  <Users :size="32" color="var(--border)" style="margin-bottom: 0.5rem;" />
                  <p style="color: var(--text-secondary); font-size: 0.875rem;">选择一个账号登录Antigravity以查看配额</p>
                </div>
                <div style="display: flex; flex-direction: column; gap: 0.75rem;">
                  <div v-for="acc in store.accounts" :key="acc.id" 
                       class="card" 
                       style="padding: 1rem; display: flex; align-items: center; justify-content: space-between;">
                    <div style="display: flex; align-items: center; gap: 0.5rem;">
                      <span style="font-weight: 700; font-size: 0.875rem;">{{ acc.name }}</span>
                      <span v-if="acc.account_type && !['gemini', 'free', 'standard'].includes(acc.account_type.toLowerCase())" 
                            class="badge-capsule" 
                            style="font-size: 0.6rem; padding: 0.15rem 0.5rem;">
                        {{ acc.account_type }}
                      </span>
                    </div>
                    <button 
                      class="btn btn-primary" 
                      style="font-size: 0.7rem; padding: 0.4rem 0.8rem; display: flex; align-items: center; gap: 0.25rem;" 
                      @click="switchAccount(acc.id)"
                      :disabled="isSwitchingAccount && switchingAccountId === acc.id"
                    >
                      <RefreshCw v-if="isSwitchingAccount && switchingAccountId === acc.id" :size="12" class="spin" />
                      {{ isSwitchingAccount && switchingAccountId === acc.id ? '切换中...' : '登录' }}
                    </button>
                  </div>
                </div>
              </div>
              <!-- If no accounts at all, show login prompt -->
              <div v-else style="text-align: center;">
                <Users :size="32" color="var(--border)" style="margin-bottom: 1rem;" />
                <p style="color: var(--text-secondary); font-size: 0.875rem;">添加 Google 账号以开始使用</p>
                <button class="btn btn-primary" style="margin-top: 1.5rem; font-size: 0.75rem;" @click="activeTab = 'add_account'">添加账号</button>
              </div>
            </div>
          </div>

          <!-- System Proxy Status (Display Only) -->
          <div class="card" style="display: flex; flex-direction: column; justify-content: center; align-items: center; border-color: var(--accent);">
            <div class="pulse-container" :class="{ active: isProxyEnabled }">
              <Globe :size="48" :color="isProxyEnabled ? 'var(--accent)' : 'var(--text-secondary)'" style="position: relative; z-index: 2;" />
              <div class="pulse-ring"></div>
              <div class="pulse-ring" style="animation-delay: 1s"></div>
            </div>
            <div style="margin-top: 2rem; text-align: center; width: 100%; padding: 0 1rem;">
              <p style="font-size: 1.125rem; color: var(--text-primary); margin-bottom: 0; font-weight: 600; line-height: 1.6;">
                {{ isProxyEnabled ? 'Antigravity 正在遵循系统代理' : '左下角启动代理可以让 Antigravity 遵循系统代理' }}
              </p>
              <p v-if="detectedProxy" style="font-size: 0.65rem; color: var(--text-secondary); margin-top: 1rem; padding-top: 1rem; border-top: 1px solid var(--border);">
                检测到: {{ detectedProxy }}
              </p>
            </div>
          </div>
        </div>

        <!-- Usage History Chart -->
        <div class="card" style="margin-top: 2rem;">
          <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 1.5rem;">
            <h3 style="font-size: 1rem;">
              <Activity :size="18" style="vertical-align: sub; margin-right: 0.5rem;" /> 历史用量
            </h3>
            <!-- Time range selector -->
            <div style="display: flex; gap: 0.25rem; background: var(--surface-hover); padding: 0.25rem; border-radius: 0.5rem;">
              <button v-for="days in [1, 3, 7]" :key="days"
                      @click="chartTimeRange = days"
                      :class="chartTimeRange === days ? 'btn-primary' : 'btn-ghost'"
                      style="padding: 0.25rem 0.75rem; font-size: 0.7rem; border-radius: 0.375rem;">
                {{ days }}天
              </button>
            </div>
          </div>
          
          <div v-if="chartData && chartData.buckets.length > 0" style="padding: 1rem 0;">
            <div :key="chartKey" class="chart-fade-in">
              <!-- Chart bars -->
              <div style="display: flex; align-items: flex-end; gap: 2px; height: 180px; margin-bottom: 1rem;">
                <div v-for="(bucket, idx) in chartData.buckets" :key="idx"
                     style="flex: 1; display: flex; flex-direction: column; justify-content: flex-end; position: relative;">
                  <!-- Stacked bar -->
                  <div v-if="bucket.items.length > 0"
                       class="bar-animate"
                       :style="{
                         height: calculateBarHeight(bucket.items, chartData.max_usage),
                         animationDelay: (idx * 0.01) + 's',
                         background: calculateBarGradient(bucket.items),
                         borderRadius: '2px 2px 0 0',
                         transition: 'height 0.3s',
                         cursor: 'pointer'
                       }"
                       :title="formatBucketTooltip(bucket)">
                  </div>
                  <div v-else style="height: 3px; background: rgba(255,255,255,0.05); border-radius: 2px;"></div>
                </div>
              </div>
              
              <!-- Legend -->
              <div style="display: flex; justify-content: space-between; align-items: center; font-size: 0.7rem; color: var(--text-secondary);">
                <span>最近 {{ chartData.display_minutes / 60 }} 小时 · {{ chartData.interval }}分/柱</span>
                <span style="font-size: 0.65rem;">仅在软件运行时记录 · 每 5 分钟自动刷新</span>
              </div>
            </div>
          </div>
          
          <div v-else style="text-align: center; padding: 3rem; color: var(--text-secondary); font-size: 0.875rem;">
            <Activity :size="32" color="var(--border)" style="margin-bottom: 0.5rem;" />
            <p>暂无历史数据</p>
            <p style="font-size: 0.7rem; margin-top: 0.5rem;">软件运行时将自动记录用量</p>
          </div>
        </div>
      </div>

      <!-- Extension Management -->
      <div v-if="activeTab === 'extension'" class="animate-fade-in" style="height: calc(100vh - 70px - 2rem); display: flex; align-items: center; justify-content: center; margin-top: -20px;">
        <div style="width: 100%; max-width: 700px; text-align: center;">
          <div style="margin-bottom: 2.5rem;">
            <div style="width: 120px; height: 120px; background: var(--surface-hover); border-radius: 28px; display: flex; align-items: center; justify-content: center; margin: 0 auto; box-shadow: 0 15px 35px rgba(0,0,0,0.15); position: relative;">
                <Puzzle :size="60" color="var(--accent)" />
            </div>
          </div>
          <h1 style="font-size: 2.25rem; font-weight: 800; margin-bottom: 0.75rem;">Antigravity Booster Helper</h1>
          <p style="color: var(--text-secondary); font-size: 1.125rem; max-width: 500px; margin: 0 auto; line-height: 1.6;">
            {{ extStatus === 'installed' ? '目前插件状态良好，正在为您保驾护航。' : (extStatus === 'outdated' ? '检测到新版本，建议立即升级以获得最佳体验。' : '安装插件以解锁完整功能。') }}
          </p>
          
          <div style="display: flex; flex-direction: column; gap: 1rem; align-items: center; margin-top: 2.5rem; margin-bottom: 3rem;">
              <div v-if="extStatus === 'installed'" class="badge badge-success" style="padding: 0.6rem 1.25rem; font-size: 0.9rem; border-radius: 2rem;">
                  已安装 v{{ extInfo?.installed_version }}
              </div>
              <div v-else-if="extStatus === 'outdated'" class="badge badge-info" style="padding: 0.6rem 1.25rem; font-size: 0.9rem; border-radius: 2rem;">
                  有更新: v{{ extInfo?.installed_version }} → v{{ extInfo?.latest_version }}
              </div>
              <div v-else class="badge badge-warning" style="padding: 0.6rem 1.25rem; font-size: 0.9rem; border-radius: 2rem;">
                  未检测到安装
              </div>
          </div>

          <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 2rem; margin-bottom: 4rem;">
              <div style="text-align: left; padding: 1.5rem; background: var(--surface-hover); border-radius: 1rem;">
                  <div style="display: flex; align-items: center; gap: 0.75rem; margin-bottom: 0.75rem;">
                    <Activity :size="20" color="var(--success)" />
                    <span style="font-weight: 700; font-size: 1rem;">状态同步</span>
                  </div>
                  <div style="font-size: 0.875rem; opacity: 0.8; line-height: 1.6;">在 Antigravity 状态栏实时显示当前账号的配额余量。</div>
              </div>
              <div style="text-align: left; padding: 1.5rem; background: var(--surface-hover); border-radius: 1rem;">
                  <div style="display: flex; align-items: center; gap: 0.75rem; margin-bottom: 0.75rem;">
                    <ZapOff :size="20" color="var(--accent)" />
                    <span style="font-weight: 700; font-size: 1rem;">自动同意</span>
                  </div>
                  <div style="font-size: 0.875rem; opacity: 0.8; line-height: 1.6;">无需手动点击，自动接受 Agent 发起的修改建议与终端请求。</div>
              </div>
          </div>

          <div style="display: flex; gap: 1.5rem; justify-content: center;">
              <button 
                class="btn btn-primary" 
                style="padding: 1rem 3rem; border-radius: 0.875rem; font-weight: 600; min-width: 180px; font-size: 1rem;" 
                @click="installExtension"
                :disabled="isInstallingExt">
                <RefreshCw v-if="isInstallingExt" :size="20" class="spin" style="margin-right: 0.75rem;" />
                {{ extStatus === 'installed' ? '重新安装' : (extStatus === 'outdated' ? '立即更新' : '安装插件') }}
              </button>
              <button v-if="extStatus === 'installed'" class="btn btn-ghost" style="border: 1px solid var(--border); padding: 1rem 3rem; border-radius: 0.875rem; font-size: 1rem;" @click="triggerRestart">
                <RefreshCw :size="20" style="margin-right: 0.75rem;" /> 重启软件
              </button>
          </div>
        </div>
      </div>

      <!-- Accounts -->
      <div v-if="activeTab === 'accounts'" class="animate-fade-in">
        <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 2rem;">
          <h2>账号管理</h2>
          <div style="display: flex; gap: 0.5rem;">
            <button class="btn btn-ghost" 
                style="border: 1px solid var(--border);" 
                :disabled="isRefreshingAll"
                @click="refreshAllQuotas">
               <RefreshCw :size="16" style="margin-right: 0.25rem;" :class="{ 'spin': isRefreshingAll }" /> 刷新全部
            </button>
            <button class="btn btn-ghost" 
                style="border: 1px solid var(--border);" 
                @click="handleExportBackup">
               <Download :size="16" style="margin-right: 0.25rem;" /> 导出数据
            </button>
            <button class="btn btn-primary" @click="activeTab = 'add_account'">
              <Plus :size="18" /> 添加账号
            </button>
          </div>
        </div>

        <div class="grid">
          <div v-for="acc in store.accounts" :key="acc.id" class="card" 
            :style="{ borderColor: acc.is_active ? 'var(--accent)' : 'var(--border)' }">
            <div style="display: flex; align-items: flex-start; justify-content: space-between; margin-bottom: 1rem;">
              <div>
                <div style="display: flex; align-items: center; gap: 0.5rem; margin-bottom: 0.5rem;">
                  <h3 style="font-size: 1rem; font-weight: 700;">{{ acc.name }}</h3>
                  <CheckCircle2 v-if="acc.is_active" :size="16" color="var(--accent)" />
                  
                  <!-- Capsule Badge: Inline & Only for Premium -->
                  <span v-if="acc.account_type && !['gemini', 'free', 'standard'].includes(acc.account_type.toLowerCase())" 
                        class="badge-capsule" 
                        style="margin-left: 0.5rem;">
                    {{ acc.account_type }}
                  </span>
                </div>
              </div>
              <div style="display: flex; gap: 0.5rem;">
                <button class="btn btn-ghost" style="padding: 0.4rem;" @click="removeAccount(acc.id, acc.name)">
                  <Trash2 :size="16" color="var(--error)" />
                </button>
              </div>
            </div>

            <!-- Detailed Quota View (Like Dashboard) -->
            <div v-if="acc.quota" style="margin: 1.25rem 0; display: flex; flex-direction: column; gap: 1rem;">
              <div v-for="m in acc.quota.models" :key="m.name">
                <div style="display: flex; justify-content: space-between; font-size: 0.75rem; margin-bottom: 0.4rem;">
                  <span style="font-weight: 700; color: var(--text-primary); text-transform: uppercase;">{{ m.name }}</span>
                  <span :style="{ color: m.percentage < 20 ? 'var(--error)' : 'var(--accent)' }" style="font-weight: 800;">
                    {{ m.percentage }}%
                  </span>
                </div>
                <div style="height: 5px; width: 100%; background: var(--surface-hover); border-radius: 3px; overflow: hidden;">
                   <div :style="{ width: m.percentage + '%', background: (m.percentage < 20) ? 'var(--error)' : 'var(--accent)' }" style="height: 100%; transition: width 0.5s;"></div>
                </div>
                <div style="display: flex; justify-content: space-between; font-size: 0.65rem; color: var(--text-secondary); margin-top: 0.4rem;">
                  <span>重置时间</span>
                  <span>{{ m.reset_time.replace(/(\d+)\s*小时/, (m, hStr) => {
                      const h = parseInt(hStr);
                      if (h >= 24) {
                          const days = Math.floor(h / 24);
                          const remH = h % 24;
                          return remH > 0 ? `${days}天${remH}小时` : `${days}天`;
                      }
                      return m;
                  }) }}</span>
                </div>
              </div>
            </div>
            
            <!-- Fallback if no quota -->
            <div v-else style="margin: 1.25rem 0; padding: 1rem; background: var(--surface-hover); border-radius: 0.5rem; text-align: center;">
                <p style="font-size: 0.75rem; color: var(--text-secondary);">暂无数据，请尝试刷新</p>
            </div>

            <button 
              class="btn" 
              :class="acc.is_active ? 'btn-ghost' : 'btn-primary'" 
              style="width: 100%; font-size: 0.75rem; display: flex; align-items: center; justify-content: center; gap: 0.5rem;"
              @click="switchAccount(acc.id)"
              :disabled="acc.is_active || (isSwitchingAccount && switchingAccountId === acc.id)"
            >
              <RefreshCw v-if="isSwitchingAccount && switchingAccountId === acc.id" :size="14" class="spin" />
              {{ 
                isSwitchingAccount && switchingAccountId === acc.id 
                  ? '切换中...' 
                  : acc.is_active 
                    ? '当前正在使用' 
                    : '切换至此账号' 
              }}
            </button>
          </div>
        </div>
      </div>

      <!-- About Page -->
      <div v-if="activeTab === 'settings'" class="animate-fade-in" style="height: calc(100vh - 70px - 2rem); display: flex; align-items: center; justify-content: center; margin-top: -20px;">
        <div style="width: 100%; max-width: 700px; text-align: center;">
            <div style="margin-bottom: 3.5rem;">
              <img src="./assets/logo.png" style="width: 120px; height: 120px; border-radius: 28px; box-shadow: 0 15px 35px rgba(0,0,0,0.15); object-fit: cover; margin: 0 auto;" />
            </div>
            <h1 style="font-size: 3rem; font-weight: 800; margin-bottom: 0.5rem; color: var(--text-primary); letter-spacing: -0.04em;">Antigravity Booster</h1>
            <p style="color: var(--text-secondary); font-size: 1rem; margin-bottom: 3rem; font-family: 'JetBrains Mono', monospace; letter-spacing: 1px;">Version 1.4.0 (Build 20260128)</p>
            
            <div style="max-width: 600px; margin: 0 auto 3.5rem; line-height: 2; color: var(--text-secondary); font-size: 1.125rem;">
                Antigravity Booster 是专门为您打造的效能增强助手。<br/>不仅解决了复杂的网络代理问题，更提供了优雅的多账号管理体验。
            </div>

            <div style="display: flex; justify-content: center; gap: 1.5rem; margin-bottom: 2rem;">
              <a href="https://github.com/Nostalgia546/Antigravity-Booster" target="_blank" class="btn btn-primary" style="text-decoration: none; display: flex; align-items: center; gap: 0.75rem; padding: 1rem 2.5rem; border-radius: 0.875rem; font-size: 1rem;">
                <Globe :size="20" /> GitHub 仓库
              </a>
              <a href="https://github.com/Nostalgia546/Antigravity-Booster/releases" target="_blank" class="btn btn-ghost" style="border: 1px solid var(--border); text-decoration: none; display: flex; align-items: center; gap: 0.75rem; padding: 1rem 2.5rem; border-radius: 0.875rem; font-size: 1rem;">
                <Download :size="20" /> 下载更新
              </a>
            </div>

            <button class="btn btn-ghost" style="font-size: 0.8rem; opacity: 0.7;" @click="showLogModal = true">
                <Activity :size="14" style="margin-right: 0.5rem;" /> 查看诊断日志
            </button>

            <div style="padding-top: 3rem; opacity: 0.5; font-size: 0.875rem; color: var(--text-secondary); display: flex; justify-content: center; align-items: center; gap: 2rem;">
              <span>&copy; 2026 Nostalgia546</span>
              <span style="width: 4px; height: 4px; background: var(--text-secondary); border-radius: 50%;"></span>
              <span>Licensed under GPL-3.0</span>
            </div>
        </div>
      </div>

      <!-- Add Account View -->
      <div v-if="activeTab === 'add_account'" class="animate-fade-in" style="max-width: 600px; margin: 0 auto;">
        <h2 style="margin-bottom: 0.5rem;">添加 Google 账号</h2>
        <p style="color: var(--text-secondary); margin-bottom: 2rem; font-size: 0.875rem;">选择您偏好的方式来添加账号。</p>
        
        <div class="grid" style="grid-template-columns: 1fr;">
          <!-- OAuth Option -->
          <div class="card" 
               :style="{ 
                 display: 'flex', 
                 alignItems: 'center', 
                 gap: '1.5rem', 
                 cursor: isLoggingIn ? 'not-allowed' : 'pointer',
                 opacity: isLoggingIn ? 0.7 : 1,
                 borderColor: isLoggingIn ? 'var(--accent)' : 'var(--border)'
               }" 
               @click="handleAddAccount">
            <div style="background: rgba(99, 102, 241, 0.1); padding: 1rem; border-radius: 1rem;">
              <Globe :size="32" color="var(--accent)" />
            </div>
            <div style="flex: 1;">
              <h3 style="margin-bottom: 0.25rem;">
                {{ isLoggingIn ? '等待授权中...' : 'OAuth 授权登录' }}
              </h3>
              <p style="font-size: 0.75rem; color: var(--text-secondary);">
                {{ isLoggingIn ? '请在浏览器中完成 Google 账号授权' : '使用 Google 账号安全登录并获取 Session 令牌。' }}
              </p>
            </div>
            <button class="btn btn-primary" style="padding: 0.5rem 1rem;" :disabled="isLoggingIn">
              {{ isLoggingIn ? '授权中...' : '开始登录' }}
            </button>
          </div>

          <!-- One-click Import from Editor -->
          <div class="card" 
               :style="{ 
                 display: 'flex', 
                 alignItems: 'center', 
                 gap: '1.5rem', 
                 cursor: isImportingFromEditor ? 'not-allowed' : 'pointer',
                 opacity: isImportingFromEditor ? 0.7 : 1,
                 borderColor: isImportingFromEditor ? 'var(--accent)' : 'var(--border)'
               }" 
               @click="handleImportFromAntigravity">
            <div style="background: rgba(16, 185, 129, 0.1); padding: 1rem; border-radius: 1rem;">
              <Download :size="32" color="var(--success)" />
            </div>
            <div style="flex: 1;">
              <h3 style="margin-bottom: 0.25rem;">
                {{ isImportingFromEditor ? '正在同步...' : '从 Antigravity 导入' }}
              </h3>
              <p style="font-size: 0.75rem; color: var(--text-secondary);">
                直接从编辑器中提取当前登录的账号信息。
              </p>
            </div>
            <button class="btn" style="background: var(--success); color: white; padding: 0.5rem 1rem;" :disabled="isImportingFromEditor">
              {{ isImportingFromEditor ? '请稍候...' : '立即导入' }}
            </button>
          </div>

          <!-- Local Backup -->
          <div class="card" style="display: flex; align-items: center; gap: 1.5rem; cursor: pointer;" @click="handleImportBackup">
            <div style="background: rgba(148, 163, 184, 0.1); padding: 1rem; border-radius: 1rem;">
              <RefreshCw :size="32" color="var(--text-secondary)" />
            </div>
            <div style="flex: 1;">
              <h3 style="margin-bottom: 0.25rem;">从备份恢复</h3>
              <p style="font-size: 0.75rem; color: var(--text-secondary);">从之前导出的 .json 备份文件中恢复账号。</p>
            </div>
            <button class="btn btn-ghost" style="border: 1px solid var(--border);">选择文件</button>
          </div>
        </div>

        <button class="btn btn-ghost" style="margin-top: 2rem; width: 100%;" @click="activeTab = 'accounts'">返回账号列表</button>
      </div>
    </div>
  </main>

  <!-- Custom Delete Confirmation Dialog -->
  <div v-if="showDeleteConfirm" class="modal-overlay" @click="cancelDelete">
    <div class="modal-content" @click.stop>
      <div class="modal-header">
        <AlertCircle :size="48" color="var(--error)" style="margin-bottom: 1rem;" />
        <h2 style="margin: 0; font-size: 1.5rem;">删除账号确认</h2>
      </div>
      
      <div class="modal-body">
        <p style="font-size: 1rem; margin-bottom: 1.5rem; color: var(--text-primary);">
          确定要移除账号 <strong style="color: var(--accent);">「{{ deleteConfirmData?.name }}」</strong> 吗？
        </p>
        
        <div style="background: var(--surface-hover); padding: 1.25rem; border-radius: 0.75rem; margin-bottom: 1.5rem;">
          <p style="font-size: 0.875rem; color: var(--text-secondary); margin-bottom: 0.75rem; font-weight: 600;">此操作将：</p>
          <ul style="list-style: none; padding: 0; margin: 0;">
            <li style="display: flex; align-items: flex-start; gap: 0.5rem; margin-bottom: 0.5rem;">
              <span style="color: var(--error); font-weight: bold;">•</span>
              <span style="font-size: 0.875rem; color: var(--text-secondary);">从 Booster 中删除该账号的所有数据</span>
            </li>
            <li style="display: flex; align-items: flex-start; gap: 0.5rem; margin-bottom: 0.5rem;">
              <span style="color: var(--error); font-weight: bold;">•</span>
              <span style="font-size: 0.875rem; color: var(--text-secondary);">清理本地存储的配额信息</span>
            </li>
            <li style="display: flex; align-items: flex-start; gap: 0.5rem;">
              <span style="color: var(--success); font-weight: bold;">•</span>
              <span style="font-size: 0.875rem; color: var(--text-secondary);">不会影响 Google 账号本身</span>
            </li>
          </ul>
        </div>
        
        <div style="background: rgba(239, 68, 68, 0.1); border-left: 3px solid var(--error); padding: 1rem; border-radius: 0.5rem;">
          <p style="font-size: 0.875rem; color: var(--error); margin: 0; font-weight: 600;">
            该操作不可恢复！
          </p>
        </div>
      </div>
      
      <div class="modal-footer">
        <button class="btn btn-ghost" @click="cancelDelete" style="flex: 1; border: 1px solid var(--border);">
          取消
        </button>
        <button class="btn" @click="confirmDelete" style="flex: 1; background: var(--error); color: white;">
          确定删除
        </button>
      </div>
    </div>
  </div>

  <!-- Custom Restart Confirmation Dialog -->
  <div v-if="showRestartConfirm" class="modal-overlay" @click="showRestartConfirm = false">
    <div class="modal-content" @click.stop>
      <div class="modal-header">
        <RefreshCw :size="48" color="var(--accent)" style="margin-bottom: 1rem;" />
        <h2 style="margin: 0; font-size: 1.5rem;">
            {{ restartConfirmType === 'after_install' ? '安装成功' : '重启 Antigravity' }}
        </h2>
      </div>
      
      <div class="modal-body">
        <p style="font-size: 1.125rem; margin-bottom: 1.5rem; color: var(--text-primary); text-align: center;">
          {{ restartConfirmType === 'after_install' ? '浏览器插件已安装成功！' : '确定要重启 Antigravity 吗？' }}
        </p>
        
        <div style="background: var(--surface-hover); padding: 1.25rem; border-radius: 0.75rem; margin-bottom: 1.5rem;">
          <p style="font-size: 0.875rem; color: var(--text-secondary); line-height: 1.6;">
            {{ restartConfirmType === 'after_install' 
                ? '为了使插件功能生效，我们需要重新启动编辑器环境。这可能会短暂中断当前的编辑进程。' 
                : '重新启动将刷新所有编辑器连接并应用最新的配置。请确保您的工作已保存。' 
            }}
          </p>
        </div>
        
        <div style="background: rgba(99, 102, 241, 0.1); border-left: 3px solid var(--accent); padding: 1rem; border-radius: 0.5rem; display: flex; align-items: center; gap: 0.75rem;">
          <Info :size="18" color="var(--accent)" />
          <p style="font-size: 0.875rem; color: var(--text-primary); margin: 0;">
            重启过程通常仅需几秒钟。
          </p>
        </div>
      </div>
      
      <div class="modal-footer">
        <button class="btn btn-ghost" @click="showRestartConfirm = false" style="flex: 1; border: 1px solid var(--border);">
          稍后再说
        </button>
        <button class="btn btn-primary" @click="confirmRestart" style="flex: 1; font-weight: 600;">
          立即重启
        </button>
      </div>
    </div>
  </div>

  <!-- Diagnostic Log Modal -->
  <div v-if="showLogModal" class="modal-overlay" @click="showLogModal = false">
      <div class="modal-content" style="max-width: 800px; width: 90vw;" @click.stop>
          <div class="modal-header">
              <Activity :size="32" color="var(--accent)" />
              <h2 style="margin: 0; font-size: 1.25rem;">诊断日志</h2>
          </div>
          <div class="modal-body">
              <div style="background: var(--surface-hover); color: var(--text-primary); padding: 1rem; border-radius: 0.5rem; font-family: 'JetBrains Mono', monospace; font-size: 0.75rem; max-height: 400px; overflow-y: auto; text-align: left;">
                  <div v-for="(log, idx) in store.logs" :key="idx" style="margin-bottom: 0.25rem;">
                      {{ log }}
                  </div>
                  <div v-if="store.logs.length === 0" style="opacity: 0.5;">暂无诊断数据。点击“账号管理”中的任意切换或刷新操作将生成日志。</div>
              </div>
          </div>
          <div class="modal-footer">
              <button class="btn btn-primary" @click="showLogModal = false" style="width: 100%;">确认</button>
          </div>
      </div>
  </div>
</template>
