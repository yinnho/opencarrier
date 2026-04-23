//! Multi-tenant types for OpenCarrier.
//!
//! A tenant represents an enterprise account. The `tenants` table serves as
//! both the tenant and user table — each tenant IS a login account.

use serde::{Deserialize, Serialize};

/// Tenant role — determines access level.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TenantRole {
    /// Full access — can manage all tenants and global config.
    Admin,
    /// Tenant-level access — can only manage own resources.
    #[default]
    Tenant,
}

impl TenantRole {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Admin => "admin",
            Self::Tenant => "tenant",
        }
    }

    pub fn from_role_str(s: &str) -> Self {
        match s {
            "admin" => Self::Admin,
            _ => Self::Tenant,
        }
    }
}

/// A tenant entry — represents both an enterprise and a login account.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantEntry {
    /// Unique ID (UUID string).
    pub id: String,
    /// Display name / login username.
    pub name: String,
    /// Argon2id password hash.
    pub password_hash: String,
    /// Role: "admin" or "tenant".
    pub role: TenantRole,
    /// Whether the tenant account is active.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
    /// ISO 8601 last update timestamp.
    pub updated_at: String,
}

fn default_true() -> bool {
    true
}

/// Request body for creating a new tenant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTenantRequest {
    pub name: String,
    pub password: String,
}

/// Request body for updating a tenant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateTenantRequest {
    pub name: Option<String>,
    pub password: Option<String>,
    pub enabled: Option<bool>,
}

/// Tenant context extracted from auth — injected as axum Extension.
///
/// `tenant_id = None` means global/admin access (backward compatible with no-auth mode).
#[derive(Debug, Clone)]
pub struct TenantContext {
    /// The tenant's ID. None for admin/no-auth (full access).
    pub tenant_id: Option<String>,
    /// The tenant's role.
    pub role: TenantRole,
}

impl TenantContext {
    /// Create an admin-level context (full access, no tenant filtering).
    pub fn admin() -> Self {
        Self {
            tenant_id: None,
            role: TenantRole::Admin,
        }
    }

    /// Check if this context has admin privileges.
    pub fn is_admin(&self) -> bool {
        self.role == TenantRole::Admin
    }

    /// Get the tenant_id, returns error string if not set.
    pub fn require_tenant_id(&self) -> Result<&str, String> {
        self.tenant_id
            .as_deref()
            .ok_or_else(|| "Tenant ID required".to_string())
    }
}
