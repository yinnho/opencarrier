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
    spawnMode: 'wizard',
    spawning: false,
    spawnToml: '',
    filterState: 'all',
    loading: true,
    loadError: '',
    spawnForm: {
      name: '',
      provider: 'groq',
      model: 'llama-3.3-70b-versatile',
      systemPrompt: 'You are a helpful assistant.',
      profile: 'full',
      caps: { memory_read: true, memory_write: true, network: false, shell: false, agent_spawn: false }
    },
    // -- Admin tenant assignment --
    tenants: [],
    selectedTenantId: '',
    tenantsLoaded: false,

    // -- Multi-step wizard state --
    spawnStep: 1,
    spawnIdentity: { emoji: '', color: '#FF5C00', archetype: '' },
    selectedPreset: '',
    soulContent: '',
    emojiOptions: [
      '\u{1F916}', '\u{1F4BB}', '\u{1F50D}', '\u{270D}\uFE0F', '\u{1F4CA}', '\u{1F6E0}\uFE0F',
      '\u{1F4AC}', '\u{1F393}', '\u{1F310}', '\u{1F512}', '\u{26A1}', '\u{1F680}',
      '\u{1F9EA}', '\u{1F3AF}', '\u{1F4D6}', '\u{1F9D1}\u200D\u{1F4BB}', '\u{1F4E7}', '\u{1F3E2}',
      '\u{2764}\uFE0F', '\u{1F31F}', '\u{1F527}', '\u{1F4DD}', '\u{1F4A1}', '\u{1F3A8}'
    ],
    archetypeOptions: ['助手', '研究员', '程序员', '写手', '运维', '客服', '分析师', '自定义'],
    personalityPresets: [
      { id: 'professional', label: '专业', soul: 'Communicate in a clear, professional tone. Be direct and structured. Use formal language and data-driven reasoning. Prioritize accuracy over personality.' },
      { id: 'friendly', label: '友好', soul: 'Be warm, approachable, and conversational. Use casual language and show genuine interest in the user. Add personality to your responses while staying helpful.' },
      { id: 'technical', label: '技术', soul: 'Focus on technical accuracy and depth. Use precise terminology. Show your work and reasoning. Prefer code examples and structured explanations.' },
      { id: 'creative', label: '创意', soul: 'Be imaginative and expressive. Use vivid language, analogies, and unexpected connections. Encourage creative thinking and explore multiple perspectives.' },
      { id: 'concise', label: '简洁', soul: 'Be extremely brief and to the point. No filler, no pleasantries. Answer in the fewest words possible while remaining accurate and complete.' },
      { id: 'mentor', label: '导师', soul: 'Be patient and encouraging like a great teacher. Break down complex topics step by step. Ask guiding questions. Celebrate progress and build confidence.' }
    ],

    // -- Model switch --
    editingModel: false,

    // -- Templates state --
    tplTemplates: [],
    activeTplTab: 'browse',
    tplLoading: false,
    tplLoadError: '',
    selectedCategory: '全部',
    searchQuery: '',

    builtinTemplates: [],

    // ── Profile Descriptions ──
    profileDescriptions: {
      minimal: { label: '极简', desc: '只读文件访问' },
      coding: { label: '编程', desc: '文件 + Shell + 网络请求' },
      research: { label: '研究', desc: '网页搜索 + 文件读写' },
      messaging: { label: '消息', desc: '分身 + 记忆访问' },
      automation: { label: '自动化', desc: '除自定义外的所有工具' },
      balanced: { label: '均衡', desc: '通用工具集' },
      precise: { label: '精准', desc: '专注准确性的工具集' },
      creative: { label: '创意', desc: '全部工具，侧重创意' },
      full: { label: '完整', desc: '全部 35+ 工具' }
    },
    profileInfo: function(name) {
      return this.profileDescriptions[name] || { label: name, desc: '' };
    },

    // ── Tool Preview in Spawn Modal ──
    spawnProfiles: [],
    spawnProfilesLoaded: false,
    async loadSpawnProfiles() {
      if (this.spawnProfilesLoaded) return;
      try {
        var data = await OpenCarrierAPI.get('/api/profiles');
        this.spawnProfiles = data.profiles || [];
        this.spawnProfilesLoaded = true;
      } catch(e) { this.spawnProfiles = []; }
    },
    get selectedProfileTools() {
      var pname = this.spawnForm.profile;
      var match = this.spawnProfiles.find(function(p) { return p.name === pname; });
      if (match && match.tools) return match.tools.slice(0, 15);
      return [];
    },

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

    // -- Templates computed --
    get categories() {
      var cats = { '全部': true };
      this.builtinTemplates.forEach(function(t) { cats[t.category] = true; });
      this.tplTemplates.forEach(function(t) { if (t.category) cats[t.category] = true; });
      return Object.keys(cats);
    },

    get filteredBuiltins() {
      var self = this;
      return this.builtinTemplates.filter(function(t) {
        if (self.selectedCategory !== '全部' && t.category !== self.selectedCategory) return false;
        if (self.searchQuery) {
          var q = self.searchQuery.toLowerCase();
          if (t.name.toLowerCase().indexOf(q) === -1 &&
              t.description.toLowerCase().indexOf(q) === -1) return false;
        }
        return true;
      });
    },

    get filteredCustom() {
      var self = this;
      return this.tplTemplates.filter(function(t) {
        if (self.searchQuery) {
          var q = self.searchQuery.toLowerCase();
          if ((t.name || '').toLowerCase().indexOf(q) === -1 &&
              (t.description || '').toLowerCase().indexOf(q) === -1) return false;
        }
        return true;
      });
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

    async loadTemplates() {
      this.tplLoading = true;
      this.tplLoadError = '';
      try {
        var data = await OpenCarrierAPI.get('/api/templates');
        this.tplTemplates = data.templates || [];
      } catch(e) {
        this.tplTemplates = [];
        this.tplLoadError = e.message || '无法加载模板';
      }
      this.tplLoading = false;
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
      var customName = prompt('请输入分身名称:', name);
      if (!customName || !customName.trim()) return;
      customName = customName.trim();
      this.hubInstalling = name;
      try {
        var body = {};
        if (Alpine.store('app').isAdmin() && this.selectedTenantId) {
          body.tenant_id = this.selectedTenantId;
        }
        var res = await OpenCarrierAPI.post('/api/hub/templates/' + encodeURIComponent(name) + '/install', body);
        // Rename to custom name after installation
        if (res.agent_id && customName !== res.name) {
          await OpenCarrierAPI.patch('/api/agents/' + res.agent_id + '/config', { name: customName });
        }
        OpenCarrierToast.success('已安装 "' + customName + '"');
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
      this.spawnMode = 'wizard';
      this.spawnIdentity = { emoji: '', color: '#FF5C00', archetype: '' };
      this.selectedPreset = '';
      this.soulContent = '';
      this.spawnForm.name = '';
      this.spawnForm.provider = 'groq';
      this.spawnForm.model = 'llama-3.3-70b-versatile';
      this.spawnForm.systemPrompt = 'You are a helpful assistant.';
      this.spawnForm.profile = 'full';
      await this.loadTenantsForSpawn();
      try {
        var res = await fetch('/api/status');
        if (res.ok) {
          var status = await res.json();
          if (status.default_provider) this.spawnForm.provider = status.default_provider;
          if (status.default_model) this.spawnForm.model = status.default_model;
        }
      } catch(e) { /* keep hardcoded defaults */ }
    },

    nextStep() {
      if (this.spawnStep === 1 && !this.spawnForm.name.trim()) {
        OpenCarrierToast.warn('请输入分身名称');
        return;
      }
      if (this.spawnStep < 5) this.spawnStep++;
    },

    prevStep() {
      if (this.spawnStep > 1) this.spawnStep--;
    },

    selectPreset(preset) {
      this.selectedPreset = preset.id;
      this.soulContent = preset.soul;
    },

    generateToml() {
      var f = this.spawnForm;
      var si = this.spawnIdentity;
      var lines = [
        'name = "' + tomlBasicEscape(f.name) + '"',
        'module = "builtin:chat"'
      ];
      if (f.profile && f.profile !== 'custom') {
        lines.push('profile = "' + f.profile + '"');
      }
      lines.push('', '[model]');
      lines.push('provider = "' + f.provider + '"');
      lines.push('model = "' + f.model + '"');
      lines.push('system_prompt = """\n' + tomlMultilineEscape(f.systemPrompt) + '\n"""');
      if (f.profile === 'custom') {
        lines.push('', '[capabilities]');
        if (f.caps.memory_read) lines.push('memory_read = ["*"]');
        if (f.caps.memory_write) lines.push('memory_write = ["self.*"]');
        if (f.caps.network) lines.push('network = ["*"]');
        if (f.caps.shell) lines.push('shell = ["*"]');
        if (f.caps.agent_spawn) lines.push('agent_spawn = true');
      }
      return lines.join('\n');
    },

    async spawnAgent() {
      this.spawning = true;
      var toml = this.spawnMode === 'wizard' ? this.generateToml() : this.spawnToml;
      if (!toml.trim()) {
        this.spawning = false;
        OpenCarrierToast.warn('\u914d\u7f6e\u4e3a\u7a7a\uff0c\u8bf7\u5148\u8f93\u5165\u5206\u8eab\u914d\u7f6e');
        return;
      }

      try {
        var body = { manifest_toml: toml };
        if (Alpine.store('app').isAdmin() && this.selectedTenantId) {
          body.tenant_id = this.selectedTenantId;
        }
        var res = await OpenCarrierAPI.post('/api/agents', body);
        if (res.agent_id) {
          // Post-spawn: update identity + write SOUL.md if personality preset selected
          var patchBody = {};
          if (this.spawnIdentity.emoji) patchBody.emoji = this.spawnIdentity.emoji;
          if (this.spawnIdentity.color) patchBody.color = this.spawnIdentity.color;
          if (this.spawnIdentity.archetype) patchBody.archetype = this.spawnIdentity.archetype;
          if (this.selectedPreset) patchBody.vibe = this.selectedPreset;

          if (Object.keys(patchBody).length) {
            OpenCarrierAPI.patch('/api/agents/' + res.agent_id + '/config', patchBody).catch(function(e) { console.warn('Post-spawn config patch failed:', e.message); });
          }
          if (this.soulContent.trim()) {
            OpenCarrierAPI.put('/api/agents/' + res.agent_id + '/files/SOUL.md', { content: '# Soul\n' + this.soulContent }).catch(function(e) { console.warn('SOUL.md write failed:', e.message); });
          }

          this.showSpawnModal = false;
          this.spawnForm.name = '';
          this.spawnToml = '';
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

    // -- Template methods --
    async spawnFromTemplate(name) {
      var customName = prompt('请输入分身名称:', name);
      if (!customName || !customName.trim()) return;
      customName = customName.trim();
      try {
        var data = await OpenCarrierAPI.get('/api/templates/' + encodeURIComponent(name));
        if (data.manifest_toml) {
          // Override the name in the TOML manifest
          var toml = data.manifest_toml.replace(/^name\s*=\s*"[^"]*"/m, 'name = "' + tomlBasicEscape(customName) + '"');
          var res = await OpenCarrierAPI.post('/api/agents', { manifest_toml: toml });
          if (res.agent_id) {
            OpenCarrierToast.success('分身 "' + customName + '" 已创建');
            await Alpine.store('app').refreshAgents();
            this.chatWithAgent({ id: res.agent_id, name: customName, model_provider: '?', model_name: '?' });
          }
        }
      } catch(e) {
        OpenCarrierToast.error('从模板创建失败: ' + e.message);
      }
    },

    async spawnBuiltin(t) {
      var customName = prompt('请输入分身名称:', t.name);
      if (!customName || !customName.trim()) return;
      customName = customName.trim();
      var toml = 'name = "' + tomlBasicEscape(customName) + '"\n';
      toml += 'description = "' + tomlBasicEscape(t.description) + '"\n';
      toml += 'module = "builtin:chat"\n';
      toml += 'profile = "' + t.profile + '"\n\n';
      toml += '[model]\nprovider = "' + t.provider + '"\nmodel = "' + t.model + '"\n';
      toml += 'system_prompt = """\n' + tomlMultilineEscape(t.system_prompt) + '\n"""\n';

      try {
        var res = await OpenCarrierAPI.post('/api/agents', { manifest_toml: toml });
        if (res.agent_id) {
          OpenCarrierToast.success('分身 "' + customName + '" 已创建');
          await Alpine.store('app').refreshAgents();
          this.chatWithAgent({ id: res.agent_id, name: customName, model_provider: t.provider, model_name: t.model });
        }
      } catch(e) {
        OpenCarrierToast.error('创建分身失败: ' + e.message);
      }
    }
  };
}
