// OpenCarrier Analytics Page — Full usage analytics with per-model and per-agent breakdowns
'use strict';

function analyticsPage() {
  return {
    tab: 'summary',
    summary: {},
    byModel: [],
    byAgent: [],
    loading: true,
    loadError: '',

    // Chart state
    dailyCosts: [],
    firstEventDate: null,

    // Chart colors for providers (stable palette)
    _chartColors: [
      '#FF5C00', '#3B82F6', '#10B981', '#F59E0B', '#8B5CF6',
      '#EC4899', '#06B6D4', '#EF4444', '#84CC16', '#F97316',
      '#6366F1', '#14B8A6', '#E11D48', '#A855F7', '#22D3EE'
    ],

    async loadUsage() {
      this.loading = true;
      this.loadError = '';
      try {
        await Promise.all([
          this.loadSummary(),
          this.loadByModel(),
          this.loadByAgent(),
          this.loadDailyCosts()
        ]);
      } catch(e) {
        this.loadError = e.message || 'Could not load usage data.';
      }
      this.loading = false;
    },

    async loadData() { return this.loadUsage(); },

    async loadSummary() {
      try {
        this.summary = await OpenCarrierAPI.get('/api/usage/summary');
      } catch(e) {
        this.summary = { total_input_tokens: 0, total_output_tokens: 0, call_count: 0, total_tool_calls: 0 };
        throw e;
      }
    },

    async loadByModel() {
      try {
        var data = await OpenCarrierAPI.get('/api/usage/by-model');
        this.byModel = data.models || [];
      } catch(e) { this.byModel = []; }
    },

    async loadByAgent() {
      try {
        var data = await OpenCarrierAPI.get('/api/usage');
        this.byAgent = data.agents || [];
      } catch(e) { this.byAgent = []; }
    },

    async loadDailyCosts() {
      try {
        var data = await OpenCarrierAPI.get('/api/usage/daily');
        this.dailyCosts = data.days || [];
        this.firstEventDate = data.first_event_date || null;
      } catch(e) {
        this.dailyCosts = [];
        this.firstEventDate = null;
      }
    },

    formatTokens(n) {
      if (!n) return '0';
      if (n >= 1000000) return (n / 1000000).toFixed(2) + 'M';
      if (n >= 1000) return (n / 1000).toFixed(1) + 'K';
      return String(n);
    },

    maxTokens() {
      var max = 0;
      this.byModel.forEach(function(m) {
        var t = (m.total_input_tokens || 0) + (m.total_output_tokens || 0);
        if (t > max) max = t;
      });
      return max || 1;
    },

    barWidth(m) {
      var t = (m.total_input_tokens || 0) + (m.total_output_tokens || 0);
      return Math.max(2, Math.round((t / this.maxTokens()) * 100)) + '%';
    },

    // ── Provider aggregation from byModel data ──

    tokensByProvider() {
      var providerMap = {};
      var self = this;
      this.byModel.forEach(function(m) {
        var provider = self._extractProvider(m.model);
        if (!providerMap[provider]) {
          providerMap[provider] = { provider: provider, tokens: 0, calls: 0 };
        }
        providerMap[provider].tokens += (m.total_input_tokens || 0) + (m.total_output_tokens || 0);
        providerMap[provider].calls += (m.call_count || 0);
      });
      var result = [];
      for (var key in providerMap) {
        if (providerMap.hasOwnProperty(key)) {
          result.push(providerMap[key]);
        }
      }
      result.sort(function(a, b) { return b.tokens - a.tokens; });
      return result;
    },

    _extractProvider(modelName) {
      if (!modelName) return 'Unknown';
      var lower = modelName.toLowerCase();
      if (lower.indexOf('claude') !== -1 || lower.indexOf('haiku') !== -1 || lower.indexOf('sonnet') !== -1 || lower.indexOf('opus') !== -1) return 'Anthropic';
      if (lower.indexOf('gemini') !== -1 || lower.indexOf('gemma') !== -1) return 'Google';
      if (lower.indexOf('gpt') !== -1 || lower.indexOf('o1') !== -1 || lower.indexOf('o3') !== -1 || lower.indexOf('o4') !== -1) return 'OpenAI';
      if (lower.indexOf('llama') !== -1 || lower.indexOf('mixtral') !== -1 || lower.indexOf('groq') !== -1) return 'Groq';
      if (lower.indexOf('deepseek') !== -1) return 'DeepSeek';
      if (lower.indexOf('mistral') !== -1) return 'Mistral';
      if (lower.indexOf('command') !== -1 || lower.indexOf('cohere') !== -1) return 'Cohere';
      if (lower.indexOf('grok') !== -1) return 'xAI';
      if (lower.indexOf('jamba') !== -1) return 'AI21';
      if (lower.indexOf('qwen') !== -1) return 'Together';
      return 'Other';
    },

    // ── Donut chart (stroke-dasharray on circles) — token-based ──

    donutSegments() {
      var providers = this.tokensByProvider();
      var total = 0;
      var colors = this._chartColors;
      providers.forEach(function(p) { total += p.tokens; });
      if (total === 0) return [];

      var segments = [];
      var offset = 0;
      var circumference = 2 * Math.PI * 60; // r=60
      for (var i = 0; i < providers.length; i++) {
        var pct = providers[i].tokens / total;
        var dashLen = pct * circumference;
        segments.push({
          provider: providers[i].provider,
          tokens: providers[i].tokens,
          percent: Math.round(pct * 100),
          color: colors[i % colors.length],
          dasharray: dashLen + ' ' + (circumference - dashLen),
          dashoffset: -offset,
          circumference: circumference
        });
        offset += dashLen;
      }
      return segments;
    },

    // ── Bar chart (last 7 days) ──

    barChartData() {
      var days = this.dailyCosts;
      if (!days || days.length === 0) return [];
      var maxTokens = 0;
      days.forEach(function(d) { if (d.tokens > maxTokens) maxTokens = d.tokens; });
      if (maxTokens === 0) maxTokens = 1;

      var dayNames = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
      var result = [];
      for (var i = 0; i < days.length; i++) {
        var d = new Date(days[i].date + 'T12:00:00');
        var dayName = dayNames[d.getDay()] || '?';
        var heightPct = Math.max(2, Math.round((days[i].tokens / maxTokens) * 120));
        result.push({
          date: days[i].date,
          dayName: dayName,
          tokens: days[i].tokens,
          calls: days[i].calls,
          barHeight: heightPct
        });
      }
      return result;
    },

    // ── Token by model table (sorted by tokens descending) ──

    tokensByModelSorted() {
      var models = this.byModel.slice();
      models.sort(function(a, b) { return ((b.total_input_tokens || 0) + (b.total_output_tokens || 0)) - ((a.total_input_tokens || 0) + (a.total_output_tokens || 0)); });
      return models;
    },

    maxModelTokens() {
      var max = 0;
      this.byModel.forEach(function(m) {
        var t = (m.total_input_tokens || 0) + (m.total_output_tokens || 0);
        if (t > max) max = t;
      });
      return max || 1;
    },

    tokenBarWidth(m) {
      var t = (m.total_input_tokens || 0) + (m.total_output_tokens || 0);
      return Math.max(2, Math.round((t / this.maxModelTokens()) * 100)) + '%';
    },

    modelTier(modelName) {
      if (!modelName) return 'unknown';
      var lower = modelName.toLowerCase();
      if (lower.indexOf('opus') !== -1 || lower.indexOf('o1') !== -1 || lower.indexOf('o3') !== -1 || lower.indexOf('deepseek-r1') !== -1) return 'frontier';
      if (lower.indexOf('sonnet') !== -1 || lower.indexOf('gpt-4') !== -1 || lower.indexOf('gemini-2.5') !== -1 || lower.indexOf('gemini-1.5-pro') !== -1) return 'smart';
      if (lower.indexOf('haiku') !== -1 || lower.indexOf('gpt-3.5') !== -1 || lower.indexOf('flash') !== -1 || lower.indexOf('mixtral') !== -1) return 'balanced';
      if (lower.indexOf('llama') !== -1 || lower.indexOf('groq') !== -1 || lower.indexOf('gemma') !== -1) return 'fast';
      return 'balanced';
    }
  };
}
