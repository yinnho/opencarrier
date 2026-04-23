//! Tenant store — CRUD operations for the `tenants` table.

use rusqlite::{params, Connection};
use std::sync::{Arc, Mutex};

use opencarrier_types::tenant::{TenantEntry, TenantRole};

/// Persistent store for tenant accounts.
#[derive(Clone)]
pub struct TenantStore {
    conn: Arc<Mutex<Connection>>,
}

impl TenantStore {
    /// Create a new TenantStore sharing an existing connection.
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    /// Create a tenant from an existing connection reference (for use within substrate).
    pub fn create_tenant_conn(conn: &Connection, entry: &TenantEntry) -> Result<(), String> {
        conn.execute(
            "INSERT INTO tenants (id, name, password_hash, role, enabled, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                entry.id,
                entry.name,
                entry.password_hash,
                entry.role.as_str(),
                entry.enabled as i32,
                entry.created_at,
                entry.updated_at,
            ],
        )
        .map_err(|e| format!("Failed to create tenant: {e}"))?;
        Ok(())
    }

    /// Create a new tenant.
    pub fn create_tenant(&self, entry: &TenantEntry) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {e}"))?;
        Self::create_tenant_conn(&conn, entry)
    }

    /// Get a tenant by ID.
    pub fn get_tenant(&self, id: &str) -> Result<Option<TenantEntry>, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {e}"))?;
        let mut stmt = conn
            .prepare("SELECT id, name, password_hash, role, enabled, created_at, updated_at FROM tenants WHERE id = ?1")
            .map_err(|e| format!("Prepare error: {e}"))?;
        let mut rows = stmt
            .query(params![id])
            .map_err(|e| format!("Query error: {e}"))?;
        if let Some(row) = rows.next().map_err(|e| format!("Fetch error: {e}"))? {
            Ok(Some(row_to_entry(row)?))
        } else {
            Ok(None)
        }
    }

    /// Get a tenant by name (used for login).
    pub fn get_tenant_by_name(&self, name: &str) -> Result<Option<TenantEntry>, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {e}"))?;
        let mut stmt = conn
            .prepare("SELECT id, name, password_hash, role, enabled, created_at, updated_at FROM tenants WHERE name = ?1")
            .map_err(|e| format!("Prepare error: {e}"))?;
        let mut rows = stmt
            .query(params![name])
            .map_err(|e| format!("Query error: {e}"))?;
        if let Some(row) = rows.next().map_err(|e| format!("Fetch error: {e}"))? {
            Ok(Some(row_to_entry(row)?))
        } else {
            Ok(None)
        }
    }

    /// List all tenants.
    pub fn list_tenants(&self) -> Result<Vec<TenantEntry>, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {e}"))?;
        let mut stmt = conn
            .prepare("SELECT id, name, password_hash, role, enabled, created_at, updated_at FROM tenants ORDER BY created_at")
            .map_err(|e| format!("Prepare error: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                row_to_entry(row).map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(std::io::Error::other(e))))
            })
            .map_err(|e| format!("Query error: {e}"))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Update a tenant.
    pub fn update_tenant(&self, entry: &TenantEntry) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {e}"))?;
        conn.execute(
            "UPDATE tenants SET name = ?2, password_hash = ?3, role = ?4, enabled = ?5, updated_at = ?6 WHERE id = ?1",
            params![
                entry.id,
                entry.name,
                entry.password_hash,
                entry.role.as_str(),
                entry.enabled as i32,
                entry.updated_at,
            ],
        )
        .map_err(|e| format!("Failed to update tenant: {e}"))?;
        Ok(())
    }

    /// Delete a tenant by ID.
    pub fn delete_tenant(&self, id: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {e}"))?;
        conn.execute("DELETE FROM tenants WHERE id = ?1", params![id])
            .map_err(|e| format!("Failed to delete tenant: {e}"))?;
        Ok(())
    }

    /// Check if the tenants table is empty (used for admin auto-migration).
    pub fn is_empty(&self) -> Result<bool, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {e}"))?;
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM tenants", [], |row| row.get(0))
            .map_err(|e| format!("Count error: {e}"))?;
        Ok(count == 0)
    }
}

fn row_to_entry(row: &rusqlite::Row) -> Result<TenantEntry, String> {
    Ok(TenantEntry {
        id: row.get(0).map_err(|e| format!("id: {e}"))?,
        name: row.get(1).map_err(|e| format!("name: {e}"))?,
        password_hash: row.get(2).map_err(|e| format!("password_hash: {e}"))?,
        role: TenantRole::from_role_str(&row.get::<_, String>(3).map_err(|e| format!("role: {e}"))?),
        enabled: row.get::<_, i32>(4).map_err(|e| format!("enabled: {e}"))? != 0,
        created_at: row.get(5).map_err(|e| format!("created_at: {e}"))?,
        updated_at: row.get(6).map_err(|e| format!("updated_at: {e}"))?,
    })
}
