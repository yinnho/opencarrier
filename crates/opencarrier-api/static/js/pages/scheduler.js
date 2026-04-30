// OpenCarrier Scheduler Page — Cron job management
'use strict';

function schedulerPage() {
  return {
    tab: 'jobs',
    jobs: [],
    loading: true,
    loadError: '',

    // -- Create Job form --
    showCreateForm: false,
    newJob: {
      name: '',
      cron: '',
      agent_id: '',
      message: '',
      enabled: true
    },
    creating: false,

    // -- Run Now state --
    runningJobId: '',

    // Cron presets
    cronPresets: [
      { label: '每分钟', cron: '* * * * *' },
      { label: '每 5 分钟', cron: '*/5 * * * *' },
      { label: '每 15 分钟', cron: '*/15 * * * *' },
      { label: '每 30 分钟', cron: '*/30 * * * *' },
      { label: '每小时', cron: '0 * * * *' },
      { label: '每 6 小时', cron: '0 */6 * * *' },
      { label: '每天午夜', cron: '0 0 * * *' },
      { label: '每天上午 9 点', cron: '0 9 * * *' },
      { label: '工作日上午 9 点', cron: '0 9 * * 1-5' },
      { label: '每周一上午 9 点', cron: '0 9 * * 1' },
      { label: '每月 1 号', cron: '0 0 1 * *' }
    ],

    // ── Lifecycle ──

    async loadData() {
      this.loading = true;
      this.loadError = '';
      try {
        await this.loadJobs();
      } catch(e) {
        this.loadError = e.message || '无法加载定时任务数据。';
      }
      this.loading = false;
    },

    async loadJobs() {
      var data = await OpenCarrierAPI.get('/api/cron/jobs');
      var raw = data.jobs || [];
      this.jobs = raw.map(function(j) {
        var cron = '';
        if (j.schedule) {
          if (j.schedule.kind === 'cron') cron = j.schedule.expr || '';
          else if (j.schedule.kind === 'every') cron = 'every ' + j.schedule.every_secs + 's';
          else if (j.schedule.kind === 'at') cron = 'at ' + (j.schedule.at || '');
        }
        return {
          id: j.id,
          name: j.name,
          cron: cron,
          agent_id: j.agent_id,
          message: j.action ? j.action.message || '' : '',
          enabled: j.enabled,
          last_run: j.last_run,
          next_run: j.next_run,
          delivery: j.delivery ? j.delivery.kind || '' : '',
          created_at: j.created_at
        };
      });
    },

    // ── Job CRUD ──

    async createJob() {
      if (!this.newJob.name.trim()) {
        OpenCarrierToast.warn('请输入任务名称');
        return;
      }
      if (!this.newJob.cron.trim()) {
        OpenCarrierToast.warn('请输入 Cron 表达式');
        return;
      }
      this.creating = true;
      try {
        var jobName = this.newJob.name;
        var body = {
          agent_id: this.newJob.agent_id,
          name: this.newJob.name,
          schedule: { kind: 'cron', expr: this.newJob.cron },
          action: { kind: 'agent_turn', message: this.newJob.message || '定时任务: ' + this.newJob.name },
          delivery: { kind: 'last_channel' },
          enabled: this.newJob.enabled
        };
        await OpenCarrierAPI.post('/api/cron/jobs', body);
        this.showCreateForm = false;
        this.newJob = { name: '', cron: '', agent_id: '', message: '', enabled: true };
        OpenCarrierToast.success('计划 "' + jobName + '" 已创建');
        await this.loadJobs();
      } catch(e) {
        OpenCarrierToast.error('创建计划失败: ' + (e.message || e));
      }
      this.creating = false;
    },

    async toggleJob(job) {
      try {
        var newState = !job.enabled;
        await OpenCarrierAPI.put('/api/cron/jobs/' + job.id + '/enable', { enabled: newState });
        job.enabled = newState;
        OpenCarrierToast.success('计划' + (newState ? '已启用' : '已暂停'));
      } catch(e) {
        OpenCarrierToast.error('切换计划状态失败: ' + (e.message || e));
      }
    },

    deleteJob(job) {
      var self = this;
      var jobName = job.name || job.id;
      OpenCarrierToast.confirm('删除计划', '确定要删除 "' + jobName + '" 吗？此操作不可撤销。', async function() {
        try {
          await OpenCarrierAPI.del('/api/cron/jobs/' + job.id);
          self.jobs = self.jobs.filter(function(j) { return j.id !== job.id; });
          OpenCarrierToast.success('计划 "' + jobName + '" 已删除');
        } catch(e) {
          OpenCarrierToast.error('删除计划失败: ' + (e.message || e));
        }
      });
    },

    async runNow(job) {
      this.runningJobId = job.id;
      try {
        var result = await OpenCarrierAPI.post('/api/schedules/' + job.id + '/run', {});
        if (result.status === 'completed') {
          OpenCarrierToast.success('计划 "' + (job.name || 'job') + '" 执行成功');
          job.last_run = new Date().toISOString();
        } else {
          OpenCarrierToast.error('计划运行失败: ' + (result.error || '未知错误'));
        }
      } catch(e) {
        OpenCarrierToast.error('立即运行功能暂不支持 Cron 任务');
      }
      this.runningJobId = '';
    },

    // ── Utility ──

    get availableAgents() {
      return Alpine.store('app').agents || [];
    },

    agentName(agentId) {
      if (!agentId) return '(任意)';
      var agents = this.availableAgents;
      for (var i = 0; i < agents.length; i++) {
        if (agents[i].id === agentId) return agents[i].name;
      }
      if (agentId.length > 12) return agentId.substring(0, 8) + '...';
      return agentId;
    },

    describeCron(expr) {
      if (!expr) return '';
      if (expr.indexOf('every ') === 0) return expr;
      if (expr.indexOf('at ') === 0) return '一次性: ' + expr.substring(3);

      var map = {
        '* * * * *': '每分钟',
        '*/2 * * * *': '每 2 分钟',
        '*/5 * * * *': '每 5 分钟',
        '*/10 * * * *': '每 10 分钟',
        '*/15 * * * *': '每 15 分钟',
        '*/30 * * * *': '每 30 分钟',
        '0 * * * *': '每小时',
        '0 */2 * * *': '每 2 小时',
        '0 */4 * * *': '每 4 小时',
        '0 */6 * * *': '每 6 小时',
        '0 */12 * * *': '每 12 小时',
        '0 0 * * *': '每天午夜',
        '0 6 * * *': '每天上午 6:00',
        '0 9 * * *': '每天上午 9:00',
        '0 12 * * *': '每天中午',
        '0 18 * * *': '每天下午 6:00',
        '0 9 * * 1-5': '工作日上午 9:00',
        '0 9 * * 1': '每周一上午 9:00',
        '0 0 * * 0': '每周日午夜',
        '0 0 1 * *': '每月 1 号',
        '0 0 * * 1': '每周一午夜'
      };
      if (map[expr]) return map[expr];

      var parts = expr.split(' ');
      if (parts.length !== 5) return expr;

      var min = parts[0];
      var hour = parts[1];
      var dom = parts[2];
      var mon = parts[3];
      var dow = parts[4];

      if (min.indexOf('*/') === 0 && hour === '*' && dom === '*' && mon === '*' && dow === '*') {
        return '每 ' + min.substring(2) + ' 分钟';
      }
      if (min === '0' && hour.indexOf('*/') === 0 && dom === '*' && mon === '*' && dow === '*') {
        return '每 ' + hour.substring(2) + ' 小时';
      }

      var dowNames = { '0': '周日', '1': '周一', '2': '周二', '3': '周三', '4': '周四', '5': '周五', '6': '周六', '7': '周日',
                       '1-5': '工作日', '0,6': '周末', '6,0': '周末' };

      if (dom === '*' && mon === '*' && min.match(/^\d+$/) && hour.match(/^\d+$/)) {
        var h = parseInt(hour, 10);
        var m = parseInt(min, 10);
        var ampm = h >= 12 ? 'PM' : 'AM';
        var h12 = h === 0 ? 12 : (h > 12 ? h - 12 : h);
        var mStr = m < 10 ? '0' + m : '' + m;
        var timeStr = h12 + ':' + mStr + ' ' + ampm;
        if (dow === '*') return '每天 ' + timeStr;
        var dowLabel = dowNames[dow] || ('星期 ' + dow);
        return dowLabel + ' ' + timeStr;
      }

      return expr;
    },

    applyCronPreset(preset) {
      this.newJob.cron = preset.cron;
    },

    formatTime(ts) {
      if (!ts) return '-';
      try {
        var d = new Date(ts);
        if (isNaN(d.getTime())) return '-';
        return d.toLocaleString();
      } catch(e) { return '-'; }
    },

    relativeTime(ts) {
      if (!ts) return '从未';
      try {
        var diff = Date.now() - new Date(ts).getTime();
        if (isNaN(diff)) return '从未';
        if (diff < 0) {
          var absDiff = Math.abs(diff);
          if (absDiff < 60000) return '<1 分钟后';
          if (absDiff < 3600000) return Math.floor(absDiff / 60000) + ' 分钟后';
          if (absDiff < 86400000) return Math.floor(absDiff / 3600000) + ' 小时后';
          return Math.floor(absDiff / 86400000) + ' 天后';
        }
        if (diff < 60000) return '刚刚';
        if (diff < 3600000) return Math.floor(diff / 60000) + ' 分钟前';
        if (diff < 86400000) return Math.floor(diff / 3600000) + ' 小时前';
        return Math.floor(diff / 86400000) + ' 天前';
      } catch(e) { return '从未'; }
    },

    jobCount() {
      var enabled = 0;
      for (var i = 0; i < this.jobs.length; i++) {
        if (this.jobs[i].enabled) enabled++;
      }
      return enabled;
    }
  };
}
