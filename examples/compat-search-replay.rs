//! Private, provider-free search comparison; never exposed as an MCP tool.
#[cfg(feature = "graphiti-compat")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use graphiti_rs::compat::{
        config::{CompatConfig, WriteMode},
        engine::Engine,
    };
    use serde_json::{Value, json};
    use std::collections::BTreeMap;
    let config = CompatConfig::from_env()?;
    anyhow::ensure!(
        config.redis_url == "redis://127.0.0.1:16380" && config.mode == WriteMode::ReadOnly,
        "replay requires the private loopback restoration in read-only mode"
    );
    let path = std::env::args_os()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("explicit private replay fixture required"))?;
    let data = std::fs::read(path)?;
    anyhow::ensure!(data.len() <= 32 * 1024 * 1024, "fixture exceeds bound");
    let fixture: Value = serde_json::from_slice(&data)?;
    anyhow::ensure!(
        fixture["scope"] == "private provider-free original-memory search replay",
        "invalid replay scope"
    );
    let cases = fixture["cases"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("invalid cases"))?;
    anyhow::ensure!(cases.len() <= 512, "case bound exceeded");
    let engine = Engine::new(config).await?;
    let mut totals = BTreeMap::<String, usize>::new();
    let mut mismatch_paths = BTreeMap::<String, usize>::new();
    let mut diagnostics = vec![];
    for (case_index, case) in cases.iter().enumerate() {
        let name = case["tool"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("invalid tool"))?;
        let vector: Vec<f32> = serde_json::from_value(case["vector"].clone())?;
        let actual = match engine
            .replay_search(name, case["args"].clone(), &vector)
            .await
        {
            Ok(value) => value,
            Err(_) => json!({"error":"replay search refused"}),
        };
        let expected = &case["expected"];
        let refusal = expected.get("error").is_some();
        let matched = if refusal {
            actual.get("error").is_some()
        } else {
            &actual == expected
        };
        *totals.entry(format!("{name}_compared")).or_default() += 1;
        *totals.entry(format!("{name}_matched")).or_default() += usize::from(matched);
        *totals.entry("reference_refusals".into()).or_default() += usize::from(refusal);
        if !matched {
            diagnostics.push(json!({"case_index":case_index,"actual":actual}));
            for path in differences(expected, &actual, "") {
                *mismatch_paths.entry(path).or_default() += 1;
            }
        }
    }
    if let Some(path) = std::env::args_os().nth(2) {
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options
            .open(path)?
            .write_all(&serde_json::to_vec(&diagnostics)?)?;
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"scope":"provider-free original-memory search replay; not query embedding quality", "cases":cases.len(), "totals":totals,"mismatch_paths":mismatch_paths,"provider_calls":0})
        )?
    );
    Ok(())
}
#[cfg(feature = "graphiti-compat")]
fn differences(a: &serde_json::Value, b: &serde_json::Value, path: &str) -> Vec<String> {
    use serde_json::Value;
    if a == b {
        return vec![];
    }
    if path.ends_with("/attributes") {
        return vec![path.into()];
    }
    match (a, b) {
        (Value::Object(a), Value::Object(b)) => {
            let keys: std::collections::BTreeSet<_> = a.keys().chain(b.keys()).collect();
            keys.into_iter()
                .flat_map(|key| {
                    let path = format!("{path}/{key}");
                    match (a.get(key), b.get(key)) {
                        (Some(a), Some(b)) => differences(a, b, &path),
                        _ => vec![path],
                    }
                })
                .collect()
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => a
            .iter()
            .zip(b)
            .enumerate()
            .flat_map(|(i, (a, b))| differences(a, b, &format!("{path}/{i}")))
            .collect(),
        (Value::Array(_), Value::Array(_)) => vec![format!("{path}/length")],
        _ => vec![path.into()],
    }
}
#[cfg(not(feature = "graphiti-compat"))]
fn main() {
    eprintln!("Requires graphiti-compat feature");
    std::process::exit(1);
}
