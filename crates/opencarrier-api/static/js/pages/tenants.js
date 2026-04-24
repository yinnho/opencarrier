// OpenCarrier Tenants Page — Admin-only tenant management
'use strict';

function tenantsPage() {
  return {
    tenants: [],
    loading: true,
    loadError: '',
    showCreateModal: false,
    showEditModal: false,
    createForm: { name: '', password: '' },
    editForm: { id: '', name: '', password: '', enabled: true },
    saving: false,

    async loadTenants() {
      this.loading = true;
      this.loadError = '';
      try {
        var data = await OpenCarrierAPI.get('/api/tenants');
        this.tenants = Array.isArray(data) ? data : [];
      } catch(e) {
        this.loadError = e.message || 'Failed to load tenants';
      }
      this.loading = false;
    },

    async createTenant() {
      var f = this.createForm;
      if (!f.name.trim() || !f.password.trim()) {
        OpenCarrierToast.error('Name and password are required');
        return;
      }
      this.saving = true;
      try {
        await OpenCarrierAPI.post('/api/tenants', {
          name: f.name.trim(),
          password: f.password.trim()
        });
        OpenCarrierToast.success('Tenant created');
        this.showCreateModal = false;
        this.createForm = { name: '', password: '' };
        await this.loadTenants();
      } catch(e) {
        OpenCarrierToast.error('Failed to create tenant: ' + (e.message || e));
      }
      this.saving = false;
    },

    openEdit(tenant) {
      this.editForm = {
        id: tenant.id,
        name: tenant.name,
        password: '',
        enabled: tenant.enabled !== false
      };
      this.showEditModal = true;
    },

    async updateTenant() {
      var f = this.editForm;
      if (!f.name.trim()) {
        OpenCarrierToast.error('Name is required');
        return;
      }
      this.saving = true;
      try {
        var body = {
          name: f.name.trim(),
          enabled: f.enabled
        };
        if (f.password.trim()) {
          body.password = f.password.trim();
        }
        await OpenCarrierAPI.put('/api/tenants/' + f.id, body);
        OpenCarrierToast.success('Tenant updated');
        this.showEditModal = false;
        this.editForm = { id: '', name: '', password: '', enabled: true };
        await this.loadTenants();
      } catch(e) {
        OpenCarrierToast.error('Failed to update tenant: ' + (e.message || e));
      }
      this.saving = false;
    },

    async deleteTenant(id, name) {
      if (!confirm('Delete tenant "' + (name || id) + '"? This cannot be undone.')) return;
      try {
        await OpenCarrierAPI.del('/api/tenants/' + id);
        OpenCarrierToast.success('Tenant deleted');
        await this.loadTenants();
      } catch(e) {
        OpenCarrierToast.error('Failed to delete tenant: ' + (e.message || e));
      }
    },

    formatDate(ts) {
      if (!ts) return '-';
      try { return new Date(ts).toLocaleDateString(); } catch(e) { return ts; }
    }
  };
}
