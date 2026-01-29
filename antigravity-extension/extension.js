const vscode = require('vscode');
const fs = require('fs');
const path = require('path');
const os = require('os');

let statusBarItem;
let outputChannel;
let automationService;
let guardianInstance;
let webviewProviderInstance;

/**
 * 自动同意服务: 完全保持原始逻辑
 */
class AutomationService {
    constructor() {
        this._enabled = false;
        this._timer = null;
        this._interval = 800;
        this._configSection = "tfa";
        this._configKey = "system.autoAccept";
    }

    start() {
        if (this._enabled) return;
        this._enabled = true;
        this._scheduleNext();
        if (outputChannel) outputChannel.appendLine("Automation: Started");
    }

    stop() {
        this._enabled = false;
        if (this._timer) {
            clearTimeout(this._timer);
            this._timer = null;
        }
        if (outputChannel) outputChannel.appendLine("Automation: Stopped");
    }

    _scheduleNext() {
        if (!this._enabled) return;
        this._timer = setTimeout(async () => {
            await this._execute();
            this._scheduleNext();
        }, this._interval);
    }

    async _execute() {
        if (!this._enabled) return;
        const config = vscode.workspace.getConfiguration(this._configSection);
        if (!config.get(this._configKey, false)) return;
        try { await vscode.commands.executeCommand('antigravity.agent.acceptAgentStep'); } catch (e) { }
        try { await vscode.commands.executeCommand('antigravity.terminal.accept'); } catch (e) { }
    }

    syncWithConfig() {
        const config = vscode.workspace.getConfiguration(this._configSection);
        const enabled = config.get(this._configKey, false);
        if (enabled) this.start();
        else this.stop();
        return enabled;
    }
}

/**
 * QuotaGuardian: 离线配额守护 + Token 自动续期
 */
class QuotaGuardian {
    constructor(logger) {
        this.logger = logger;
        this.appData = process.env.APPDATA || (process.platform == 'darwin' ? path.join(process.env.HOME, 'Library', 'Application Support') : path.join(process.env.HOME, '.local', 'share'));
        this.baseDir = path.join(this.appData, 'com.tz.antigravity-booster');
        this.bridgePath = path.join(this.baseDir, 'quota_bridge.json');
        this.accountsPath = path.join(this.baseDir, 'accounts.json');
        this.bufferPath = path.join(this.baseDir, 'quota_buffer.json');
        this._cachedToken = null;
        this.onDataUpdated = null;
        // 对齐 Booster 后端 Client ID
        this.CLIENT_ID = "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
    }

    start() {
        this._planNextCheck();
    }

    _planNextCheck() {
        const now = new Date();
        const secondsPast5min = (now.getMinutes() % 5) * 60 + now.getSeconds();
        let sleepMs = (300 - secondsPast5min + 15) * 1000;
        if (sleepMs < 0) sleepMs += 300000;

        setTimeout(async () => {
            await this.performCheck();
            this._planNextCheck();
        }, sleepMs);
    }

    async performCheck() {
        const activeAcc = this._loadActiveAccount();
        if (!activeAcc) return;

        let tokenToUse = (this._cachedToken && this._cachedToken.email === activeAcc.email)
            ? this._cachedToken.access_token
            : (activeAcc.token_data ? activeAcc.token_data.access_token : activeAcc.token);

        if (!tokenToUse) return;

        try {
            let quota;
            try {
                quota = await this._fetchQuota(tokenToUse);
            } catch (err) {
                if (err.message.includes('401') && activeAcc.token_data && activeAcc.token_data.refresh_token) {
                    const newToken = await this._refreshAccessToken(activeAcc.token_data.refresh_token);
                    if (newToken) {
                        this._cachedToken = { email: activeAcc.email, access_token: newToken };
                        quota = await this._fetchQuota(newToken);
                    } else throw err;
                } else throw err;
            }

            if (quota && quota.models) {
                if (this.onDataUpdated) this.onDataUpdated(quota);
                this._recordToBuffer(activeAcc.id, activeAcc.name, quota);
            }
        } catch (e) { }
    }

    async _refreshAccessToken(refreshToken) {
        const https = require('https');
        const data = `client_id=${this.CLIENT_ID}&refresh_token=${refreshToken}&grant_type=refresh_token`;
        return new Promise((resolve) => {
            const req = https.request({
                hostname: 'oauth2.googleapis.com', path: '/token', method: 'POST',
                headers: { 'Content-Type': 'application/x-www-form-urlencoded' }
            }, (res) => {
                let body = ''; res.on('data', chunk => body += chunk);
                res.on('end', () => { try { resolve(JSON.parse(body).access_token || null); } catch (e) { resolve(null); } });
            });
            req.on('error', () => resolve(null)); req.write(data); req.end();
        });
    }

    _loadActiveAccount() {
        try {
            if (!fs.existsSync(this.accountsPath)) return null;
            const accounts = JSON.parse(fs.readFileSync(this.accountsPath, 'utf8'));
            return accounts.find(a => a.is_active);
        } catch (e) { return null; }
    }

    async _fetchQuota(token) {
        const https = require('https');
        const payload = JSON.stringify({ project: "bamboo-precept-lgxtn" });
        return new Promise((resolve, reject) => {
            const req = https.request({
                hostname: 'cloudcode-pa.googleapis.com', path: '/v1internal:fetchAvailableModels', method: 'POST',
                headers: { 'Authorization': `Bearer ${token}`, 'Content-Type': 'application/json', 'User-Agent': 'antigravity/1.11.3 Darwin/arm64' },
                timeout: 10000
            }, (res) => {
                let body = ''; res.on('data', chunk => body += chunk);
                res.on('end', () => {
                    if (res.statusCode === 200) {
                        try {
                            const data = JSON.parse(body);
                            const models = [];
                            const mapping = { "gemini-3-pro-high": "Gemini Pro", "gemini-3-flash": "Gemini Flash", "claude-sonnet-4-5": "Claude" };
                            for (const [id, info] of Object.entries(data.models || {})) {
                                if (mapping[id]) models.push({ name: mapping[id], percentage: (info.quotaInfo?.remainingFraction || 0) * 100, reset_at: this._parseDate(info.quotaInfo?.resetTime) });
                            }
                            resolve({ models, last_updated: Math.floor(Date.now() / 1000) });
                        } catch (e) { reject(e); }
                    } else reject(new Error(`Status ${res.statusCode}`));
                });
            });
            req.on('error', (e) => reject(e)); req.write(payload); req.end();
        });
    }

    _parseDate(iso) { return iso ? Math.floor(new Date(iso).getTime() / 1000) : null; }

    _recordToBuffer(accId, accName, quota) {
        let buffer = [];
        try { if (fs.existsSync(this.bufferPath)) buffer = JSON.parse(fs.readFileSync(this.bufferPath, 'utf8')); } catch (e) { }
        const point = { timestamp: quota.last_updated, usage: {}, reset_at: {}, account_names: {} };
        point.account_names[accId] = accName;
        for (const m of quota.models) {
            const key = `${accId}:${m.name}`;
            point.usage[key] = m.percentage;
            if (m.reset_at) point.reset_at[key] = m.reset_at;
        }
        buffer.push(point);
        if (buffer.length > 5000) buffer.shift();
        fs.writeFileSync(this.bufferPath, JSON.stringify(buffer, null, 2));
    }

    getBufferCount() {
        try {
            if (!fs.existsSync(this.bufferPath)) return 0;
            const buffer = JSON.parse(fs.readFileSync(this.bufferPath, 'utf8'));
            return Array.isArray(buffer) ? buffer.length : 0;
        } catch (e) { return 0; }
    }
}

/**
 * 辅助：时间格式化逻辑 (对齐 Booster 后端)
 */
function formatTimeLeft(seconds) {
    if (!seconds || seconds <= 0) return "已重置";
    const now = Math.floor(Date.now() / 1000);
    const diff = seconds - now;
    if (diff <= 0) return "已重置";
    const hours = Math.floor(diff / 3600);
    const mins = Math.floor((diff % 3600) / 60);
    if (hours >= 24) {
        const days = Math.floor(hours / 24);
        const remH = hours % 24;
        return remH > 0 ? `${days}天${remH}小时` : `${days}天`;
    }
    return `${hours}小时 ${mins}分`;
}

function activate(context) {
    outputChannel = vscode.window.createOutputChannel("Antigravity Booster");
    outputChannel.appendLine("Antigravity Booster Helper Activated");

    automationService = new AutomationService();
    automationService.syncWithConfig();

    guardianInstance = new QuotaGuardian(outputChannel);
    guardianInstance.start();

    statusBarItem = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 100);
    statusBarItem.command = 'antigravityBooster.showDetails';
    context.subscriptions.push(statusBarItem);

    webviewProviderInstance = new BoosterWebviewProvider(context.extensionUri);
    context.subscriptions.push(vscode.window.registerWebviewViewProvider('antigravityBoosterView', webviewProviderInstance));

    const appDir = path.join(os.homedir(), 'AppData', 'Roaming', 'com.tz.antigravity-booster');
    const bridgePath = path.join(appDir, 'quota_bridge.json');

    const reflectDataToUI = (data) => {
        // 1. 更新底部状态栏
        if (data.models && data.models.length > 0) {
            const shortParts = data.models.map(m => {
                const sName = m.name.includes("Pro") ? "Pro" : (m.name.includes("Flash") ? "Flash" : (m.name.includes("Claude") ? "Claude" : m.name));
                return `${sName}: ${Math.floor(m.percentage)}%`;
            });
            statusBarItem.text = `$(rocket) ${shortParts.join('  ')}`;

            let md = "| Model | Usage | Reset |\n|---|---|---|\n";
            data.models.forEach(m => {
                md += `| ${m.name} | ${m.percentage.toFixed(1)}% | ${formatTimeLeft(m.reset_at)} |\n`;
            });
            statusBarItem.tooltip = new vscode.MarkdownString(md);
            statusBarItem.show();
        }

        // 2. 更新 Webview
        if (webviewProviderInstance) {
            webviewProviderInstance.updateData({ status: 'connected', models: data.models || [] });
            webviewProviderInstance.updateConfig({ bufferCount: guardianInstance.getBufferCount() });
        }
    };

    const updateState = () => {
        try {
            if (fs.existsSync(bridgePath)) {
                const data = JSON.parse(fs.readFileSync(bridgePath, 'utf8'));
                reflectDataToUI(data);
            } else {
                statusBarItem.text = "$(rocket) Booster Linked";
                statusBarItem.show();
                if (webviewProviderInstance) webviewProviderInstance.updateData({ status: 'disconnected', models: [] });
            }
        } catch (e) { }
    };

    // 绑定 Guardian 回调，实现离线时的状态栏更新
    guardianInstance.onDataUpdated = (data) => {
        let boosterActive = false;
        if (fs.existsSync(bridgePath)) {
            const stats = fs.statSync(bridgePath);
            if (Date.now() - stats.mtimeMs < 150000) boosterActive = true;
        }
        if (!boosterActive) reflectDataToUI(data);
    };

    updateState();
    fs.watchFile(bridgePath, { interval: 2000 }, () => updateState());

    context.subscriptions.push(vscode.workspace.onDidChangeConfiguration(e => {
        if (e.affectsConfiguration('tfa.system.autoAccept')) {
            const enabled = automationService.syncWithConfig();
            if (webviewProviderInstance) webviewProviderInstance.updateConfig({ autoAccept: enabled });
        }
    }));

    context.subscriptions.push(vscode.commands.registerCommand('antigravityBooster.toggleAutoAccept', async (val) => {
        const config = vscode.workspace.getConfiguration('tfa');
        let nextVal = !config.get('system.autoAccept', false);
        if (typeof val === 'boolean') nextVal = val;
        await config.update('system.autoAccept', nextVal, vscode.ConfigurationTarget.Global);
    }));

    context.subscriptions.push(vscode.commands.registerCommand('antigravityBooster.showDetails', () => {
        vscode.window.showInformationMessage(`Antigravity Booster v1.3.5`);
    }));
}

function deactivate() { if (automationService) automationService.stop(); }

class BoosterWebviewProvider {
    constructor(extensionUri) { this._extensionUri = extensionUri; this._view = undefined; this._lastData = { status: 'unknown', models: [] }; }
    resolveWebviewView(webviewView, context, _token) {
        this._view = webviewView;
        webviewView.webview.options = { enableScripts: true, localResourceRoots: [this._extensionUri] };
        webviewView.webview.html = this._getHtmlForWebview(webviewView.webview);
        webviewView.webview.onDidReceiveMessage(message => {
            if (message.type === 'webviewReady') {
                const isFeatureOn = vscode.workspace.getConfiguration('tfa').get('system.autoAccept', false);
                this.updateConfig({ autoAccept: isFeatureOn, version: "1.3.5", bufferCount: guardianInstance.getBufferCount() });
                this.updateData(this._lastData);
            } else if (message.type === 'toggleAutoAccept') {
                vscode.commands.executeCommand('antigravityBooster.toggleAutoAccept', message.val);
            }
        });
    }
    updateData(data) { this._lastData = data; if (this._view) this._view.webview.postMessage({ type: 'updateData', payload: data }); }
    updateConfig(config) { if (this._view) this._view.webview.postMessage({ type: 'updateConfig', payload: config }); }
    _getHtmlForWebview(webview) {
        return `<!DOCTYPE html><html><head><meta charset="UTF-8"><style>
        :root { --bg: var(--vscode-sideBar-background); --fg: var(--vscode-sideBar-foreground); --border: var(--vscode-widget-border); --button-bg: var(--vscode-button-background); --button-fg: var(--vscode-button-foreground); }
        body { padding: 16px; font-family: var(--vscode-font-family); color: var(--fg); background: var(--bg); user-select: none; }
        .section-title { font-size: 11px; font-weight: 600; text-transform: uppercase; opacity: 0.8; margin-bottom: 12px; letter-spacing: 0.5px; }
        .gauges { display: grid; grid-template-columns: repeat(2, 1fr); gap: 16px; margin-bottom: 24px; }
        .gauge-item { display: flex; flex-direction: column; align-items: center; position: relative; }
        .gauge-circle { width: 48px; height: 48px; border-radius: 50%; display: flex; align-items: center; justify-content: center; background: conic-gradient(var(--color) var(--deg), var(--border) 0deg); margin-bottom: 4px; position: relative; }
        .gauge-circle::before { content: ''; position: absolute; inset: 4px; background: var(--bg); border-radius: 50%; }
        .gauge-val { position: relative; font-size: 10px; font-weight: bold; }
        .gauge-label { font-size: 11px; text-align: center; opacity: 0.9; }
        .gauge-reset { font-size: 9px; opacity: 0.5; margin-top: 2px; }
        .control-row { display: flex; align-items: center; justify-content: space-between; padding: 10px 0; border-bottom: 1px solid var(--border); }
        .switch { position: relative; display: inline-block; width: 34px; height: 18px; }
        .switch input { opacity: 0; width: 0; height: 0; }
        .slider { position: absolute; cursor: pointer; top: 0; left: 0; right: 0; bottom: 0; background-color: #ccc; transition: .4s; border-radius: 18px; }
        .slider:before { position: absolute; content: ""; height: 14px; width: 14px; left: 2px; bottom: 2px; background-color: white; transition: .4s; border-radius: 50%; }
        input:checked + .slider { background-color: #2196F3; }
        input:checked + .slider:before { transform: translateX(16px); }
        .footer { margin-top: 24px; padding-top: 16px; border-top: 1px dashed var(--border); opacity: 0.6; font-size: 10px; display: flex; justify-content: space-between; }
        </style></head><body>
    <div class="section-title">模型余量池</div>
    <div id="gauges-container" class="gauges"><div style="font-size:12px; opacity:0.6; text-align:center; grid-column: span 2;">数据加载中...</div></div>
    <div class="section-title">控制面板</div>
    <div class="control-row">
        <div><div style="font-size:13px;">自动同意 (Auto-Accept)</div><div style="font-size:10px; opacity:0.7;">自动同意 Agent 操作</div></div>
        <label class="switch"><input type="checkbox" id="auto-accept-toggle"><span class="slider"></span></label>
    </div>
    <div class="footer"><span id="ver">v1.3.5</span><span id="buf">Buffered: 0</span></div>
    <script>
        const vscode = acquireVsCodeApi();
        const toggle = document.getElementById('auto-accept-toggle');
        
        function formatTimeLeft(seconds) {
            if (!seconds || seconds <= 0) return "已重置";
            const now = Math.floor(Date.now() / 1000);
            const diff = seconds - now;
            if (diff <= 0) return "已重置";
            const h = Math.floor(diff / 3600);
            const m = Math.floor((diff % 3600) / 60);
            if (h >= 24) {
                const days = Math.floor(h / 24);
                const remH = h % 24;
                return remH > 0 ? days + "天" + remH + "小时" : days + "天";
            }
            return h + "小时 " + m + "分";
        }

        window.addEventListener('message', event => {
            const msg = event.data;
            if (msg.type === 'updateData' && msg.payload.models) {
                const container = document.getElementById('gauges-container');
                container.innerHTML = '';
                msg.payload.models.forEach(m => {
                    const percent = Math.floor(m.percentage);
                    let color = percent <= 20 ? 'red' : (percent <= 50 ? 'orange' : '#2196F3');
                    const item = document.createElement('div');
                    item.className = 'gauge-item';
                    item.innerHTML = \`<div class="gauge-circle" style="--deg: \${percent * 3.6}deg; --color: \${color}"><div class="gauge-val">\${percent}%</div></div>
                                     <div class="gauge-label">\${m.name}</div>
                                     <div class="gauge-reset">\${formatTimeLeft(m.reset_at)}</div>\`;
                    container.appendChild(item);
                });
            }
            if (msg.type === 'updateConfig') {
                if (msg.payload.autoAccept !== undefined) toggle.checked = msg.payload.autoAccept;
                if (msg.payload.bufferCount !== undefined) document.getElementById('buf').innerText = 'Buffered: ' + msg.payload.bufferCount;
            }
        });
        toggle.addEventListener('change', (e) => vscode.postMessage({ type: 'toggleAutoAccept', val: e.target.checked }));
        vscode.postMessage({ type: 'webviewReady' });
    </script></body></html>`;
    }
}
module.exports = { activate, deactivate }
