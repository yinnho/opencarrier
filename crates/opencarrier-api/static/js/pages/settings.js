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
    // Tenants tab
    tenantsList: [],
    tenantsLoaded: false,
    showTenantModal: false,
    newTenantName: '',
    newTenantPass: '',
    tenantSaving: false,
    coreFeatures: [
      { name: '路径遍历防护', key: 'path_traversal', description: '阻止所有文件操作中的目录逃逸攻击。' },
      { name: 'SSRF 防护', key: 'ssrf_protection', description: '阻止对私有 IP 和云元数据端点的出站请求。' },
      { name: '基于能力的访问控制', key: 'capability_system', description: '默认拒绝的权限系统。' },
      { name: '子进程环境隔离', key: 'subprocess_isolation', description: '子进程仅继承安全的环境变量。' },
      { name: '安全响应头', key: 'security_headers', description: '每个 HTTP 响应包含 CSP、X-Frame-Options 等安全头。' }
    ],
    async loadSettings() {
      this.loading = true; this.loadError = '';
      try { await Promise.all([this.loadSysInfo(), this.loadUsage(), this.loadTools(), this.loadConfig(), this.loadBrainConfig(), this.loadProviderKeys()]); } catch(e) { this.loadError = e.message || '无法加载设置。'; }
      this.loading = false;
    },
    async loadData() { return this.loadSettings(); },
    async loadSysInfo() { try { var ver = await OpenCarrierAPI.get('/api/version'); var status = await OpenCarrierAPI.get('/api/status'); this.sysInfo = { version: ver.version || '-', platform: ver.platform || '-', arch: ver.arch || '-', uptime_seconds: status.uptime_seconds || 0, agent_count: status.agent_count || 0 }; } catch(e) { throw e; } },
    async loadUsage() { try { var data = await OpenCarrierAPI.get('/api/usage'); this.usageData = data.agents || []; } catch(e) { this.usageData = []; } },
    async loadTools() { try { var data = await OpenCarrierAPI.get('/api/tools'); this.tools = data.tools || []; } catch(e) { this.tools = []; } },
    async loadConfig() { try { this.config = await OpenCarrierAPI.get('/api/config'); } catch(e) { this.config = {}; } },
    async loadConfigSchema() { try { var r = await Promise.all([OpenCarrierAPI.get('/api/config/schema').catch(function(){return{}}), OpenCarrierAPI.get('/api/config')]); this.configSchema = r[0].sections || null; this.configValues = r[1] || {}; } catch(e) {} },
    async loadBrainConfig() { try { var data = await OpenCarrierAPI.get('/api/brain/config'); this.brainConfig = data; this.brainConfigRaw = JSON.stringify(data, null, 2); this.brainConfigError = ''; } catch(e) { this.brainConfig = null; this.brainConfigRaw = ''; this.brainConfigError = e.message || '加载大脑配置失败'; } },
    async saveBrainConfig() { this.brainConfigSaving = true; this.brainConfigError = ''; try { var json = JSON.parse(this.brainConfigRaw); await OpenCarrierAPI.put('/api/brain/config', json); this.brainConfig = json; OpenCarrierToast.success('大脑配置已保存'); } catch(e) { this.brainConfigError = e.message || '保存大脑配置失败'; OpenCarrierToast.error(this.brainConfigError); } this.brainConfigSaving = false; },
    async loadProviderKeys() { try { var data = await OpenCarrierAPI.get('/api/providers/keys'); this.providerKeys = data.providers || []; this.providerKeyInputs = {}; } catch(e) { this.providerKeys = []; } },
    async saveProviderKey(name) { var p = this.providerKeys.find(function(x){return x.name===name}); if (p && p.auth_type === 'jwt') { return this.saveProviderKeyJwt(name); } var key = (this.providerKeyInputs[name] || '').trim(); if (!key) { OpenCarrierToast.error('API 密钥不能为空'); return; } this.providerKeySaving[name] = true; try { await OpenCarrierAPI.post('/api/providers/' + name + '/key', { key: key }); await this.loadProviderKeys(); OpenCarrierToast.success('已保存 ' + name + ' 的 API 密钥'); } catch(e) { OpenCarrierToast.error('保存密钥失败: ' + (e.message || e)); } this.providerKeySaving[name] = false; },
    async saveProviderKeyJwt(name) { var p = this.providerKeys.find(function(x){return x.name===name}); if (!p) return; var params = {}; var hasValue = false; (p.params || []).forEach(function(param) { var val = (this.providerKeyInputs[name + '_' + param.name] || '').trim(); if (val) { params[param.name] = val; hasValue = true; } }.bind(this)); if (!hasValue) { OpenCarrierToast.error('请至少填写一项凭证'); return; } this.providerKeySaving[name] = true; try { await OpenCarrierAPI.post('/api/providers/' + name + '/key', { params: params }); await this.loadProviderKeys(); OpenCarrierToast.success('已保存 ' + name + ' 的凭证'); } catch(e) { OpenCarrierToast.error('保存凭证失败: ' + (e.message || e)); } this.providerKeySaving[name] = false; },
    async deleteProviderKey(name) { if (!confirm('确定删除 ' + name + ' 的凭证吗？')) return; try { await OpenCarrierAPI.del('/api/providers/' + name + '/key'); await this.loadProviderKeys(); OpenCarrierToast.success('已删除 ' + name + ' 的凭证'); } catch(e) { OpenCarrierToast.error('删除凭证失败: ' + (e.message || e)); } },
    isConfigDirty(s, f) { return this.configDirty[s + '.' + f] === true; },
    markConfigDirty(s, f) { this.configDirty[s + '.' + f] = true; },
    async saveConfigField(section, field, value) { var key = section + '.' + field; var meta = this.configSchema && this.configSchema[section]; var path = (meta && meta.root_level) ? field : key; this.configSaving[key] = true; try { await OpenCarrierAPI.post('/api/config/set', { path: path, value: value }); this.configDirty[key] = false; OpenCarrierToast.success('已保存 ' + field); } catch(e) { OpenCarrierToast.error('保存失败: ' + e.message); } this.configSaving[key] = false; },
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
    async startQrLogin() { this.wechatQrCode = null; this.wechatQrRaw = null; this.wechatQrStatus = null; this.wechatPolling = true; try { var res = await OpenCarrierAPI.get('/api/weixin/qrcode?tenant=default'); if (res.data && res.data.qrcode_img_content) { this.wechatQrCode = res.data.qrcode_img_content; this.wechatQrRaw = res.data.qrcode; this.pollQrStatus(); } else { OpenCarrierToast.error('QR code not available'); this.wechatPolling = false; } } catch(e) { OpenCarrierToast.error('Failed to get QR code: ' + (e.message || e)); this.wechatPolling = false; } },
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
    },
    // Tenants
    async loadTenantsTab() {
      try {
        var data = await OpenCarrierAPI.get('/api/tenants');
        this.tenantsList = Array.isArray(data) ? data : (data.tenants || []);
        this.tenantsLoaded = true;
      } catch(e) { this.tenantsList = []; this.tenantsLoaded = true; }
    },
    async createTenantInSettings() {
      this.tenantSaving = true;
      try {
        await OpenCarrierAPI.post('/api/tenants', { name: this.newTenantName.trim(), password: this.newTenantPass });
        OpenCarrierToast.success('租户已创建');
        this.showTenantModal = false;
        this.newTenantName = '';
        this.newTenantPass = '';
        await this.loadTenantsTab();
      } catch(e) { OpenCarrierToast.error('创建租户失败: ' + (e.message || e)); }
      this.tenantSaving = false;
    },
    async deleteTenantInSettings(id, name) {
      if (!confirm('确定要删除租户 "' + name + '" 吗？此操作不可撤销。')) return;
      try {
        await OpenCarrierAPI.del('/api/tenants/' + id);
        OpenCarrierToast.success('租户已删除');
        await this.loadTenantsTab();
      } catch(e) { OpenCarrierToast.error('删除租户失败: ' + (e.message || e)); }
    }
  };
}
