//! Re-encode an agent.toml manifest into msgpack blob for SQLite.
//!
//! Usage: cargo run --bin reencode_manifest -- <agent_toml_path> <db_path> <agent_name>

use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eprintln!("Usage: {} <agent.toml> <db_path> <agent_name>", args[0]);
        std::process::exit(1);
    }
    let toml_path = PathBuf::from(&args[1]);
    let db_path = PathBuf::from(&args[2]);
    let agent_name = &args[3];

    // 1. Read and parse agent.toml
    let toml_content = std::fs::read_to_string(&toml_path).unwrap_or_else(|e| {
        eprintln!("Failed to read {}: {e}", toml_path.display());
        std::process::exit(1);
    });

    let toml_value: toml::Value = toml_content.parse().unwrap_or_else(|e| {
        eprintln!("Failed to parse TOML: {e}");
        std::process::exit(1);
    });

    // 2. Deserialize into AgentManifest
    let manifest: opencarrier_types::agent::AgentManifest =
        serde::Deserialize::deserialize(toml_value).unwrap_or_else(|e| {
            eprintln!("Failed to deserialize manifest: {e}");
            std::process::exit(1);
        });

    // 3. Serialize to msgpack
    let blob = rmp_serde::to_vec_named(&manifest).unwrap_or_else(|e| {
        eprintln!("Failed to encode msgpack: {e}");
        std::process::exit(1);
    });

    // 4. Update SQLite
    let conn = rusqlite::Connection::open(&db_path).unwrap_or_else(|e| {
        eprintln!("Failed to open DB: {e}");
        std::process::exit(1);
    });

    conn.execute(
        "UPDATE agents SET manifest = ?1 WHERE name = ?2",
        rusqlite::params![blob, agent_name],
    )
    .unwrap_or_else(|e| {
        eprintln!("Failed to update DB: {e}");
        std::process::exit(1);
    });

    let rows = conn.changes();
    if rows == 0 {
        eprintln!("No agent found with name '{agent_name}'");
        std::process::exit(1);
    }

    // 5. Verify
    let mut stmt = conn
        .prepare("SELECT manifest FROM agents WHERE name = ?1")
        .unwrap();
    let verification: Vec<u8> = stmt
        .query_row(rusqlite::params![agent_name], |row| row.get(0))
        .unwrap();
    let verify_manifest: opencarrier_types::agent::AgentManifest =
        rmp_serde::from_slice(&verification).unwrap();

    let tools = &verify_manifest.capabilities.tools;
    println!("Successfully updated manifest for '{agent_name}'");
    println!("capabilities.tools ({} total):", tools.len());
    for t in tools {
        println!("  - {t}");
    }
}
