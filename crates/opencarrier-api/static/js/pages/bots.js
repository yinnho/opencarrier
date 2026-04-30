// Bots page — manage WeCom/Feishu/DingTalk bots
'use strict';

function botsPage() {
  return {
    bots: [],
    agents: [],
    loading: true,
    loadError: '',

    // Create modal state
    showCreateModal: false,
    createStep: 1,
    createPlatform: '',
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
      // Feishu fields
      app_id: '',
      app_secret: '',
      brand: 'feishu',
      // DingTalk fields
      app_key: '',
    },

    // Notification
    notifyMsg: '',
    notifyType: 'info',

    async loadData() {
      this.loading = true;
      this.loadError = '';
      try {
        var [botsRes, agentsRes] = await Promise.all([
          OpenCarrierAPI.get('/api/bots'),
          OpenCarrierAPI.get('/api/agents'),
        ]);
        this.bots = botsRes.bots || [];
        this.agents = Array.isArray(agentsRes) ? agentsRes : (agentsRes.agents || []);
      } catch(e) {
        this.loadError = e.message || '加载失败';
      }
      this.loading = false;
    },

    platformLabel(p) {
      var m = { wecom: '企业微信', feishu: '飞书', dingtalk: '钉钉' };
      return m[p] || p;
    },

    platformIcon(p) {
      var colors = { wecom: '#07c160', feishu: '#3370ff', dingtalk: '#0089ff' };
      var color = colors[p] || '#888';
      if (p === 'wecom') {
        return '<svg width="18" height="18" viewBox="0 0 24 24" fill="' + color + '"><path d="M8.5 13.5l2.5 3 5-7"/><circle cx="12" cy="12" r="10" fill="none" stroke="' + color + '" stroke-width="2"/></svg>';
      }
      if (p === 'feishu') {
        return '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="' + color + '" stroke-width="2"><path d="M12 2L2 7l10 5 10-5-10-5z"/><path d="M2 17l10 5 10-5"/><path d="M2 12l10 5 10-5"/></svg>';
      }
      if (p === 'dingtalk') {
        return '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="' + color + '" stroke-width="2"><circle cx="12" cy="12" r="10"/><path d="M12 8v4l3 3"/></svg>';
      }
      return '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="#888" stroke-width="2"><rect x="3" y="11" width="18" height="11" rx="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/></svg>';
    },

    platformBadgeClass(p) {
      var m = { wecom: 'badge-wecom', feishu: 'badge-feishu', dingtalk: 'badge-dingtalk' };
      return m[p] || '';
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
      var m = { smartbot: 'SmartBot', app: '企业应用', kf: '客服' };
      return m[mode] || mode || '-';
    },

    agentName(id) {
      if (!id) return '-';
      for (var i = 0; i < this.agents.length; i++) {
        if (this.agents[i].name === id || this.agents[i].id === id) return this.agents[i].name;
      }
      return id;
    },

    // ---- Create modal ----
    openCreateModal() {
      this.createStep = 1;
      this.createPlatform = '';
      this.smartbotScode = '';
      this.smartbotAuthUrl = '';
      this.smartbotPolling = false;
      this.smartbotStatus = '';
      this.smartbotResult = null;
      this.botForm = { name: '', platform: '', mode: 'smartbot', corp_id: '', bot_id: '', secret: '', bind_agent: '', app_id: '', app_secret: '', brand: 'feishu', app_key: '' };
      this.showCreateModal = true;
    },

    selectPlatform(p) {
      this.createPlatform = p;
      this.botForm.platform = p;
      this.createStep = 2;
    },

    backToStep1() {
      this.createStep = 1;
      this.createPlatform = '';
      this.smartbotPolling = false;
      if (this.smartbotPollTimer) { clearInterval(this.smartbotPollTimer); this.smartbotPollTimer = null; }
    },

    // ---- WeCom SmartBot flow ----
    async startSmartbotFlow() {
      if (this.smartbotPolling) return; // Guard against double-invocation
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
        // Render as a data URL for an <img>
        var imgTag = qr.createImgTag(6, 8);
        el.innerHTML = imgTag;
        // Style the img
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
        platform: this.createPlatform,
      };

      if (this.createPlatform === 'wecom') {
        payload.mode = this.botForm.mode;
        if (this.botForm.corp_id) payload.corp_id = this.botForm.corp_id;
        if (this.botForm.bot_id) payload.bot_id = this.botForm.bot_id;
        if (this.botForm.secret) payload.secret = this.botForm.secret;
      } else if (this.createPlatform === 'feishu') {
        payload.brand = this.botForm.brand;
        if (this.botForm.app_id) payload.app_id = this.botForm.app_id;
        if (this.botForm.app_secret) payload.app_secret = this.botForm.app_secret;
      } else if (this.createPlatform === 'dingtalk') {
        if (this.botForm.app_key) payload.app_key = this.botForm.app_key;
        if (this.botForm.app_secret) payload.app_secret = this.botForm.app_secret;
        if (this.botForm.corp_id) payload.corp_id = this.botForm.corp_id;
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
