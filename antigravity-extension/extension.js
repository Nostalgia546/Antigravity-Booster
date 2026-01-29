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
 * AutomationEngine: 自动同意服务
 * 监听配置空间 antigravity.automation.autoAccept
 */
class AutomationEngine {
    constructor() {
        this._active = false;
        this._timer = null;
        this._pulseRate = 800;
        this._namespace = "antigravity";
        this._featureKey = "automation.autoAccept";
    }

    start() {
        if (this._active) return;
        this._active = true;
        this._schedulePulse();
        if (outputChannel) outputChannel.appendLine("[Automation] Engine started");
    }

    stop() {
        this._active = false;
        if (this._timer) {
            clearTimeout(this._timer);
            this._timer = null;
        }
        if (outputChannel) outputChannel.appendLine("[Automation] Engine stopped");
    }

    _schedulePulse() {
        if (!this._active) return;
        this._timer = setTimeout(async () => {
            await this._performPulse();
            this._schedulePulse();
        }, this._pulseRate);
    }

    async _performPulse() {
        if (!this._active) return;
        const config = vscode.workspace.getConfiguration(this._namespace);
        if (!config.get(this._featureKey, false)) return;

        try {
            await vscode.commands.executeCommand('antigravity.agent.acceptAgentStep');
        } catch (e) { }

        try {
            await vscode.commands.executeCommand('antigravity.terminal.accept');
        } catch (e) { }
    }

    syncWithConfig() {
        const config = vscode.workspace.getConfiguration(this._namespace);
        const enabled = config.get(this._featureKey, false);
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
    }

    start() {
        this._planNextCheck();
        this.logger.appendLine("[Guardian] Monitoring active with Auto-Refresh Token support");
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
        let boosterAlive = false;
        if (fs.existsSync(this.bridgePath)) {
            const stats = fs.statSync(this.bridgePath);
            if (Date.now() - stats.mtimeMs < 240000) boosterAlive = true;
        }

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
                if (!boosterAlive && webviewProviderInstance) {
                    webviewProviderInstance.updateData({ status: 'connected', models: quota.models });
                    webviewProviderInstance.updateConfig({ bufferCount: this.getBufferCount() });
                }
                this._recordToBuffer(activeAcc.id, activeAcc.name, quota);

                const nowTotalSec = Math.floor(Date.now() / 1000);
                for (const m of quota.models) {
                    if (m.reset_at) {
                        const timeToReset = m.reset_at - nowTotalSec;
                        if (timeToReset > 35 && timeToReset < 310) {
                            setTimeout(() => this.performCheck(), (timeToReset - 30) * 1000);
                        } else if (timeToReset > 0 && timeToReset <= 35) {
                            setTimeout(() => this.performCheck(), (timeToReset + 2) * 1000);
                        }
                    }
                }
            }
        } catch (e) { }
    }

    async _refreshAccessToken(refreshToken) {
        const https = require('https');
        const data = `client_id=764086051850-6v6968m678q3948t10tmldv4sq4c9rjt.apps.googleusercontent.com&refresh_token=${refreshToken}&grant_type=refresh_token`;
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

    getBufferCount() {
        try {
            if (!fs.existsSync(this.bufferPath)) return 0;
            const buffer = JSON.parse(fs.readFileSync(this.bufferPath, 'utf8'));
            return Array.isArray(buffer) ? buffer.length : 0;
        } catch (e) { return 0; }
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
                                if (mapping[id]) models.push({ name: mapping[id], percentage: (info.quotaInfo?.remainingFraction || 0) * 100, reset_at: this._parseTimestamp(info.quotaInfo?.resetTime) });
                            }
                            resolve({ models, last_updated: Math.floor(Date.now() / 1000) });
                        } catch (e) { reject(e); }
                    } else reject(new Error(`Status ${res.statusCode}`));
                });
            });
            req.on('error', (e) => reject(e)); req.write(payload); req.end();
        });
    }

    _parseTimestamp(iso) { try { return iso ? Math.floor(new Date(iso).getTime() / 1000) : null; } catch (e) { return null; } }

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
        if (buffer.length > 10000) buffer.shift();
        fs.writeFileSync(this.bufferPath, JSON.stringify(buffer, null, 2));
    }
}

function activate(context) {
    outputChannel = vscode.window.createOutputChannel("Antigravity Booster");
    outputChannel.appendLine("Antigravity Booster Helper v1.3.5 Activated");

    // 1. 初始化引擎
    automationService = new AutomationEngine();
    automationService.syncWithConfig();

    // 2. 初始化新增的记录守护
    guardianInstance = new QuotaGuardian(outputChannel);
    guardianInstance.start();

    // 3. 状态栏
    statusBarItem = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 100);
    statusBarItem.command = 'antigravity.booster.showDetails';
    context.subscriptions.push(statusBarItem);

    // 4. Webview 
    webviewProviderInstance = new BoosterWebviewProvider(context.extensionUri);
    context.subscriptions.push(
        vscode.window.registerWebviewViewProvider('antigravity.booster.statusView', webviewProviderInstance)
    );

    const appData = process.env.APPDATA || (process.platform == 'darwin' ? process.env.HOME + '/Library/Application Support' : process.env.HOME + "/.local/share");
    const bridgePath = path.join(appData, 'com.tz.antigravity-booster', 'quota_bridge.json');

    const updateState = () => {
        try {
            if (fs.existsSync(bridgePath)) {
                const data = JSON.parse(fs.readFileSync(bridgePath, 'utf8'));
                const displayText = data.status_text || data.text;
                if (displayText) {
                    statusBarItem.text = `$(rocket) ${displayText}`;
                    if (data.tooltip) {
                        statusBarItem.tooltip = new vscode.MarkdownString(data.tooltip);
                    }
                    statusBarItem.show();
                } else {
                    statusBarItem.hide();
                }
                webviewProviderInstance.updateData({ status: 'connected', models: data.models || [] });
                webviewProviderInstance.updateConfig({ bufferCount: guardianInstance.getBufferCount() });
            } else {
                statusBarItem.text = "$(rocket) Booster Linked";
                statusBarItem.show();
                webviewProviderInstance.updateData({ status: 'disconnected', models: [] });
            }
        } catch (e) { }
    };

    updateState();
    fs.watchFile(bridgePath, { interval: 2000 }, () => updateState());

    // 5. 配置监听
    context.subscriptions.push(vscode.workspace.onDidChangeConfiguration(e => {
        if (e.affectsConfiguration('antigravity.automation.autoAccept')) {
            const enabled = automationService.syncWithConfig();
            webviewProviderInstance.updateConfig({ autoAccept: enabled });
        }
    }));

    // 6. 命令注册
    context.subscriptions.push(vscode.commands.registerCommand('antigravity.booster.toggleAutoAccept', async (val) => {
        const config = vscode.workspace.getConfiguration('antigravity');
        let nextVal = !config.get('automation.autoAccept', false);
        if (typeof val === 'boolean') nextVal = val;

        await config.update('automation.autoAccept', nextVal, vscode.ConfigurationTarget.Global);
    }));

    context.subscriptions.push(vscode.commands.registerCommand('antigravity.openRule', async () => {
        const doc = await vscode.workspace.openTextDocument(path.join(os.homedir(), '.gemini', 'GEMINI.md'));
        await vscode.window.showTextDocument(doc);
    }));

    context.subscriptions.push(vscode.commands.registerCommand('antigravity.booster.showDetails', () => {
        vscode.window.showInformationMessage(`Antigravity Booster v1.3.5`);
    }));
}

function deactivate() { if (automationService) automationService.stop(); if (statusBarItem) statusBarItem.dispose(); }

class BoosterWebviewProvider {
    constructor(extensionUri) { this._extensionUri = extensionUri; this._view = undefined; this._lastData = { status: 'unknown', models: [] }; }
    resolveWebviewView(webviewView, context, _token) {
        this._view = webviewView;
        webviewView.webview.options = { enableScripts: true, localResourceRoots: [this._extensionUri] };
        webviewView.webview.html = this._getHtmlForWebview(webviewView.webview);
        webviewView.webview.onDidReceiveMessage(message => {
            if (message.type === 'webviewReady') {
                const config = vscode.workspace.getConfiguration('antigravity');
                const enabled = config.get('automation.autoAccept', false);
                this.updateConfig({
                    autoAccept: enabled,
                    version: "1.3.5",
                    bufferCount: guardianInstance ? guardianInstance.getBufferCount() : 0
                });
                this.updateData(this._lastData);
            } else if (message.type === 'toggleAutoAccept') {
                vscode.commands.executeCommand('antigravity.booster.toggleAutoAccept', message.val);
            } else if (message.type === 'openRules') {
                vscode.commands.executeCommand('antigravity.openRule');
            }
        });
    }
    updateData(data) { this._lastData = data; if (this._view) this._view.webview.postMessage({ type: 'updateData', payload: data }); }
    updateConfig(config) { if (this._view) this._view.webview.postMessage({ type: 'updateConfig', payload: config }); }
    _getHtmlForWebview(webview) {
        return `<!DOCTYPE html><html><head><meta charset="UTF-8"><style>
        :root { --bg: var(--vscode-sideBar-background); --fg: var(--vscode-sideBar-foreground); --border: var(--vscode-widget-border); --button-bg: var(--vscode-button-background); --button-fg: var(--vscode-button-foreground); --button-hover: var(--vscode-button-hoverBackground); }
        body { padding: 16px; font-family: var(--vscode-font-family); color: var(--fg); background: var(--bg); user-select: none; }
        .section-title { font-size: 11px; font-weight: 600; text-transform: uppercase; opacity: 0.8; margin-bottom: 12px; letter-spacing: 0.5px; }
        .gauges { display: grid; grid-template-columns: repeat(2, 1fr); gap: 16px; margin-bottom: 24px; }
        .gauge-item { display: flex; flex-direction: column; align-items: center; position: relative; }
        .gauge-circle { width: 48px; height: 48px; border-radius: 50%; display: flex; align-items: center; justify-content: center; background: conic-gradient(var(--color) var(--deg), var(--border) 0deg); margin-bottom: 4px; position: relative; }
        .gauge-circle::before { content: ''; position: absolute; inset: 4px; background: var(--bg); border-radius: 50%; }
        .gauge-val { position: relative; font-size: 10px; font-weight: bold; }
        .gauge-label { font-size: 11px; text-align: center; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; width: 100%; opacity: 0.9; }
        .gauge-reset { font-size: 9px; opacity: 0.5; margin-top: 2px; }
        .control-row { display: flex; align-items: center; justify-content: space-between; padding: 10px 0; border-bottom: 1px solid var(--border); }
        .control-label { font-size: 13px; font-weight: 500; }
        .hint { font-size: 10px; opacity: 0.7; margin-top: 2px; }
        .switch { position: relative; display: inline-block; width: 36px; height: 18px; }
        .switch input { opacity: 0; width: 0; height: 0; }
        .slider { position: absolute; cursor: pointer; top: 0; left: 0; right: 0; bottom: 0; background-color: var(--border); transition: .4s; border-radius: 18px; }
        .slider:before { position: absolute; content: ""; height: 14px; width: 14px; left: 2px; bottom: 2px; background-color: white; transition: .4s; border-radius: 50%; }
        input:checked + .slider { background-color: var(--vscode-charts-blue); }
        input:checked + .slider:before { transform: translateX(18px); }
        .btn { background: var(--button-bg); color: var(--button-fg); border: none; padding: 8px 12px; border-radius: 2px; cursor: pointer; width: 100%; font-size: 12px; margin-top: 10px; }
        .btn:hover { background: var(--button-hover); }
        .footer { margin-top: 24px; padding-top: 16px; border-top: 1px dashed var(--border); opacity: 0.6; font-size: 10px; display: flex; justify-content: space-between; }
        </style></head><body>
    <div class="section-title">模型余量池</div>
    <div id="gauges-container" class="gauges"><div style="font-size:12px; opacity:0.6; grid-column:span 2; text-align:center">获取中...</div></div>
    <div class="section-title">控制面板</div>
    <div class="control-row">
        <div><div class="control-label">自动同意 (Auto-Accept)</div><div class="hint">自动同意 Agent 的操作</div></div>
        <label class="switch"><input type="checkbox" id="auto-accept-toggle"><span class="slider"></span></label>
    </div>
    <button class="btn" id="btn-rules">编辑规则 (GEMINI.md)</button>
    <div class="footer"><span id="ver">v--</span><span id="buf">Buffered: 0 points</span></div>
    <script>
        const vscode = acquireVsCodeApi();
        const toggle = document.getElementById('auto-accept-toggle');
        vscode.postMessage({ type: 'webviewReady' });
        toggle.addEventListener('change', (e) => { vscode.postMessage({ type: 'toggleAutoAccept', val: e.target.checked }); });
        document.getElementById('btn-rules').addEventListener('click', () => { vscode.postMessage({ type: 'openRules' }); });
        window.addEventListener('message', event => {
            const msg = event.data;
            if (msg.type === 'updateData') {
                const container = document.getElementById('gauges-container');
                if (!msg.payload.models?.length) return;
                container.innerHTML = '';
                msg.payload.models.forEach(m => {
                    const percent = Math.floor(m.percentage);
                    let color = percent <= 20 ? 'var(--vscode-charts-red)' : (percent <= 40 ? 'var(--vscode-charts-orange)' : 'var(--vscode-charts-blue)');
                    
                    let resetText = '';
                    if (m.reset_at) {
                        const now = Math.floor(Date.now() / 1000);
                        const diff = m.reset_at - now;
                        if (diff <= 0) {
                            resetText = '已重置';
                        } else {
                            const hours = Math.floor(diff / 3600);
                            const mins = Math.floor((diff % 3600) / 60);
                            if (hours >= 24) {
                                const days = Math.floor(hours / 24);
                                const remH = hours % 24;
                                resetText = remH > 0 ? \`\${days}天\${remH}小时 \${mins}分\` : \`\${days}天 \${mins}分\`;
                            } else {
                                resetText = \`\${hours}小时 \${mins}分\`;
                            }
                        }
                    }

                    const item = document.createElement('div');
                    item.className = 'gauge-item';
                    item.innerHTML = \`<div class="gauge-circle" style="--deg: \${percent * 3.6}deg; --color: \${color}"><div class="gauge-val">\${percent}%</div></div><div class="gauge-label">\${m.name}</div><div class="gauge-reset">\${resetText}</div>\`;
                    container.appendChild(item);
                });
            }
            if (msg.type === 'updateConfig') {
                if (msg.payload.autoAccept !== undefined && toggle.checked !== msg.payload.autoAccept) toggle.checked = msg.payload.autoAccept;
                if (msg.payload.version) document.getElementById('ver').innerText = 'v' + msg.payload.version;
                if (msg.payload.bufferCount !== undefined) document.getElementById('buf').innerText = 'Buffered: ' + msg.payload.bufferCount + ' points';
            }
        });
    </script></body></html>`;
    }
}
module.exports = { activate, deactivate }
