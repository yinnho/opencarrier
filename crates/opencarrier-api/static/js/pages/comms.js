// OpenCarrier Comms Page — Agent topology & inter-agent communication feed
'use strict';

function commsPage() {
  return {
    topology: { nodes: [], edges: [] },
    events: [],
    loading: true,
    loadError: '',
    sseSource: null,
    showSendModal: false,
    showTaskModal: false,
    sendFrom: '',
    sendTo: '',
    sendMsg: '',
    sendLoading: false,
    taskTitle: '',
    taskDesc: '',
    taskAssign: '',
    taskLoading: false,

    async loadData() {
      this.loading = true;
      this.loadError = '';
      try {
        var results = await Promise.all([
          OpenCarrierAPI.get('/api/comms/topology'),
          OpenCarrierAPI.get('/api/comms/events?limit=200')
        ]);
        this.topology = results[0] || { nodes: [], edges: [] };
        this.events = results[1] || [];
        this.startSSE();
      } catch(e) {
        this.loadError = e.message || '无法加载通讯数据。';
      }
      this.loading = false;
    },

    startSSE() {
      if (this.sseSource) this.sseSource.close();
      var self = this;
      var url = OpenCarrierAPI.baseUrl + '/api/comms/events/stream';
      if (OpenCarrierAPI.apiKey) url += '?token=' + encodeURIComponent(OpenCarrierAPI.apiKey);
      this.sseSource = new EventSource(url);
      this.sseSource.onmessage = function(ev) {
        if (ev.data === 'ping') return;
        try {
          var event = JSON.parse(ev.data);
          self.events.unshift(event);
          if (self.events.length > 200) self.events.length = 200;
          // Refresh topology on spawn/terminate events
          if (event.kind === 'agent_spawned' || event.kind === 'agent_terminated') {
            self.refreshTopology();
          }
        } catch(e) { /* ignore parse errors */ }
      };
    },

    stopSSE() {
      if (this.sseSource) {
        this.sseSource.close();
        this.sseSource = null;
      }
    },

    async refreshTopology() {
      try {
        this.topology = await OpenCarrierAPI.get('/api/comms/topology');
      } catch(e) { /* silent */ }
    },

    rootNodes() {
      var childIds = {};
      var self = this;
      this.topology.edges.forEach(function(e) {
        if (e.kind === 'parent_child') childIds[e.to] = true;
      });
      return this.topology.nodes.filter(function(n) { return !childIds[n.id]; });
    },

    childrenOf(id) {
      var childIds = {};
      this.topology.edges.forEach(function(e) {
        if (e.kind === 'parent_child' && e.from === id) childIds[e.to] = true;
      });
      return this.topology.nodes.filter(function(n) { return childIds[n.id]; });
    },

    peersOf(id) {
      var peerIds = {};
      this.topology.edges.forEach(function(e) {
        if (e.kind === 'peer') {
          if (e.from === id) peerIds[e.to] = true;
          if (e.to === id) peerIds[e.from] = true;
        }
      });
      return this.topology.nodes.filter(function(n) { return peerIds[n.id]; });
    },

    stateBadgeClass(state) {
      switch(state) {
        case 'Running': return 'badge badge-success';
        case 'Suspended': return 'badge badge-warning';
        case 'Terminated': case 'Crashed': return 'badge badge-danger';
        default: return 'badge badge-dim';
      }
    },

    eventBadgeClass(kind) {
      switch(kind) {
        case 'agent_message': return 'badge badge-info';
        case 'agent_spawned': return 'badge badge-success';
        case 'agent_terminated': return 'badge badge-danger';
        case 'task_posted': return 'badge badge-warning';
        case 'task_claimed': return 'badge badge-info';
        case 'task_completed': return 'badge badge-success';
        default: return 'badge badge-dim';
      }
    },

    eventIcon(kind) {
      switch(kind) {
        case 'agent_message': return '\u2709';
        case 'agent_spawned': return '+';
        case 'agent_terminated': return '\u2715';
        case 'task_posted': return '\u2691';
        case 'task_claimed': return '\u2690';
        case 'task_completed': return '\u2713';
        default: return '\u2022';
      }
    },

    eventLabel(kind) {
      switch(kind) {
        case 'agent_message': return '消息';
        case 'agent_spawned': return '已创建';
        case 'agent_terminated': return '已终止';
        case 'task_posted': return '任务已发布';
        case 'task_claimed': return '任务已认领';
        case 'task_completed': return '任务已完成';
        default: return kind;
      }
    },

    timeAgo(dateStr) {
      if (!dateStr) return '';
      var d = new Date(dateStr);
      var secs = Math.floor((Date.now() - d.getTime()) / 1000);
      if (secs < 60) return secs + '秒前';
      if (secs < 3600) return Math.floor(secs / 60) + '分钟前';
      if (secs < 86400) return Math.floor(secs / 3600) + '小时前';
      return Math.floor(secs / 86400) + '天前';
    },

    openSendModal() {
      this.sendFrom = '';
      this.sendTo = '';
      this.sendMsg = '';
      this.showSendModal = true;
    },

    async submitSend() {
      if (!this.sendFrom || !this.sendTo || !this.sendMsg.trim()) return;
      this.sendLoading = true;
      try {
        await OpenCarrierAPI.post('/api/comms/send', {
          from_agent_id: this.sendFrom,
          to_agent_id: this.sendTo,
          message: this.sendMsg
        });
        OpenCarrierToast.success('消息已发送');
        this.showSendModal = false;
      } catch(e) {
        OpenCarrierToast.error(e.message || '发送失败');
      }
      this.sendLoading = false;
    },

    openTaskModal() {
      this.taskTitle = '';
      this.taskDesc = '';
      this.taskAssign = '';
      this.showTaskModal = true;
    },

    async submitTask() {
      if (!this.taskTitle.trim()) return;
      this.taskLoading = true;
      try {
        var body = { title: this.taskTitle, description: this.taskDesc };
        if (this.taskAssign) body.assigned_to = this.taskAssign;
        await OpenCarrierAPI.post('/api/comms/task', body);
        OpenCarrierToast.success('任务已发布');
        this.showTaskModal = false;
      } catch(e) {
        OpenCarrierToast.error(e.message || '任务创建失败');
      }
      this.taskLoading = false;
    }
  };
}
