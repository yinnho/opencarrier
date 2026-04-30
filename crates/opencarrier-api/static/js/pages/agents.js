// OpenCarrier Agents Page — Multi-step spawn wizard, detail view with tabs, file editor, personality presets
'use strict';

/** Escape a string for use inside TOML triple-quoted strings ("""\n...\n""").
 *  Backslashes are escaped, and runs of 3+ consecutive double-quotes are
 *  broken up so the TOML parser never sees an unintended closing delimiter.
 */
function tomlMultilineEscape(s) {
  return s.replace(/\\/g, '\\\\').replace(/"""/g, '""\\"');
}

/** Escape a string for use inside a TOML basic (single-line) string ("...").
 *  Backslashes, double-quotes, and common control chars are escaped.
 */
function tomlBasicEscape(s) {
  return s.replace(/\\/g, '\\\\').replace(/"/g, '\\"').replace(/\n/g, '\\n').replace(/\r/g, '\\r').replace(/\t/g, '\\t');
}

function agentsPage() {
  return {
    tab: 'agents',
    activeChatAgent: null,
    // -- Agents state --
    showSpawnModal: false,
    // -- Hub install modal --
    showHubModal: false,
    hubTemplates: [],
    hubLoading: false,
    hubError: '',
    hubInstalling: '',
    spawning: false,
    filterState: 'all',
    loading: true,
    loadError: '',
    spawnForm: {
      name: '',
      provider: 'groq',
      model: 'llama-3.3-70b-versatile',
      systemPrompt: 'You are a helpful assistant.'
    },
    tenants: [],
    selectedTenantId: '',
    tenantsLoaded: false,

    spawnStep: 1,

    get agents() { return Alpine.store('app').agents; },

    get filteredAgents() {
      var self = this;
      var result = this.agents;
      // Filter by tenant if admin selected one
      if (Alpine.store('app').isAdmin() && this.selectedTenantId) {
        result = result.filter(function(a) {
          return a.tenant_id === self.selectedTenantId;
        });
      }
      var f = this.filterState;
      if (f === 'all') return result;
      return result.filter(function(a) { return a.state.toLowerCase() === f; });
    },

    get runningCount() {
      return this.agents.filter(function(a) { return a.state === 'Running'; }).length;
    },

    get stoppedCount() {
      return this.agents.filter(function(a) { return a.state !== 'Running'; }).length;
    },


    async init() {
      var self = this;
      this.loading = true;
      this.loadError = '';
      try {
        await Alpine.store('app').refreshAgents();
        await this.loadTenantsForSpawn();
      } catch(e) {
        this.loadError = e.message || '无法加载分身列表，请确认守护进程是否正在运行';
      }
      this.loading = false;

      // If a pending agent was set (e.g. from wizard or redirect), open chat inline
      var store = Alpine.store('app');
      if (store.pendingAgent) {
        this.activeChatAgent = store.pendingAgent;
      }
      // Watch for future pendingAgent changes
      this.$watch('$store.app.pendingAgent', function(agent) {
        if (agent) {
          self.activeChatAgent = agent;
        }
      });
    },

    async loadData() {
      this.loading = true;
      this.loadError = '';
      try {
        await Alpine.store('app').refreshAgents();
      } catch(e) {
        this.loadError = e.message || '无法加载分身列表';
      }
      this.loading = false;
    },


    chatWithAgent(agent) {
      Alpine.store('app').pendingAgent = agent;
      this.activeChatAgent = agent;
    },

    closeChat() {
      this.activeChatAgent = null;
      OpenCarrierAPI.wsDisconnect();
    },

    deleteAgent(agent) {
      var self = this;
      OpenCarrierToast.confirm('删除分身', '确定永久删除分身 "' + agent.name + '" 吗？此操作不可撤销。', async function() {
        try {
          await OpenCarrierAPI.del('/api/agents/' + agent.id);
          OpenCarrierToast.success('分身 "' + agent.name + '" 已删除');
          await Alpine.store('app').refreshAgents();
        } catch(e) {
          OpenCarrierToast.error('删除分身失败: ' + e.message);
        }
      });
    },

    killAllAgents() {
      var list = this.filteredAgents;
      if (!list.length) return;
      OpenCarrierToast.confirm('停止所有分身', '确定停止 ' + list.length + ' 个分身吗？当前运行将被取消。', async function() {
        var errors = [];
        for (var i = 0; i < list.length; i++) {
          try {
            await OpenCarrierAPI.post('/api/agents/' + list[i].id + '/stop');
          } catch(e) { errors.push(list[i].name + ': ' + e.message); }
        }
        await Alpine.store('app').refreshAgents();
        if (errors.length) {
          OpenCarrierToast.error('部分分身停止失败: ' + errors.join(', '));
        } else {
          OpenCarrierToast.success(list.length + ' 个分身已停止');
        }
      });
    },

    async loadTenantsForSpawn() {
      if (this.tenantsLoaded) return;
      if (!Alpine.store('app').isAdmin()) return;
      try {
        var data = await OpenCarrierAPI.get('/api/tenants');
        this.tenants = Array.isArray(data) ? data : [];
        this.tenantsLoaded = true;
      } catch(e) { this.tenants = []; }
    },

    // ── Hub install modal ──
    async openHubModal() {
      this.showHubModal = true;
      this.hubTemplates = [];
      this.hubError = '';
      await this.loadTenantsForSpawn();
      await this.loadHubTemplates();
    },

    closeHubModal() {
      this.showHubModal = false;
      this.hubTemplates = [];
      this.hubError = '';
      this.hubInstalling = '';
    },

    async loadHubTemplates() {
      this.hubLoading = true;
      this.hubError = '';
      try {
        var data = await OpenCarrierAPI.get('/api/hub/templates');
        this.hubTemplates = (data.templates || []).map(function(t) {
          return {
            name: t.name || '',
            description: t.description || '',
            version: t.latest_version || '1',
            downloads: t.download_count || 0,
            rating: t.rating_avg || 0,
            author: t.author || ''
          };
        });
      } catch(e) {
        this.hubError = e.message || '加载 Hub 模板失败';
        this.hubTemplates = [];
      }
      this.hubLoading = false;
    },

    async installHubTemplate(name) {
      this.hubInstalling = name;
      try {
        var body = {};
        if (Alpine.store('app').isAdmin() && this.selectedTenantId) {
          body.tenant_id = this.selectedTenantId;
        }
        var res = await OpenCarrierAPI.post('/api/hub/templates/' + encodeURIComponent(name) + '/install', body);
        OpenCarrierToast.success('已安装 "' + name + '"');
        this.closeHubModal();
        await Alpine.store('app').refreshAgents();
      } catch(e) {
        OpenCarrierToast.error('安装失败: ' + e.message);
      }
      this.hubInstalling = '';
    },

    // ── Multi-step wizard navigation ──
    async openSpawnWizard() {
      this.showSpawnModal = true;
      this.spawnStep = 1;
      this.spawnForm.name = '';
      this.spawnForm.provider = 'groq';
      this.spawnForm.model = 'llama-3.3-70b-versatile';
      this.spawnForm.systemPrompt = 'You are a helpful assistant.';
      await this.loadTenantsForSpawn();
      try {
        var res = await fetch('/api/status');
        if (res.ok) {
          var status = await res.json();
          if (status.default_provider) this.spawnForm.provider = status.default_provider;
          if (status.default_model) this.spawnForm.model = status.default_model;
        }
      } catch(e) {}
    },

    nextStep() {
      if (this.spawnStep === 1 && !this.spawnForm.name.trim()) {
        OpenCarrierToast.warn('请输入分身名称');
        return;
      }
      if (this.spawnStep < 2) this.spawnStep++;
    },

    prevStep() {
      if (this.spawnStep > 1) this.spawnStep--;
    },

    generateToml() {
      var f = this.spawnForm;
      var lines = [
        'name = "' + tomlBasicEscape(f.name) + '"',
        'module = "builtin:chat"',
        '',
        '[model]',
        'provider = "' + f.provider + '"',
        'model = "' + f.model + '"',
        'system_prompt = """',
        tomlMultilineEscape(f.systemPrompt),
        '"""'
      ];
      return lines.join('\n');
    },

    async spawnAgent() {
      this.spawning = true;
      var toml = this.generateToml();
      try {
        var body = { manifest_toml: toml };
        if (Alpine.store('app').isAdmin() && this.selectedTenantId) {
          body.tenant_id = this.selectedTenantId;
        }
        var res = await OpenCarrierAPI.post('/api/agents', body);
        if (res.agent_id) {
          this.showSpawnModal = false;
          this.spawnForm.name = '';
          this.spawnStep = 1;
          OpenCarrierToast.success('分身 "' + (res.name || '新分身') + '" 已创建');
          await Alpine.store('app').refreshAgents();
          this.chatWithAgent({ id: res.agent_id, name: res.name, model_provider: '?', model_name: '?' });
        } else {
          OpenCarrierToast.error('创建失败: ' + (res.error || '未知错误'));
        }
      } catch(e) {
        OpenCarrierToast.error('创建分身失败: ' + e.message);
      }
      this.spawning = false;
    },

  };
}
