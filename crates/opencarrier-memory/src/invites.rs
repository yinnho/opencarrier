//! Invite tracking store — records share-page referral relationships.

use rusqlite::{params, Connection};
use std::sync::{Arc, Mutex};

/// A recorded invite relationship.
#[derive(Debug, Clone)]
pub struct InviteRecord {
    pub id: i64,
    pub inviter_fp: String,
    pub invitee_tenant_id: Option<String>,
    pub invited_at: String,
    pub converted_at: Option<String>,
    pub source_platform: Option<String>,
}

/// Summary stats for an inviter.
#[derive(Debug, Clone, Default)]
pub struct InviteStats {
    pub total_invites: i64,
    pub converted: i64,
    pub pending: i64,
}

/// Store for invite tracking queries.
pub struct InviteStore {
    conn: Arc<Mutex<Connection>>,
}

impl InviteStore {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    /// Record a new invite (before conversion).
    pub fn record_invite(
        &self,
        inviter_fp: &str,
        source_platform: Option<&str>,
    ) -> Result<i64, rusqlite::Error> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO invites (inviter_fp, invited_at, source_platform)
             VALUES (?1, datetime('now'), ?2)",
            params![inviter_fp, source_platform],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Mark an invite as converted when the invitee completes onboarding.
    pub fn mark_converted(
        &self,
        invite_id: i64,
        invitee_tenant_id: &str,
    ) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE invites
             SET invitee_tenant_id = ?1, converted_at = datetime('now')
             WHERE id = ?2",
            params![invitee_tenant_id, invite_id],
        )?;
        Ok(())
    }

    /// Link an invite by inviter_fp + invitee_tenant_id (for lookup after onboarding).
    pub fn link_invitee(
        &self,
        inviter_fp: &str,
        invitee_tenant_id: &str,
    ) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        // Find the most recent unconverted invite for this inviter
        let id: Option<i64> = conn
            .query_row(
                "SELECT id FROM invites
                 WHERE inviter_fp = ?1 AND invitee_tenant_id IS NULL
                 ORDER BY invited_at DESC LIMIT 1",
                params![inviter_fp],
                |row| row.get(0),
            )
            .ok();
        if let Some(id) = id {
            conn.execute(
                "UPDATE invites
                 SET invitee_tenant_id = ?1, converted_at = datetime('now')
                 WHERE id = ?2",
                params![invitee_tenant_id, id],
            )?;
        }
        Ok(())
    }

    /// Query invite stats for a given inviter fingerprint.
    pub fn query_stats(&self, inviter_fp: &str) -> Result<InviteStats, rusqlite::Error> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let total: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM invites WHERE inviter_fp = ?1",
                params![inviter_fp],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let converted: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM invites WHERE inviter_fp = ?1 AND converted_at IS NOT NULL",
                params![inviter_fp],
                |row| row.get(0),
            )
            .unwrap_or(0);
        Ok(InviteStats {
            total_invites: total,
            converted,
            pending: total - converted,
        })
    }

    /// List recent invites for an inviter.
    pub fn list_invites(
        &self,
        inviter_fp: &str,
        limit: usize,
    ) -> Result<Vec<InviteRecord>, rusqlite::Error> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, inviter_fp, invitee_tenant_id, invited_at, converted_at, source_platform
             FROM invites
             WHERE inviter_fp = ?1
             ORDER BY invited_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![inviter_fp, limit as i64], |row| {
            Ok(InviteRecord {
                id: row.get(0)?,
                inviter_fp: row.get(1)?,
                invitee_tenant_id: row.get(2)?,
                invited_at: row.get(3)?,
                converted_at: row.get(4)?,
                source_platform: row.get(5)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
    }

    /// Global leaderboard: top inviters by conversion count.
    pub fn top_inviters(&self, limit: usize) -> Result<Vec<(String, i64, i64)>, rusqlite::Error> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT inviter_fp,
                    COUNT(*) as total,
                    COUNT(converted_at) as converted
             FROM invites
             GROUP BY inviter_fp
             ORDER BY converted DESC, total DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?))
        })?;
        rows.collect::<Result<Vec<_>, _>>()
    }
}
