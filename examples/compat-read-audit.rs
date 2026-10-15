//! Maintenance example, never exposed as an MCP tool. Explicit private manifest.
#[cfg(feature = "graphiti-compat")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use graphiti_rs::compat::{audit, config::WriteMode, policy, store::LegacyStore};
    use serde_json::{Value, json};
    let url = std::env::var("COMPAT_AUDIT_DB_URL")?;
    anyhow::ensure!(
        url == "redis://127.0.0.1:16380",
        "private restore audit requires loopback port 16380"
    );
    let path = std::env::args_os()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("explicit private graph manifest required"))?;
    let manifest: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let store = LegacyStore::connect(&url, WriteMode::ReadOnly).await?;
    let mut reports = vec![];
    for entry in manifest["groups"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("invalid graph manifest"))?
    {
        let group = entry["group"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("invalid group"))?;
        if policy::read(group).is_err() {
            reports.push(json!({"group":group,"status":"outside_recorded_mcp_policy"}));
            continue;
        }
        match audit::inspect(&store, group).await {
            Ok(audit) => {
                let status = if audit.mis_scoped_entities
                    + audit.mis_scoped_episodes
                    + audit.mis_scoped_facts
                    > 0
                {
                    "readable_with_scope_exclusions"
                } else {
                    "readable"
                };
                reports.push(json!({"group":group,"status":status,"audit":audit}));
            }
            Err(error) => {
                reports.push(json!({"group":group,"status":"failed","error":format!("{error:#}")}))
            }
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"scope":"private original-memory read-only audit","groups":reports})
        )?
    );
    anyhow::ensure!(
        reports.iter().all(|r| r["status"] != "failed"),
        "some restored groups failed audit; see private report"
    );
    Ok(())
}
#[cfg(not(feature = "graphiti-compat"))]
fn main() {
    eprintln!("Requires graphiti-compat feature");
    std::process::exit(1);
}
