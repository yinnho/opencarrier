// OpenCarrier Brain Page — Config, Brain config, Provider keys
'use strict';

function brainPage() {
  return {
    loading: true,
    loadError: '',
    providerKeys: [],
    providerKeyInputs: {},
    providerKeySaving: {},

    async loadBrain() {
      this.loading = true;
      this.loadError = '';
      try {
        await this.loadProviderKeys();
      } catch(e) { this.loadError = e.message || '加载失败'; }
      this.loading = false;
    },

    async loadProviderKeys() {
      try {
        var data = await OpenCarrierAPI.get('/api/providers/keys');
        this.providerKeys = data.providers || [];
        this.providerKeyInputs = {};
      } catch(e) { this.providerKeys = []; }
    },

    async saveProviderKey(name) {
      var p = this.providerKeys.find(function(x){return x.name===name});
      if (p && p.auth_type === 'jwt') { return this.saveProviderKeyJwt(name); }
      var key = (this.providerKeyInputs[name] || '').trim();
      if (!key) { OpenCarrierToast.error('API 密钥不能为空'); return; }
      this.providerKeySaving[name] = true;
      try {
        await OpenCarrierAPI.post('/api/providers/' + name + '/key', { key: key });
        await this.loadProviderKeys();
        OpenCarrierToast.success('已保存 ' + name + ' 的 API 密钥');
      } catch(e) { OpenCarrierToast.error('保存密钥失败: ' + (e.message || e)); }
      this.providerKeySaving[name] = false;
    },

    async saveProviderKeyJwt(name) {
      var p = this.providerKeys.find(function(x){return x.name===name});
      if (!p) return;
      var params = {};
      var hasValue = false;
      (p.params || []).forEach(function(param) {
        var val = (this.providerKeyInputs[name + '_' + param.name] || '').trim();
        if (val) { params[param.name] = val; hasValue = true; }
      }.bind(this));
      if (!hasValue) { OpenCarrierToast.error('请至少填写一项凭证'); return; }
      this.providerKeySaving[name] = true;
      try {
        await OpenCarrierAPI.post('/api/providers/' + name + '/key', { params: params });
        await this.loadProviderKeys();
        OpenCarrierToast.success('已保存 ' + name + ' 的凭证');
      } catch(e) { OpenCarrierToast.error('保存凭证失败: ' + (e.message || e)); }
      this.providerKeySaving[name] = false;
    },

    async deleteProviderKey(name) {
      if (!confirm('确定删除 ' + name + ' 的凭证吗？')) return;
      try {
        await OpenCarrierAPI.del('/api/providers/' + name + '/key');
        await this.loadProviderKeys();
        OpenCarrierToast.success('已删除 ' + name + ' 的凭证');
      } catch(e) { OpenCarrierToast.error('删除凭证失败: ' + (e.message || e)); }
    },

  };
}
