// OpenCarrier MCP Page — MCP servers + Tool management
'use strict';

function mcpPage() {
  return {
    activeTab: 'servers',
    mcpLoading: false,
    mcpData: { configured: [], connected: [], total_configured: 0, total_connected: 0 },
    toolsLoading: false,
    toolsData: { tools: [], total: 0 },
    searchQuery: '',

    get filteredTools() {
      if (!this.searchQuery) return this.toolsData.tools;
      var q = this.searchQuery.toLowerCase();
      return this.toolsData.tools.filter(function(t) {
        return t.name.toLowerCase().indexOf(q) >= 0 || (t.description && t.description.toLowerCase().indexOf(q) >= 0);
      });
    },

    get builtinTools() {
      return this.filteredTools.filter(function(t) { return t.source !== 'mcp'; });
    },

    get mcpTools() {
      return this.filteredTools.filter(function(t) { return t.source === 'mcp'; });
    },

    async loadData() {
      await Promise.all([this.loadMcpServers(), this.loadTools()]);
    },

    async loadMcpServers() {
      this.mcpLoading = true;
      try {
        this.mcpData = await OpenCarrierAPI.get('/api/mcp/servers');
      } catch(e) {
        this.mcpData = { configured: [], connected: [], total_configured: 0, total_connected: 0 };
      }
      this.mcpLoading = false;
    },

    async loadTools() {
      this.toolsLoading = true;
      try {
        this.toolsData = await OpenCarrierAPI.get('/api/tools');
      } catch(e) {
        this.toolsData = { tools: [], total: 0 };
      }
      this.toolsLoading = false;
    },

    transportLabel(server) {
      if (!server || !server.transport) return 'unknown';
      return server.transport.type === 'stdio' ? 'Stdio' : 'SSE';
    },

    transportDetail(server) {
      if (!server || !server.transport) return '';
      if (server.transport.type === 'stdio') {
        var cmd = server.transport.command || '';
        var args = (server.transport.args || []).join(' ');
        return cmd + (args ? ' ' + args : '');
      }
      return server.transport.url || '';
    },

    isServerConnected(name) {
      return (this.mcpData.connected || []).some(function(s) { return s.name === name; });
    },

    connectedToolsForServer(name) {
      var server = (this.mcpData.connected || []).find(function(s) { return s.name === name; });
      return server ? (server.tools || []) : [];
    },

    toolCategory(name) {
      if (!name) return 'other';
      var n = name.toLowerCase();
      if (n.indexOf('mcp__') === 0) return 'mcp';
      if (n.indexOf('file_') === 0 || n.indexOf('directory_') === 0) return 'file';
      if (n.indexOf('web_') === 0 || n.indexOf('link_') === 0) return 'web';
      if (n.indexOf('shell') === 0 || n.indexOf('exec_') === 0) return 'shell';
      if (n.indexOf('agent_') === 0) return 'agent';
      if (n.indexOf('memory_') === 0 || n.indexOf('knowledge_') === 0) return 'memory';
      if (n.indexOf('cron_') === 0 || n.indexOf('schedule_') === 0) return 'cron';
      if (n.indexOf('browser_') === 0) return 'browser';
      if (n.indexOf('container_') === 0 || n.indexOf('docker_') === 0) return 'container';
      if (n.indexOf('task_') === 0) return 'task';
      if (n.indexOf('hand_') === 0) return 'hand';
      return 'other';
    },

    categoryLabel(cat) {
      var map = {
        'file': 'File System',
        'web': 'Web',
        'shell': 'Shell',
        'agent': 'Agent',
        'memory': 'Memory',
        'cron': 'Scheduler',
        'browser': 'Browser',
        'container': 'Container',
        'task': 'Task',
        'hand': 'Hand',
        'mcp': 'MCP',
        'other': 'Other'
      };
      return map[cat] || cat;
    }
  };
}
