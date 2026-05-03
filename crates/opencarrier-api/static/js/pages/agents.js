// OpenCarrier Agents Page — agent list, Hub install, chat inline
'use strict';

function agentsPage() {
  return {
    tab: 'agents',
    activeChatAgent: null,
    // -- Agents state --
    // -- Hub install modal --
    showHubModal: false,
    hubTemplates: [],
    hubLoading: false,
    hubError: '',
    hubInstalling: '',
    filterState: 'all',
    loading: true,
    loadError: '',
    tenants: [],
    selectedTenantId: '',
    tenantsLoaded: false,

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

    tenantNameFor(tid) {
      if (!tid) return 'admin';
      for (var i = 0; i < this.tenants.length; i++) {
        if (this.tenants[i].id === tid) return this.tenants[i].name;
      }
      return tid.substring(0, 8);
    },

    get agentsByTenant() {
      var groups = {};
      var agents = this.filteredAgents;
      for (var i = 0; i < agents.length; i++) {
        var a = agents[i];
        var tid = a.tenant_id || 'admin';
        if (!groups[tid]) groups[tid] = { id: tid, name: this.tenantNameFor(tid), agents: [] };
        groups[tid].agents.push(a);
      }
      var result = [];
      // admin group first (empty tenant_id or 'admin')
      var adminKeys = Object.keys(groups).filter(function(k) { return !k || k === 'admin'; });
      for (var i = 0; i < adminKeys.length; i++) {
        if (groups[adminKeys[i]]) { result.push(groups[adminKeys[i]]); delete groups[adminKeys[i]]; }
      }
      // remaining sorted by name
      var keys = Object.keys(groups).sort(function(a, b) { return groups[a].name.localeCompare(groups[b].name); });
      for (var i = 0; i < keys.length; i++) { result.push(groups[keys[i]]); }
      return result;
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

      // If a pending agent was set (e.g. from Hub install or redirect), open chat inline
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

  };
}
