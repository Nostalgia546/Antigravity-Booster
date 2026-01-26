import { defineStore } from 'pinia';
import { ref, watch, computed } from 'vue';

export interface ModelQuota {
    name: string;
    percentage: number;
    reset_time: string;
}

export interface QuotaData {
    models: ModelQuota[];
    last_updated: number;
}

export interface Account {
    id: string;
    name: string;
    email: string;
    token: string;
    account_type: 'Gemini' | 'Claude' | 'OpenAI';
    status: 'active' | 'expired';
    quota?: QuotaData;
    is_active: boolean;
}

export interface BoosterConfig {
    proxy_enabled: boolean;
    proxy_host: string;
    proxy_port: number;
    proxy_type: 'socks5' | 'http';
    target_processes: string[];
}

export const useAppStore = defineStore('app', () => {
    // Theme State
    const theme = ref<'light' | 'dark'>(
        (localStorage.getItem('theme') as 'light' | 'dark') ||
        (window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light')
    );

    // App State
    const accounts = ref<Account[]>([]);
    const isBoosted = ref(false);
    const proxyConfig = ref<BoosterConfig>({
        proxy_enabled: false,
        proxy_host: '127.0.0.1',
        proxy_port: 7890,
        proxy_type: 'socks5',
        target_processes: ['Claude.exe', 'Code.exe', 'cherry-studio.exe']
    });

    const activeAccount = computed(() => accounts.value.find(a => a.is_active));

    // Logs
    const logs = ref<string[]>([]);

    // Watchers
    watch(theme, (newTheme) => {
        localStorage.setItem('theme', newTheme);
        document.documentElement.setAttribute('data-theme', newTheme);
    }, { immediate: true });

    // Actions
    function toggleTheme() {
        theme.value = theme.value === 'light' ? 'dark' : 'light';
    }

    function addLog(msg: string) {
        logs.value.unshift(`[${new Date().toLocaleTimeString()}] ${msg}`);
        if (logs.value.length > 50) logs.value.pop();
    }

    function addAccount(acc: Omit<Account, 'id' | 'status' | 'is_active' | 'quota'>) {
        // This will be called before backend sync for UI responsiveness
        const newAcc: Account = {
            ...acc,
            id: crypto.randomUUID(),
            status: 'active',
            is_active: accounts.value.length === 0,
            quota: {
                models: [
                    { name: 'Default', percentage: 100, reset_time: 'Tomorrow' }
                ],
                last_updated: Date.now()
            }
        };
        accounts.value.push(newAcc);
        addLog(`Preparing account: ${acc.name}`);
        return newAcc;
    }

    return {
        theme,
        toggleTheme,
        accounts,
        activeAccount,
        addAccount,
        isBoosted,
        proxyConfig,
        logs,
        addLog
    };
});
