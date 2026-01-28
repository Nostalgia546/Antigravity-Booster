const vscode = require('vscode');
const fs = require('fs');
const path = require('path');
const os = require('os');

let statusBarItem;
let outputChannel;
let automationService;

/**
 * BoosterActionEngine: 核心异步任务引擎
 * 负责监听编辑器内部状态并根据策略执行自动化联动
 */
class BoosterActionEngine {
    constructor() {
        this._active = false;
        this._heartbeat = null;
        this._tickRate = 800;
        this._namespace = "antigravity";
        this._featureKey = "automation.autoAccept";
    }

    start() {
        if (this._active) return;
        this._active = true;
        this._planNextPulse();
        if (outputChannel) outputChannel.appendLine("[Engine] Automation Active");
    }

    stop() {
        this._active = false;
        if (this._heartbeat) {
            clearTimeout(this._heartbeat);
            this._heartbeat = null;
        }
        if (outputChannel) outputChannel.appendLine("[Engine] Automation Idle");
    }

    _planNextPulse() {
        if (!this._active) return;
        this._heartbeat = setTimeout(async () => {
            await this._pulseCheck();
            this._planNextPulse();
        }, this._tickRate);
    }

    async _pulseCheck() {
        if (!this._active) return;

        const settings = vscode.workspace.getConfiguration(this._namespace);
        if (!settings.get(this._featureKey, false)) return;

        try {
            await vscode.commands.executeCommand('antigravity.agent.acceptAgentStep');
        } catch (e) { }

        try {
            await vscode.commands.executeCommand('antigravity.terminal.accept');
        } catch (e) { }
    }

    syncWithConfig() {
        const settings = vscode.workspace.getConfiguration(this._namespace);
        const isSwitchedOn = settings.get(this._featureKey, false);
        if (isSwitchedOn) this.start();
        else this.stop();
        return isSwitchedOn;
    }
}

function activate(context) {
    outputChannel = vscode.window.createOutputChannel("Antigravity Booster");
    outputChannel.appendLine("Antigravity Booster Helper v1.2.0 Activated");

    // 1. 初始化引擎
    automationService = new BoosterActionEngine();
    // 执行深度自检与状态对齐
    automationService.syncWithConfig();

    // 2. 状态栏
    statusBarItem = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 100);
    statusBarItem.command = 'antigravityBooster.showDetails';
    context.subscriptions.push(statusBarItem);

    // 3. Webview Provider
    const provider = new BoosterWebviewProvider(context.extensionUri);
    context.subscriptions.push(
        vscode.window.registerWebviewViewProvider('antigravityBoosterView', provider)
    );

    // 4. Quota Bridge 监听
    const appData = process.env.APPDATA || (process.platform == 'darwin' ? process.env.HOME + '/Library/Application Support' : process.env.HOME + "/.local/share");
    const bridgePath = path.join(appData, 'com.tz.antigravity-booster', 'quota_bridge.json');

    const updateState = () => {
        try {
            if (fs.existsSync(bridgePath)) {
                const content = fs.readFileSync(bridgePath, 'utf8');
                const data = JSON.parse(content);

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
                provider.updateData({ status: 'connected', models: data.models || [] });
            } else {
                statusBarItem.text = "$(rocket) Booster Linked";
                statusBarItem.show();
                provider.updateData({ status: 'disconnected', models: [] });
            }
        } catch (e) {
            outputChannel.appendLine(`Bridge Read Error: ${e}`);
            provider.updateData({ status: 'error', models: [] });
        }
    };

    updateState();
    fs.watchFile(bridgePath, { interval: 2000 }, () => updateState());

    // 5. 配置动态响应
    context.subscriptions.push(vscode.workspace.onDidChangeConfiguration(e => {
        if (e.affectsConfiguration('antigravity.automation.autoAccept')) {
            const isLive = automationService.syncWithConfig();
            provider.updateConfig({ autoAccept: isLive });
            outputChannel.appendLine(`[Sync] Policy Updated: ${isLive}`);
        }
    }));

    // 6. 命令注册
    context.subscriptions.push(vscode.commands.registerCommand('antigravityBooster.toggleAutoAccept', async (val) => {
        const settings = vscode.workspace.getConfiguration('antigravity');
        let nextState = !settings.get('automation.autoAccept', false);
        if (typeof val === 'boolean') nextState = val;

        await settings.update('automation.autoAccept', nextState, vscode.ConfigurationTarget.Global);
    }));

    context.subscriptions.push(vscode.commands.registerCommand('antigravity.openRule', async () => {
        const geminiRoot = path.join(os.homedir(), '.gemini');
        const rulePath = path.join(geminiRoot, 'GEMINI.md');
        if (!fs.existsSync(rulePath)) {
            if (!fs.existsSync(geminiRoot)) {
                try { fs.mkdirSync(geminiRoot, { recursive: true }); } catch (e) { }
            }
            try { fs.writeFileSync(rulePath, "# Antigravity Rules\n\n", "utf8"); } catch (e) { }
        }
        const doc = await vscode.workspace.openTextDocument(rulePath);
        await vscode.window.showTextDocument(doc);
    }));

    context.subscriptions.push(vscode.commands.registerCommand('antigravityBooster.showDetails', () => {
        vscode.window.showInformationMessage(`Antigravity Booster v${context.extension.packageJSON.version}`);
    }));
}

function deactivate() {
    if (automationService) automationService.stop();
    if (statusBarItem) statusBarItem.dispose();
    if (outputChannel) outputChannel.dispose();
}

/**
 * Booster Webview Provider
 */
class BoosterWebviewProvider {
    constructor(extensionUri) {
        this._extensionUri = extensionUri;
        this._view = undefined;
        this._lastData = { status: 'unknown', models: [] };
    }

    resolveWebviewView(webviewView, context, _token) {
        this._view = webviewView;
        webviewView.webview.options = { enableScripts: true, localResourceRoots: [this._extensionUri] };
        webviewView.webview.html = this._getHtmlForWebview(webviewView.webview);

        webviewView.webview.onDidReceiveMessage(message => {
            switch (message.type) {
                case 'webviewReady':
                    // 握手：立即提取核心配置
                    const settings = vscode.workspace.getConfiguration('antigravity');
                    const isFeatureOn = settings.get('automation.autoAccept', false);
                    this.updateConfig({ autoAccept: isFeatureOn });
                    this.updateData(this._lastData);
                    break;
                case 'toggleAutoAccept':
                    vscode.commands.executeCommand('antigravityBooster.toggleAutoAccept', message.val);
                    break;
                case 'openRules':
                    vscode.commands.executeCommand('antigravity.openRule');
                    break;
            }
        });
    }

    updateData(data) {
        this._lastData = data;
        if (this._view) this._view.webview.postMessage({ type: 'updateData', payload: data });
    }

    updateConfig(config) {
        if (this._view) this._view.webview.postMessage({ type: 'updateConfig', payload: config });
    }

    _getHtmlForWebview(webview) {
        return `<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <style>
        :root { --bg: var(--vscode-sideBar-background); --fg: var(--vscode-sideBar-foreground); --border: var(--vscode-widget-border); --button-bg: var(--vscode-button-background); --button-fg: var(--vscode-button-foreground); --button-hover: var(--vscode-button-hoverBackground); }
        body { padding: 16px; font-family: var(--vscode-font-family); color: var(--fg); background: var(--bg); user-select: none; }
        .section-title { font-size: 11px; font-weight: 600; text-transform: uppercase; opacity: 0.8; margin-bottom: 12px; letter-spacing: 0.5px; }
        .gauges { display: grid; grid-template-columns: repeat(2, 1fr); gap: 16px; margin-bottom: 24px; }
        .gauge-item { display: flex; flex-direction: column; align-items: center; }
        .gauge-circle { width: 48px; height: 48px; border-radius: 50%; display: flex; align-items: center; justify-content: center; background: conic-gradient(var(--color) var(--deg), var(--border) 0deg); margin-bottom: 6px; position: relative; }
        .gauge-circle::before { content: ''; position: absolute; inset: 4px; background: var(--bg); border-radius: 50%; }
        .gauge-val { position: relative; font-size: 10px; font-weight: bold; }
        .gauge-label { font-size: 11px; text-align: center; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; width: 100%; opacity: 0.9; }
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
    </style>
</head>
<body>
    <div class="section-title">模型余量池</div>
    <div id="gauges-container" class="gauges">
        <div style="font-size:12px; opacity:0.6; grid-column:span 2; text-align:center">获取中...</div>
    </div>
    
    <div class="section-title">控制面板</div>
    <div class="control-row">
        <div>
            <div class="control-label">自动同意 (Auto-Accept)</div>
            <div class="hint">自动同意 Agent 的操作</div>
        </div>
        <label class="switch">
            <input type="checkbox" id="auto-accept-toggle">
            <span class="slider"></span>
        </label>
    </div>
    <button class="btn" id="btn-rules">编辑规则 (GEMINI.md)</button>

    <script>
        const vscode = acquireVsCodeApi();
        const toggle = document.getElementById('auto-accept-toggle');

        vscode.postMessage({ type: 'webviewReady' });
        
        toggle.addEventListener('change', (e) => {
            vscode.postMessage({ type: 'toggleAutoAccept', val: e.target.checked });
        });
        document.getElementById('btn-rules').addEventListener('click', () => {
             vscode.postMessage({ type: 'openRules' });
        });

        function renderGauges(models) {
            const container = document.getElementById('gauges-container');
            if (!models?.length) return;
            container.innerHTML = '';
            
            models.forEach(m => {
                const percent = Math.floor(m.percentage);
                let color;
                if (percent <= 20) color = 'var(--vscode-charts-red)';
                else if (percent <= 40) color = 'var(--vscode-charts-orange)';
                else if (m.name.toLowerCase().includes('pro')) color = 'var(--vscode-charts-blue)';
                else if (m.name.toLowerCase().includes('flash')) color = 'var(--vscode-charts-green)';
                else color = 'var(--vscode-charts-purple)';

                const item = document.createElement('div');
                item.className = 'gauge-item';
                item.innerHTML = \`<div class="gauge-circle" style="--deg: \${percent * 3.6}deg; --color: \${color}"><div class="gauge-val">\${percent}%</div></div><div class="gauge-label">\${m.name}</div>\`;
                container.appendChild(item);
            });
        }

        window.addEventListener('message', event => {
            const msg = event.data;
            if (msg.type === 'updateData') renderGauges(msg.payload.models);
            if (msg.type === 'updateConfig') {
                if (toggle.checked !== msg.payload.autoAccept) toggle.checked = msg.payload.autoAccept;
            }
        });
    </script>
</body>
</html>`;
    }
}

module.exports = { activate, deactivate }
