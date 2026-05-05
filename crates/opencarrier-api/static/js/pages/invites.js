// OpenCarrier Invite Analytics — Referral tracking dashboard
'use strict';

function invitesPage() {
  return {
    myStats: { total_invites: 0, converted: 0, pending: 0 },
    leaderboard: [],
    myInvites: [],
    loading: true,
    loadError: '',
    myFp: '',

    async loadData() {
      this.loading = true;
      this.loadError = '';
      this.myFp = this.getFingerprint();
      try {
        await Promise.all([
          this.loadMyStats(),
          this.loadLeaderboard(),
          this.loadMyInvites()
        ]);
      } catch(e) {
        this.loadError = e.message || '加载邀请数据失败';
      }
      this.loading = false;
    },

    async loadMyStats() {
      try {
        var data = await OpenCarrierAPI.get('/api/invites/stats?inviter_fp=' + encodeURIComponent(this.myFp));
        this.myStats = data;
      } catch(e) {
        this.myStats = { total_invites: 0, converted: 0, pending: 0 };
      }
    },

    async loadLeaderboard() {
      try {
        var data = await OpenCarrierAPI.get('/api/invites/leaderboard?limit=10');
        this.leaderboard = data.leaderboard || [];
      } catch(e) {
        this.leaderboard = [];
      }
    },

    async loadMyInvites() {
      try {
        // Reuse stats endpoint with higher detail if needed; for now leaderboard + stats is enough
        this.myInvites = [];
      } catch(e) {
        this.myInvites = [];
      }
    },

    getFingerprint() {
      try {
        var fp = localStorage.getItem('oc_fingerprint');
        if (!fp) {
          fp = 'fp_' + Math.random().toString(36).substring(2, 10) + Math.random().toString(36).substring(2, 10);
          localStorage.setItem('oc_fingerprint', fp);
        }
        return fp;
      } catch(e) {
        return '';
      }
    },

    formatNumber(n) {
      if (!n) return '0';
      if (n >= 1000000) return (n / 1000000).toFixed(1) + 'M';
      if (n >= 1000) return (n / 1000).toFixed(1) + 'K';
      return String(n);
    },

    get conversionRate() {
      if (!this.myStats.total_invites) return '0%';
      return ((this.myStats.converted / this.myStats.total_invites) * 100).toFixed(1) + '%';
    }
  };
}
