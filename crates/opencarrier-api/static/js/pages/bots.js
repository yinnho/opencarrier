// Bots page — platform-centric bot management with card-based UI
'use strict';

function botsPage() {
  return {
    // Data
    bots: [],
    agents: [],
    loading: true,
    loadError: '',

    // Modal state
    showCreateModal: false,
    showEditModal: false,
    createStep: 1,
    createPlugin: null,
    createSaving: false,
    editSaving: false,
    editingBot: null,

    // Platform forms
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

    // WeCom SmartBot
    smartbotScode: '',
    smartbotAuthUrl: '',
    smartbotPolling: false,
    smartbotStatus: '',
    smartbotResult: null,
    smartbotPollTimer: null,

    // Weixin QR
    weixinQrCode: null,
    weixinQrRaw: null,
    weixinQrStatus: null,
    weixinPolling: false,
    weixinPollTimer: null,

    // ------------------------------------------------------------------
    // Lifecycle
    // ------------------------------------------------------------------

    async loadData() {
      this.loading = true;
      this.loadError = '';
      try {
        const [botsRes, agentsRes] = await Promise.all([
          OpenCarrierAPI.get('/api/bots'),
          OpenCarrierAPI.get('/api/agents'),
        ]);
        this.bots = botsRes.bots || [];
        this.agents = Array.isArray(agentsRes) ? agentsRes : (agentsRes.agents || []);
      } catch (e) {
        this.loadError = e.message || '加载失败';
      }
      this.loading = false;
    },

    // ------------------------------------------------------------------
    // Computed helpers
    // ------------------------------------------------------------------

    botsByPlatform() {
      const groups = {};
      for (const bot of this.bots) {
        const p = bot.platform || 'other';
        if (!groups[p]) groups[p] = [];
        groups[p].push(bot);
      }
      return groups;
    },

    platformConfig(p) {
      const configs = {
        weixin:  { label: '个人微信', color: '#07c160', icon: weixinIcon },
        wecom:   { label: '企业微信', color: '#07c160', icon: wecomIcon },
        feishu:  { label: '飞书',     color: '#3370ff', icon: feishuIcon },
        dingtalk:{ label: '钉钉',     color: '#0089ff', icon: dingtalkIcon },
      };
      return configs[p] || { label: p, color: '#888', icon: defaultIcon };
    },

    modeLabel(mode) {
      const m = { smartbot: 'SmartBot', app: '企业应用', kf: '客服', ilink: 'iLink' };
      return m[mode] || mode || '-';
    },

    // -- Compatibility wrappers for existing HTML ----------------------------

    platformLabel(p) { return this.platformConfig(p).label; },
    platformIcon(p) { return this.platformConfig(p).icon; },

    get channelPlugins() {
      // Built-in channels are always "installed"
      return [
        { name: 'wecom',  displayName: '企业微信', installed: true, channels: ['wecom'] },
        { name: 'feishu', displayName: '飞书',     installed: true, channels: ['feishu'] },
        { name: 'weixin', displayName: '个人微信', installed: true, channels: ['weixin'] },
      ];
    },

    selectPlugin(plugin) {
      const platform = plugin.channels && plugin.channels[0] || plugin.name;
      this.selectPlatform(platform);
    },

    installState: {},

    agentName(id) {
      if (!id) return null;
      const a = this.agents.find(x => x.id === id || x.name === id);
      return a ? a.name : id.substring(0, 8);
    },

    agentAvatar(id) {
      if (!id) return '';
      const a = this.agents.find(x => x.id === id);
      return a?.identity?.avatar_url || '';
    },

    // ------------------------------------------------------------------
    // Create modal
    // ------------------------------------------------------------------

    openCreateModal() {
      this.resetForm();
      this.createStep = 1;
      this.createPlugin = null;
      this.showCreateModal = true;
    },

    closeCreateModal() {
      this.showCreateModal = false;
      this.stopSmartbotPoll();
      this.stopWeixinPoll();
    },

    resetForm() {
      this.botForm = {
        name: '', platform: '', mode: 'smartbot',
        corp_id: '', bot_id: '', secret: '', bind_agent: '',
        app_id: '', app_secret: '', brand: 'feishu',
      };
      this.smartbotScode = '';
      this.smartbotAuthUrl = '';
      this.smartbotPolling = false;
      this.smartbotStatus = '';
      this.smartbotResult = null;
      this.weixinQrCode = null;
      this.weixinQrRaw = null;
      this.weixinQrStatus = null;
      this.weixinPolling = false;
      this.createPlugin = null;
    },

    selectPlatform(platform) {
      this.botForm.platform = platform;
      this.createStep = 2;
      const cfg = this.platformConfig(platform);
      this.createPlugin = { displayName: cfg.label };
    },

    backToStep1() {
      this.createStep = 1;
      this.createPlugin = null;
      this.stopSmartbotPoll();
      this.stopWeixinPoll();
    },

    // ------------------------------------------------------------------
    // Create / Edit bot
    // ------------------------------------------------------------------

    async createBot() {
      if (!this.botForm.name.trim()) {
        OpenCarrierToast.error('请输入机器人名称');
        return;
      }

      this.createSaving = true;
      const payload = {
        name: this.botForm.name.trim(),
        platform: this.botForm.platform,
      };

      if (this.botForm.platform === 'wecom') {
        payload.mode = this.botForm.mode;
        if (this.botForm.corp_id) payload.corp_id = this.botForm.corp_id;
        if (this.botForm.bot_id) payload.bot_id = this.botForm.bot_id;
        if (this.botForm.secret) payload.secret = this.botForm.secret;
      } else if (this.botForm.platform === 'feishu') {
        payload.brand = this.botForm.brand;
        if (this.botForm.app_id) payload.app_id = this.botForm.app_id;
        if (this.botForm.app_secret) payload.app_secret = this.botForm.app_secret;
      }

      if (this.botForm.bind_agent) {
        payload.bind_agent = this.botForm.bind_agent;
      }

      try {
        await OpenCarrierAPI.post('/api/bots', payload);
        OpenCarrierToast.success('机器人已创建');
        this.closeCreateModal();
        this.loadData();
      } catch (e) {
        OpenCarrierToast.error(e.message || '创建失败');
      }
      this.createSaving = false;
    },

    openEditModal(bot) {
      this.editingBot = bot;
      this.botForm = {
        name: bot.tenant_name || '',
        platform: bot.platform || '',
        mode: bot.mode || 'smartbot',
        corp_id: bot.corp_id || '',
        bot_id: bot.bot_id || '',
        secret: '',
        bind_agent: bot.bind_agent || '',
        app_id: bot.app_id || '',
        app_secret: '',
        brand: bot.brand || 'feishu',
      };
      this.showEditModal = true;
    },

    closeEditModal() {
      this.showEditModal = false;
      this.editingBot = null;
    },

    async saveEdit() {
      if (!this.editingBot) return;
      this.editSaving = true;

      const payload = {};
      if (this.botForm.name) payload.name = this.botForm.name;
      if (this.botForm.mode) payload.mode = this.botForm.mode;
      if (this.botForm.corp_id) payload.corp_id = this.botForm.corp_id;
      if (this.botForm.bot_id) payload.bot_id = this.botForm.bot_id;
      if (this.botForm.secret) payload.secret = this.botForm.secret;
      if (this.botForm.app_id) payload.app_id = this.botForm.app_id;
      if (this.botForm.app_secret) payload.app_secret = this.botForm.app_secret;
      if (this.botForm.brand) payload.brand = this.botForm.brand;

      try {
        await OpenCarrierAPI.put('/api/bots/' + this.editingBot.id, payload);
        OpenCarrierToast.success('已保存');
        this.closeEditModal();
        this.loadData();
      } catch (e) {
        OpenCarrierToast.error(e.message || '保存失败');
      }
      this.editSaving = false;
    },

    // ------------------------------------------------------------------
    // Bind / unbind
    // ------------------------------------------------------------------

    async bindAgent(bot, agentName) {
      try {
        await OpenCarrierAPI.put('/api/bots/' + bot.id + '/bind', { agent_name: agentName });
        OpenCarrierToast.success('已绑定到 ' + this.agentName(agentName));
        this.loadData();
      } catch (e) {
        OpenCarrierToast.error(e.message || '绑定失败');
      }
    },

    async unbindAgent(bot) {
      try {
        await OpenCarrierAPI.del('/api/bots/' + bot.id + '/bind');
        OpenCarrierToast.success('已解绑');
        this.loadData();
      } catch (e) {
        OpenCarrierToast.error(e.message || '解绑失败');
      }
    },

    // ------------------------------------------------------------------
    // Delete
    // ------------------------------------------------------------------

    async deleteBot(bot) {
      if (!confirm('确定删除机器人 "' + bot.tenant_name + '"？此操作不可撤销。')) return;
      try {
        await OpenCarrierAPI.del('/api/bots/' + bot.id);
        OpenCarrierToast.success('已删除');
        this.loadData();
      } catch (e) {
        OpenCarrierToast.error(e.message || '删除失败');
      }
    },

    // ------------------------------------------------------------------
    // WeCom SmartBot
    // ------------------------------------------------------------------

    async startSmartbotFlow() {
      if (this.smartbotPolling) return;
      this.stopSmartbotPoll();
      this.smartbotStatus = '';
      this.smartbotAuthUrl = '';
      this.smartbotResult = null;

      try {
        const res = await OpenCarrierAPI.post('/api/bots/wecom/smartbot/generate', {});
        this.smartbotScode = res.scode;
        this.smartbotAuthUrl = res.auth_url;
        this.smartbotPolling = true;
        this.smartbotStatus = 'pending';
        requestAnimationFrame(() => setTimeout(() => this.renderQR('smartbot-qr', this.smartbotAuthUrl), 50));
        this.smartbotPollTimer = setInterval(() => this.pollSmartbotResult(), 3000);
      } catch (e) {
        this.smartbotStatus = 'error';
        OpenCarrierToast.error(e.message || '生成链接失败');
      }
    },

    async pollSmartbotResult() {
      try {
        const res = await OpenCarrierAPI.get('/api/bots/wecom/smartbot/poll?scode=' + this.smartbotScode);
        if (res.status === 'success') {
          this.smartbotPolling = false;
          this.smartbotStatus = 'success';
          this.smartbotResult = res;
          this.botForm.bot_id = res.bot_id || '';
          this.botForm.secret = res.secret || '';
          this.stopSmartbotPoll();
          OpenCarrierToast.success('企业微信机器人创建成功！');
        } else if (res.status === 'expired') {
          this.smartbotPolling = false;
          this.smartbotStatus = 'expired';
          this.stopSmartbotPoll();
          OpenCarrierToast.error('链接已过期，请重新生成');
        }
      } catch (e) { /* silently retry */ }
    },

    stopSmartbotPoll() {
      this.smartbotPolling = false;
      if (this.smartbotPollTimer) {
        clearInterval(this.smartbotPollTimer);
        this.smartbotPollTimer = null;
      }
    },

    copyAuthUrl() {
      if (this.smartbotAuthUrl) {
        navigator.clipboard.writeText(this.smartbotAuthUrl).then(() => {
          OpenCarrierToast.success('链接已复制');
        });
      }
    },

    // ------------------------------------------------------------------
    // Weixin QR
    // ------------------------------------------------------------------

    async startWeixinQrLogin() {
      this.weixinQrCode = null;
      this.weixinQrRaw = null;
      this.weixinQrStatus = null;
      this.weixinPolling = true;
      try {
        const res = await OpenCarrierAPI.get('/api/weixin/qrcode?tenant=' + encodeURIComponent(this.botForm.name.trim()));
        if (res.data?.qrcode_img_content) {
          this.weixinQrCode = res.data.qrcode_img_content;
          this.weixinQrRaw = res.data.qrcode || null;
          this.weixinQrStatus = 'pending';
          requestAnimationFrame(() => setTimeout(() => this.renderWeixinQR(), 50));
          this.pollWeixinQrStatus();
        } else {
          OpenCarrierToast.error('获取二维码失败');
          this.weixinPolling = false;
        }
      } catch (e) {
        OpenCarrierToast.error('获取二维码失败: ' + (e.message || e));
        this.weixinPolling = false;
      }
    },

    async pollWeixinQrStatus() {
      if (!this.weixinPolling) return;
      try {
        let url = '/api/weixin/qrcode-status?tenant=' + encodeURIComponent(this.botForm.name.trim());
        if (this.weixinQrRaw) url += '&qrcode=' + encodeURIComponent(this.weixinQrRaw);
        const res = await OpenCarrierAPI.get(url);
        this.weixinQrStatus = res.status;
        if (res.status === 'confirmed') {
          this.weixinPolling = false;
          OpenCarrierToast.success('微信授权成功！');
          return;
        }
        if (res.status === 'expired') {
          this.weixinPolling = false;
          OpenCarrierToast.error('二维码已过期，请重新获取');
          return;
        }
      } catch (e) { /* retry */ }
      this.weixinPollTimer = setTimeout(() => this.pollWeixinQrStatus(), 3000);
    },

    stopWeixinPoll() {
      this.weixinPolling = false;
      if (this.weixinPollTimer) {
        clearTimeout(this.weixinPollTimer);
        this.weixinPollTimer = null;
      }
    },

    // ------------------------------------------------------------------
    // QR rendering
    // ------------------------------------------------------------------

    renderQR(elementId, content) {
      const el = document.getElementById(elementId);
      if (!el || !content) return;
      el.innerHTML = '';
      try {
        const qr = qrcode(0, 'M');
        qr.addData(content);
        qr.make();
        el.innerHTML = qr.createImgTag(6, 8);
        const img = el.querySelector('img');
        if (img) {
          img.style.width = '200px';
          img.style.height = '200px';
          img.style.imageRendering = 'pixelated';
        }
      } catch (e) {
        el.innerHTML = '<p style="color:var(--danger);font-size:12px">二维码生成失败</p>';
      }
    },

    renderWeixinQR() {
      const el = document.getElementById('weixin-qr');
      if (!el || !this.weixinQrCode) return;
      if (this.weixinQrCode.startsWith('http')) {
        this.renderQR('weixin-qr', this.weixinQrCode);
      } else {
        el.innerHTML = '';
      }
    },
  };
}

// ------------------------------------------------------------------
// Platform SVG icons
// ------------------------------------------------------------------

const weixinIcon = `<svg width="40" height="40" viewBox="0 0 24 24" fill="none"><circle cx="12" cy="12" r="11" fill="#07c160"/><path d="M8.5 9.5a1.5 1.5 0 1 0 0-3 1.5 1.5 0 0 0 0 3zm7 0a1.5 1.5 0 1 0 0-3 1.5 1.5 0 0 0 0 3zm-9.2 3c.5 2 2.5 3.5 5.2 3.5 1.2 0 2.3-.3 3.2-.8l2.3 1.2-.6-2c1-.8 1.6-2 1.6-3.2 0-2.8-2.7-5-6-5s-6 2.2-6 5c0 .4.1.8.2 1.1z" fill="#fff"/></svg>`;

const wecomIcon = `<svg width="40" height="40" viewBox="0 0 24 24" fill="none"><circle cx="12" cy="12" r="11" fill="#07c160"/><path d="M6 10h3v1H6zM15 10h3v1h-3zM8.5 14a3.5 3.5 0 0 0 7 0" stroke="#fff" stroke-width="1.5" stroke-linecap="round" fill="none"/><path d="M12 4a8 8 0 0 0-8 8c0 2.2 1 4.2 2.5 5.6L5 21l3.5-1.8c1 .5 2.2.8 3.5.8s2.5-.3 3.5-.8L19 21l-1.5-3.4A7.97 7.97 0 0 0 20 12a8 8 0 0 0-8-8z" stroke="#fff" stroke-width="1" fill="none"/></svg>`;

const feishuIcon = `<svg width="40" height="40" viewBox="0 0 24 24" fill="none"><circle cx="12" cy="12" r="11" fill="#3370ff"/><path d="M7 6l5 3 5-3v6l-5 3-5-3V6z" fill="#fff" opacity=".9"/><path d="M7 12l5 3 5-3v3l-5 3-5-3v-3z" fill="#fff" opacity=".6"/></svg>`;

const dingtalkIcon = `<svg width="40" height="40" viewBox="0 0 24 24" fill="none"><circle cx="12" cy="12" r="11" fill="#0089ff"/><path d="M8 7h8v2H8zM8 11h6v2H8zM8 15h4v2H8z" fill="#fff"/></svg>`;

const defaultIcon = `<svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="#888" stroke-width="2"><rect x="3" y="11" width="18" height="11" rx="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/></svg>`;
