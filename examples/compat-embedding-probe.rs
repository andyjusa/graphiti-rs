//! Bounded synthetic embedding verification; no DB or generation requests.
#[cfg(feature = "graphiti-compat")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use graphiti_rs::compat::{
        config::{CompatConfig, GeminiTransport, WriteMode},
        providers::Providers,
    };
    use serde_json::json;
    use sha2::{Digest, Sha256};
    anyhow::ensure!(
        std::env::var("GRAPHITI_EMBEDDING_REHEARSAL").as_deref() == Ok("synthetic-only"),
        "explicit synthetic rehearsal required"
    );
    let config = CompatConfig::from_env()?;
    anyhow::ensure!(
        config.mode == WriteMode::ReadOnly && config.gemini_transport == GeminiTransport::FactChat,
        "rehearsal requires read-only gateway profile"
    );
    anyhow::ensure!(
        config.gemini_base.trim_end_matches('/')
            == "https://factchat-cloud.mindlogic.ai/v1/gateway",
        "unexpected rehearsal endpoint"
    );
    let vectors = Providers::new(config)?
        .embed(&[
            "Gaon works at Saebit.".into(),
            "다온은 누리에서 일한다.".into(),
        ])
        .await?;
    let hashes: Vec<_> = vectors
        .iter()
        .map(|v| {
            format!(
                "{:x}",
                Sha256::digest(v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<_>>())
            )
        })
        .collect();
    println!(
        "{}",
        json!({"model":"gemini-embedding-2","dimensions":3072,"vector_count":vectors.len(),"all_valid":true,"vectors_f32_sha256":hashes,"db_accessed":false})
    );
    Ok(())
}
#[cfg(not(feature = "graphiti-compat"))]
fn main() {
    panic!("Requires graphiti-compat feature");
}
