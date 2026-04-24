// OpenCarrier Settings Page — Config, Tools + Security tabs
'use strict';

function settingsPage() {
  return {
    tab: 'config',
    sysInfo: {},
    usageData: [],
    tools: [],
    config: {},
    toolSearch: '',
    loading: true,
    loadError: '',
    configSchema: null,
    configValues: {},
    configDirty: {},
    configSaving: {},
    brainConfig: null,
    brainConfigRaw: '',
    brainConfigSaving: false,
    brainConfigError: '',
    providerKeys: [],
    providerKeyInputs: {},
    providerKeySaving: {},
    securityData: null,
    secLoading: false,
    verifyingChain: false,
    chainResult: null,
    coreFeatures: [
      { name: 'Path Traversal Prevention', key: 'path_traversal', description: 'Blocks directory escape attacks in all file operations.' },
      { name: 'SSRF Protection', key: 'ssrf_protection', description: 'Blocks outbound requests to private IPs and cloud metadata endpoints.' },
      { name: 'Capability-Based Access Control', key: 'capability_system', description: 'Deny-by-default permission system.' },
      { name: 'Subprocess Environment Isolation', key: 'subprocess_isolation', description: 'Child processes inherit only safe environment variables.' },
      { name: 'Security Headers', key: 'security_headers', description: 'Every HTTP response includes CSP, X-Frame-Options, etc.' }
    ],
    async loadSettings() {
      this.loading = true; this.loadError = '';
      try { await Promise.all([this.loadSysInfo(), this.loadUsage(), this.loadTools(), this.loadConfig(), this.loadBrainConfig(), this.loadProviderKeys()]); } catch(e) { this.loadError = e.message || 'Could not load settings.'; }
      this.loading = false;
    },
    async loadData() { return this.loadSettings(); },
    async loadSysInfo() { try { var ver = await OpenCarrierAPI.get('/api/version'); var status = await OpenCarrierAPI.get('/api/status'); this.sysInfo = { version: ver.version || '-', platform: ver.platform || '-', arch: ver.arch || '-', uptime_seconds: status.uptime_seconds || 0, agent_count: status.agent_count || 0 }; } catch(e) { throw e; } },
    async loadUsage() { try { var data = await OpenCarrierAPI.get('/api/usage'); this.usageData = data.agents || []; } catch(e) { this.usageData = []; } },
    async loadTools() { try { var data = await OpenCarrierAPI.get('/api/tools'); this.tools = data.tools || []; } catch(e) { this.tools = []; } },
    async loadConfig() { try { this.config = await OpenCarrierAPI.get('/api/config'); } catch(e) { this.config = {}; } },
    async loadConfigSchema() { try { var r = await Promise.all([OpenCarrierAPI.get('/api/config/schema').catch(function(){return{}}), OpenCarrierAPI.get('/api/config')]); this.configSchema = r[0].sections || null; this.configValues = r[1] || {}; } catch(e) {} },
    async loadBrainConfig() { try { var data = await OpenCarrierAPI.get('/api/brain/config'); this.brainConfig = data; this.brainConfigRaw = JSON.stringify(data, null, 2); this.brainConfigError = ''; } catch(e) { this.brainConfig = null; this.brainConfigRaw = ''; this.brainConfigError = e.message || 'Failed to load brain config'; } },
    async saveBrainConfig() { this.brainConfigSaving = true; this.brainConfigError = ''; try { var json = JSON.parse(this.brainConfigRaw); await OpenCarrierAPI.put('/api/brain/config', json); this.brainConfig = json; OpenCarrierToast.success('Brain config saved'); } catch(e) { this.brainConfigError = e.message || 'Failed to save brain config'; OpenCarrierToast.error(this.brainConfigError); } this.brainConfigSaving = false; },
    async loadProviderKeys() { try { var data = await OpenCarrierAPI.get('/api/providers/keys'); this.providerKeys = data.providers || []; this.providerKeyInputs = {}; } catch(e) { this.providerKeys = []; } },
    async saveProviderKey(name) { var p = this.providerKeys.find(function(x){return x.name===name}); if (p && p.auth_type === 'jwt') { return this.saveProviderKeyJwt(name); } var key = (this.providerKeyInputs[name] || '').trim(); if (!key) { OpenCarrierToast.error('API key cannot be empty'); return; } this.providerKeySaving[name] = true; try { await OpenCarrierAPI.post('/api/providers/' + name + '/key', { key: key }); await this.loadProviderKeys(); OpenCarrierToast.success('API key saved for ' + name); } catch(e) { OpenCarrierToast.error('Failed to save key: ' + (e.message || e)); } this.providerKeySaving[name] = false; },
    async saveProviderKeyJwt(name) { var p = this.providerKeys.find(function(x){return x.name===name}); if (!p) return; var params = {}; var hasValue = false; (p.params || []).forEach(function(param) { var val = (this.providerKeyInputs[name + '_' + param.name] || '').trim(); if (val) { params[param.name] = val; hasValue = true; } }.bind(this)); if (!hasValue) { OpenCarrierToast.error('Please fill in at least one credential'); return; } this.providerKeySaving[name] = true; try { await OpenCarrierAPI.post('/api/providers/' + name + '/key', { params: params }); await this.loadProviderKeys(); OpenCarrierToast.success('Credentials saved for ' + name); } catch(e) { OpenCarrierToast.error('Failed to save credentials: ' + (e.message || e)); } this.providerKeySaving[name] = false; },
    async deleteProviderKey(name) { if (!confirm('Remove credentials for ' + name + '?')) return; try { await OpenCarrierAPI.del('/api/providers/' + name + '/key'); await this.loadProviderKeys(); OpenCarrierToast.success('Credentials removed for ' + name); } catch(e) { OpenCarrierToast.error('Failed to remove credentials: ' + (e.message || e)); } },
    isConfigDirty(s, f) { return this.configDirty[s + '.' + f] === true; },
    markConfigDirty(s, f) { this.configDirty[s + '.' + f] = true; },
    async saveConfigField(section, field, value) { var key = section + '.' + field; var meta = this.configSchema && this.configSchema[section]; var path = (meta && meta.root_level) ? field : key; this.configSaving[key] = true; try { await OpenCarrierAPI.post('/api/config/set', { path: path, value: value }); this.configDirty[key] = false; OpenCarrierToast.success('Saved ' + field); } catch(e) { OpenCarrierToast.error('Failed to save: ' + e.message); } this.configSaving[key] = false; },
    get filteredTools() { var q = this.toolSearch.toLowerCase().trim(); if (!q) return this.tools; return this.tools.filter(function(t) { return t.name.toLowerCase().indexOf(q) !== -1 || (t.description || '').toLowerCase().indexOf(q) !== -1; }); },
    formatUptime(secs) { if (!secs) return '-'; var h = Math.floor(secs / 3600); var m = Math.floor((secs % 3600) / 60); var s = secs % 60; if (h > 0) return h + 'h ' + m + 'm'; if (m > 0) return m + 'm ' + s + 's'; return s + 's'; },
    async loadSecurity() { this.secLoading = true; try { this.securityData = await OpenCarrierAPI.get('/api/security'); } catch(e) { this.securityData = null; } this.secLoading = false; },
    isActive(key) { if (!this.securityData) return true; var core = this.securityData.core_protections || {}; return core[key] !== undefined ? core[key] : true; },
    async verifyAuditChain() { this.verifyingChain = true; this.chainResult = null; try { var res = await OpenCarrierAPI.get('/api/audit/verify'); this.chainResult = res; } catch(e) { this.chainResult = { valid: false, error: e.message }; } this.verifyingChain = false; },
    // Unified channels
    channelsData: null,
    channelsLoading: false,
    // WeChat QR (from original wechat)
    wechatQrCode: null,
    wechatQrRaw: null,
    wechatQrStatus: null,
    wechatPolling: false,
    // WeCom form
    wecomForm: { name: '', mode: 'smartbot', corp_id: '', bot_id: '', secret: '', webhook_port: 8454, encoding_aes_key: '' },
    wecomSaving: false,
    // Feishu form
    feishuForm: { name: '', app_id: '', app_secret: '', brand: 'feishu' },
    feishuSaving: false,
    async loadChannels() { this.channelsLoading = true; try { this.channelsData = await OpenCarrierAPI.get('/api/channels/status'); } catch(e) { this.channelsData = null; OpenCarrierToast.error('Failed to load channels: ' + (e.message || e)); } this.channelsLoading = false; },
    wechatQrSrc: null,
    async startQrLogin() { this.wechatQrCode = null; this.wechatQrRaw = null; this.wechatQrStatus = null; this.wechatQrSrc = null; this.wechatPolling = true; try { var res = await OpenCarrierAPI.get('/api/weixin/qrcode?tenant=default'); if (res.data && res.data.qrcode_img_content) { var raw = res.data.qrcode_img_content; /* Handle both base64 data and URL responses */ if (typeof raw === 'string' && (raw.startsWith('http://') || raw.startsWith('https://'))) { this.wechatQrSrc = raw; } else { this.wechatQrCode = raw; } this.wechatQrRaw = res.data.qrcode; this.pollQrStatus(); } else { OpenCarrierToast.error('QR code not available'); this.wechatPolling = false; } } catch(e) { OpenCarrierToast.error('Failed to get QR code: ' + (e.message || e)); this.wechatPolling = false; } },
    async pollQrStatus() { if (!this.wechatQrRaw || !this.wechatPolling) return; try { var res = await OpenCarrierAPI.get('/api/weixin/qrcode-status?tenant=default&qrcode=' + encodeURIComponent(this.wechatQrRaw)); this.wechatQrStatus = res.status; if (res.status === 'confirmed') { this.wechatPolling = false; OpenCarrierToast.success('WeChat bound successfully!'); await this.loadChannels(); return; } if (res.status === 'expired') { this.wechatPolling = false; return; } } catch(e) { /* retry on network error */ } var self = this; setTimeout(function() { self.pollQrStatus(); }, 3000); },
    stopQrPoll() { this.wechatPolling = false; },
    async wecomAddTenant() { var f = this.wecomForm; if (!f.name.trim() || !f.corp_id.trim() || !f.secret.trim()) { OpenCarrierToast.error('Name, Corp ID, and Secret are required'); return; } this.wecomSaving = true; try { await OpenCarrierAPI.post('/api/channels/wecom/tenants', { name: f.name.trim(), mode: f.mode, corp_id: f.corp_id.trim(), bot_id: f.bot_id.trim(), secret: f.secret.trim(), webhook_port: f.webhook_port || 8454, encoding_aes_key: f.encoding_aes_key.trim() }); OpenCarrierToast.success('WeCom tenant added'); this.wecomForm = { name: '', mode: 'smartbot', corp_id: '', bot_id: '', secret: '', webhook_port: 8454, encoding_aes_key: '' }; await this.loadChannels(); } catch(e) { OpenCarrierToast.error('Failed to add tenant: ' + (e.message || e)); } this.wecomSaving = false; },
    async feishuAddTenant() { var f = this.feishuForm; if (!f.name.trim() || !f.app_id.trim() || !f.app_secret.trim()) { OpenCarrierToast.error('Name, App ID, and App Secret are required'); return; } this.feishuSaving = true; try { await OpenCarrierAPI.post('/api/channels/feishu/tenants', { name: f.name.trim(), app_id: f.app_id.trim(), app_secret: f.app_secret.trim(), brand: f.brand }); OpenCarrierToast.success('Feishu tenant added'); this.feishuForm = { name: '', app_id: '', app_secret: '', brand: 'feishu' }; await this.loadChannels(); } catch(e) { OpenCarrierToast.error('Failed to add tenant: ' + (e.message || e)); } this.feishuSaving = false; },
    formatWechatExpiry(secs) { if (!secs || secs <= 0) return '-'; var h = Math.floor(secs / 3600); var m = Math.floor((secs % 3600) / 60); return h > 0 ? h + 'h ' + m + 'm' : m + 'm'; },

    // ── Agent Bindings ──
    bindings: [],
    bindingsLoading: false,
    showBindForm: false,
    bindForm: { agent: '', channel: '', account_id: '', peer_id: '', guild_id: '', tenant_id: '' },
    bindSaving: false,
    bindingsTenants: [],

    async loadBindings() {
      this.bindingsLoading = true;
      try {
        var data = await OpenCarrierAPI.get('/api/bindings');
        this.bindings = data.bindings || [];
      } catch(e) { this.bindings = []; }
      this.bindingsLoading = false;
    },

    async loadChannelsFull() {
      await Promise.all([this.loadChannels(), this.loadBindings()]);
      if (Alpine.store('app').isAdmin()) {
        this.loadBindingsTenants();
      }
    },

    async loadBindingsTenants() {
      try {
        var data = await OpenCarrierAPI.get('/api/tenants');
        this.bindingsTenants = Array.isArray(data) ? data : [];
      } catch(e) { this.bindingsTenants = []; }
    },

    async addBinding() {
      var f = this.bindForm;
      if (!f.agent.trim()) { OpenCarrierToast.error('Please select an agent'); return; }
      if (!f.channel.trim()) { OpenCarrierToast.error('Channel type is required'); return; }
      this.bindSaving = true;
      try {
        var body = { agent: f.agent.trim(), match_rule: {} };
        if (f.channel.trim()) body.match_rule.channel = f.channel.trim();
        if (f.account_id.trim()) body.match_rule.account_id = f.account_id.trim();
        if (f.peer_id.trim()) body.match_rule.peer_id = f.peer_id.trim();
        if (f.guild_id.trim()) body.match_rule.guild_id = f.guild_id.trim();
        if (f.tenant_id.trim()) body.tenant_id = f.tenant_id.trim();
        await OpenCarrierAPI.post('/api/bindings', body);
        OpenCarrierToast.success('Binding created');
        this.bindForm = { agent: '', channel: '', account_id: '', peer_id: '', guild_id: '', tenant_id: '' };
        this.showBindForm = false;
        await this.loadBindings();
      } catch(e) {
        OpenCarrierToast.error('Failed to create binding: ' + (e.message || e));
      }
      this.bindSaving = false;
    },

    async removeBinding(index) {
      if (!confirm('Remove this binding?')) return;
      try {
        await OpenCarrierAPI.del('/api/bindings/' + index);
        OpenCarrierToast.success('Binding removed');
        await this.loadBindings();
      } catch(e) {
        OpenCarrierToast.error('Failed to remove binding: ' + (e.message || e));
      }
    }
  };
}
