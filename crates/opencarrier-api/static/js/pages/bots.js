// Bots page — manage bots across all channel plugins
'use strict';

function botsPage() {
  return {
    bots: [],
    agents: [],
    loading: true,
    loadError: '',

    // Plugin data
    localPlugins: [],   // installed plugins (with channel/tool info)
    hubPlugins: [],     // plugins available on hub
    channelPlugins: [], // merged: plugins that provide channels (for create modal)
    installState: {},   // { pluginName: 'idle'|'installing'|'done'|'error' }

    // Create modal state
    showCreateModal: false,
    createStep: 1,      // 1=select plugin, 2=config form
    createPlugin: null,  // selected plugin object
    createSaving: false,

    // WeCom SmartBot flow
    smartbotScode: '',
    smartbotAuthUrl: '',
    smartbotPolling: false,
    smartbotStatus: '',
    smartbotResult: null,
    smartbotPollTimer: null,

    // Bot form
    botForm: {
      name: '',
      platform: '',
      mode: 'smartbot',
      corp_id: '',
      bot_id: '',
      secret: '',
      bind_agent: '',
      app_id: '',
      app_secret: '',
      brand: 'feishu',
    },

    // Notification
    notifyMsg: '',
    notifyType: 'info',

    async loadData() {
      this.loading = true;
      this.loadError = '';
      try {
        var results = await Promise.allSettled([
          OpenCarrierAPI.get('/api/bots'),
          OpenCarrierAPI.get('/api/agents'),
          OpenCarrierAPI.get('/api/plugins'),
          OpenCarrierAPI.get('/api/plugins/search'),
        ]);

        this.bots = results[0].status === 'fulfilled' ? (results[0].value.bots || []) : [];
        this.agents = results[1].status === 'fulfilled'
          ? (Array.isArray(results[1].value) ? results[1].value : (results[1].value.agents || []))
          : [];

        // Local plugins with channel info
        this.localPlugins = results[2].status === 'fulfilled' ? (results[2].value.plugins || []) : [];

        // Hub plugins
        var hubRaw = results[3].status === 'fulfilled' ? (results[3].value.result || []) : [];
        if (hubRaw.plugins) hubRaw = hubRaw.plugins;

        this.hubPlugins = hubRaw;
        this.mergeChannelPlugins();
      } catch(e) {
        this.loadError = e.message || '加载失败';
      }
      this.loading = false;
    },

    // Merge local + hub plugins into channelPlugins (only those with channel capability)
    mergeChannelPlugins() {
      var map = {};
      var localNames = {};

      // Local plugins that have channels
      for (var i = 0; i < this.localPlugins.length; i++) {
        var p = this.localPlugins[i];
        localNames[p.name] = true;
        if (p.channels && p.channels.length > 0) {
          map[p.name] = {
            name: p.name,
            displayName: this.pluginDisplayName(p.name),
            description: p.description || '',
            channels: p.channels,
            installed: true,
            local: true,
          };
        }
      }

      // Known channel plugins from hub (not yet installed locally)
      var knownChannels = {
        'opencarrier-plugin-wecom': { display: '企业微信', channels: ['wecom'] },
        'opencarrier-plugin-feishu': { display: '飞书', channels: ['feishu'] },
        'opencarrier-plugin-weixin': { display: '个人微信', channels: ['weixin'] },
      };

      for (var i = 0; i < this.hubPlugins.length; i++) {
        var hp = this.hubPlugins[i];
        if (!localNames[hp.name]) {
          var known = knownChannels[hp.name];
          if (known) {
            map[hp.name] = {
              name: hp.name,
              displayName: known.display,
              description: hp.description || '',
              channels: known.channels,
              installed: false,
              local: false,
            };
          }
        }
      }

      this.channelPlugins = Object.values(map);
    },

    pluginDisplayName(name) {
      var m = {
        'opencarrier-plugin-wecom': '企业微信',
        'opencarrier-plugin-feishu': '飞书',
        'opencarrier-plugin-weixin': '个人微信',
      };
      return m[name] || name.replace('opencarrier-plugin-', '');
    },

    platformLabel(p) {
      var m = { wecom: '企业微信', feishu: '飞书', weixin: '微信', dingtalk: '钉钉' };
      return m[p] || p;
    },

    platformIcon(p) {
      var colors = { wecom: '#07c160', feishu: '#3370ff', weixin: '#07c160', dingtalk: '#0089ff' };
      var color = colors[p] || '#888';
      if (p === 'wecom') {
        return '<svg width="18" height="18" viewBox="0 0 24 24" fill="' + color + '"><path d="M8.5 13.5l2.5 3 5-7"/><circle cx="12" cy="12" r="10" fill="none" stroke="' + color + '" stroke-width="2"/></svg>';
      }
      if (p === 'feishu') {
        return '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="' + color + '" stroke-width="2"><path d="M12 2L2 7l10 5 10-5-10-5z"/><path d="M2 17l10 5 10-5"/><path d="M2 12l10 5 10-5"/></svg>';
      }
      if (p === 'weixin') {
        return '<svg width="18" height="18" viewBox="0 0 24 24" fill="' + color + '"><path d="M8.5 13.5l2.5 3 5-7"/><circle cx="12" cy="12" r="10" fill="none" stroke="' + color + '" stroke-width="2"/></svg>';
      }
      return '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="#888" stroke-width="2"><rect x="3" y="11" width="18" height="11" rx="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/></svg>';
    },

    botsByPlatform() {
      var groups = {};
      for (var i = 0; i < this.bots.length; i++) {
        var bot = this.bots[i];
        var p = bot.platform || 'other';
        if (!groups[p]) groups[p] = [];
        groups[p].push(bot);
      }
      return groups;
    },

    countByPlatform(p) {
      return this.bots.filter(function(b) { return b.platform === p; }).length;
    },

    modeLabel(mode) {
      var m = { smartbot: 'SmartBot', app: '企业应用', kf: '客服', ilink: 'iLink' };
      return m[mode] || mode || '-';
    },

    agentName(id) {
      if (!id) return '-';
      for (var i = 0; i < this.agents.length; i++) {
        if (this.agents[i].name === id || this.agents[i].id === id) return this.agents[i].name;
      }
      return id;
    },

    // ---- Install plugin from hub ----
    async installPlugin(plugin) {
      var name = plugin.name;
      this.installState[name] = 'installing';
      try {
        await OpenCarrierAPI.post('/api/plugins/install', { name: name });
        this.installState[name] = 'done';
        plugin.installed = true;
        plugin.local = true;
        // Refresh local plugins
        var res = await OpenCarrierAPI.get('/api/plugins');
        this.localPlugins = res.plugins || [];
        this.mergeChannelPlugins();
        OpenCarrierToast.success(this.pluginDisplayName(name) + ' 安装成功');
      } catch(e) {
        this.installState[name] = 'error';
        OpenCarrierToast.error('安装失败: ' + (e.message || '未知错误'));
      }
    },

    // ---- Create modal ----
    openCreateModal() {
      this.createStep = 1;
      this.createPlugin = null;
      this.smartbotScode = '';
      this.smartbotAuthUrl = '';
      this.smartbotPolling = false;
      this.smartbotStatus = '';
      this.smartbotResult = null;
      this.botForm = { name: '', platform: '', mode: 'smartbot', corp_id: '', bot_id: '', secret: '', bind_agent: '', app_id: '', app_secret: '', brand: 'feishu' };
      this.showCreateModal = true;
    },

    async selectPlugin(plugin) {
      // If not installed, install first
      if (!plugin.installed) {
        await this.installPlugin(plugin);
        if (!plugin.installed) return; // install failed
      }
      this.createPlugin = plugin;
      // Derive platform from channel type
      var ch = plugin.channels && plugin.channels[0] || '';
      this.botForm.platform = ch;
      this.createStep = 2;
    },

    backToStep1() {
      this.createStep = 1;
      this.createPlugin = null;
      this.smartbotPolling = false;
      if (this.smartbotPollTimer) { clearInterval(this.smartbotPollTimer); this.smartbotPollTimer = null; }
    },

    // ---- WeCom SmartBot flow ----
    async startSmartbotFlow() {
      if (this.smartbotPolling) return;
      if (this.smartbotPollTimer) { clearInterval(this.smartbotPollTimer); this.smartbotPollTimer = null; }

      this.smartbotStatus = '';
      this.smartbotAuthUrl = '';
      this.smartbotResult = null;
      try {
        var res = await OpenCarrierAPI.post('/api/bots/wecom/smartbot/generate', {});
        this.smartbotScode = res.scode;
        this.smartbotAuthUrl = res.auth_url;
        this.smartbotPolling = true;
        this.smartbotStatus = 'pending';

        var self = this;
        requestAnimationFrame(function() {
          setTimeout(function() { self.renderQR(); }, 50);
        });
        this.smartbotPollTimer = setInterval(function() { self.pollSmartbotResult(); }, 3000);
      } catch(e) {
        this.smartbotStatus = 'error';
        OpenCarrierToast.error(e.message || '生成链接失败');
      }
    },

    async pollSmartbotResult() {
      try {
        var res = await OpenCarrierAPI.get('/api/bots/wecom/smartbot/poll?scode=' + this.smartbotScode);
        if (res.status === 'success') {
          this.smartbotPolling = false;
          this.smartbotStatus = 'success';
          this.smartbotResult = res;
          this.botForm.bot_id = res.bot_id || '';
          this.botForm.secret = res.secret || '';
          if (this.smartbotPollTimer) { clearInterval(this.smartbotPollTimer); this.smartbotPollTimer = null; }
          OpenCarrierToast.success('企业微信机器人创建成功！');
        } else if (res.status === 'expired') {
          this.smartbotPolling = false;
          this.smartbotStatus = 'expired';
          if (this.smartbotPollTimer) { clearInterval(this.smartbotPollTimer); this.smartbotPollTimer = null; }
          OpenCarrierToast.error('链接已过期，请重新生成');
        }
      } catch(e) {
        // Silently retry
      }
    },

    copyAuthUrl() {
      if (this.smartbotAuthUrl) {
        navigator.clipboard.writeText(this.smartbotAuthUrl).then(function() {
          OpenCarrierToast.success('链接已复制');
        });
      }
    },

    renderQR() {
      var el = document.getElementById('smartbot-qr');
      if (!el || !this.smartbotAuthUrl) return;
      el.innerHTML = '';
      try {
        var qr = qrcode(0, 'M');
        qr.addData(this.smartbotAuthUrl);
        qr.make();
        var imgTag = qr.createImgTag(6, 8);
        el.innerHTML = imgTag;
        var img = el.querySelector('img');
        if (img) { img.style.width = '200px'; img.style.height = '200px'; img.style.imageRendering = 'pixelated'; }
      } catch(e) {
        el.innerHTML = '<p style="color:var(--danger);font-size:12px">二维码生成失败</p>';
      }
    },

    // ---- Create bot ----
    async createBot() {
      if (!this.botForm.name.trim()) {
        OpenCarrierToast.error('请输入机器人名称');
        return;
      }

      this.createSaving = true;
      var payload = {
        name: this.botForm.name.trim(),
        platform: this.botForm.platform,
      };

      // Platform-specific fields
      if (this.botForm.platform === 'wecom') {
        payload.mode = this.botForm.mode;
        if (this.botForm.corp_id) payload.corp_id = this.botForm.corp_id;
        if (this.botForm.bot_id) payload.bot_id = this.botForm.bot_id;
        if (this.botForm.secret) payload.secret = this.botForm.secret;
      } else if (this.botForm.platform === 'feishu') {
        payload.brand = this.botForm.brand;
        if (this.botForm.app_id) payload.app_id = this.botForm.app_id;
        if (this.botForm.app_secret) payload.app_secret = this.botForm.app_secret;
      } else if (this.botForm.platform === 'weixin') {
        // WeChat personal — no extra fields needed, authorization is QR scan at runtime
      }

      if (this.botForm.bind_agent) {
        payload.bind_agent = this.botForm.bind_agent;
      }

      try {
        await OpenCarrierAPI.post('/api/bots', payload);
        OpenCarrierToast.success('机器人已创建，重启后生效');
        this.showCreateModal = false;
        this.loadData();
      } catch(e) {
        OpenCarrierToast.error(e.message || '创建失败');
      }
      this.createSaving = false;
    },

    // ---- Bind / unbind ----
    async bindAgent(bot, agentName) {
      try {
        await OpenCarrierAPI.put('/api/bots/' + bot.id + '/bind', { agent_name: agentName });
        OpenCarrierToast.success('已绑定到 ' + agentName + '，重启后生效');
        this.loadData();
      } catch(e) {
        OpenCarrierToast.error(e.message || '绑定失败');
      }
    },

    async unbindAgent(bot) {
      try {
        await OpenCarrierAPI.del('/api/bots/' + bot.id + '/bind');
        OpenCarrierToast.success('已解绑，重启后生效');
        this.loadData();
      } catch(e) {
        OpenCarrierToast.error(e.message || '解绑失败');
      }
    },

    // ---- Delete ----
    async deleteBot(bot) {
      if (!confirm('确定删除机器人 "' + bot.tenant_name + '"？此操作不可撤销。')) return;
      try {
        await OpenCarrierAPI.del('/api/bots/' + bot.id);
        OpenCarrierToast.success('已删除，重启后生效');
        this.loadData();
      } catch(e) {
        OpenCarrierToast.error(e.message || '删除失败');
      }
    },
  };
}
