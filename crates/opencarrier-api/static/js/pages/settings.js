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
      try { await Promise.all([this.loadSysInfo(), this.loadUsage(), this.loadTools(), this.loadConfig()]); } catch(e) { this.loadError = e.message || 'Could not load settings.'; }
      this.loading = false;
    },
    async loadData() { return this.loadSettings(); },
    async loadSysInfo() { try { var ver = await OpenCarrierAPI.get('/api/version'); var status = await OpenCarrierAPI.get('/api/status'); this.sysInfo = { version: ver.version || '-', platform: ver.platform || '-', arch: ver.arch || '-', uptime_seconds: status.uptime_seconds || 0, agent_count: status.agent_count || 0 }; } catch(e) { throw e; } },
    async loadUsage() { try { var data = await OpenCarrierAPI.get('/api/usage'); this.usageData = data.agents || []; } catch(e) { this.usageData = []; } },
    async loadTools() { try { var data = await OpenCarrierAPI.get('/api/tools'); this.tools = data.tools || []; } catch(e) { this.tools = []; } },
    async loadConfig() { try { this.config = await OpenCarrierAPI.get('/api/config'); } catch(e) { this.config = {}; } },
    async loadConfigSchema() { try { var r = await Promise.all([OpenCarrierAPI.get('/api/config/schema').catch(function(){return{}}), OpenCarrierAPI.get('/api/config')]); this.configSchema = r[0].sections || null; this.configValues = r[1] || {}; } catch(e) {} },
    isConfigDirty(s, f) { return this.configDirty[s + '.' + f] === true; },
    markConfigDirty(s, f) { this.configDirty[s + '.' + f] = true; },
    async saveConfigField(section, field, value) { var key = section + '.' + field; var meta = this.configSchema && this.configSchema[section]; var path = (meta && meta.root_level) ? field : key; this.configSaving[key] = true; try { await OpenCarrierAPI.post('/api/config/set', { path: path, value: value }); this.configDirty[key] = false; OpenCarrierToast.success('Saved ' + field); } catch(e) { OpenCarrierToast.error('Failed to save: ' + e.message); } this.configSaving[key] = false; },
    get filteredTools() { var q = this.toolSearch.toLowerCase().trim(); if (!q) return this.tools; return this.tools.filter(function(t) { return t.name.toLowerCase().indexOf(q) !== -1 || (t.description || '').toLowerCase().indexOf(q) !== -1; }); },
    formatUptime(secs) { if (!secs) return '-'; var h = Math.floor(secs / 3600); var m = Math.floor((secs % 3600) / 60); var s = secs % 60; if (h > 0) return h + 'h ' + m + 'm'; if (m > 0) return m + 'm ' + s + 's'; return s + 's'; },
    async loadSecurity() { this.secLoading = true; try { this.securityData = await OpenCarrierAPI.get('/api/security'); } catch(e) { this.securityData = null; } this.secLoading = false; },
    isActive(key) { if (!this.securityData) return true; var core = this.securityData.core_protections || {}; return core[key] !== undefined ? core[key] : true; },
    async verifyAuditChain() { this.verifyingChain = true; this.chainResult = null; try { var res = await OpenCarrierAPI.get('/api/audit/verify'); this.chainResult = res; } catch(e) { this.chainResult = { valid: false, error: e.message }; } this.verifyingChain = false; }
  };
}
