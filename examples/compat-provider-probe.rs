//! Explicit, bounded real-provider rehearsal; synthetic input only, no DB access.
#[cfg(feature = "graphiti-compat")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use graphiti_rs::compat::{
        config::{CompatConfig, WriteMode},
        providers::Providers,
    };
    use serde_json::json;
    let config = CompatConfig::from_env()?;
    anyhow::ensure!(
        config.mode == WriteMode::ReadOnly,
        "provider rehearsal requires read-only configuration"
    );
    anyhow::ensure!(
        (1..=4096).contains(&config.max_tokens),
        "explicit provider token bound required"
    );
    anyhow::ensure!(
        config.llm_key_file.is_some() && config.gemini_key_file.is_some(),
        "explicit private credential files required"
    );
    let mut types = vec![
        json!({"entity_type_id":0,"entity_type_name":"Entity","entity_type_description":"Default entity classification. Use this entity type if the entity is not one of the other listed types."}),
    ];
    types.extend(config.entity_types.iter().enumerate().map(|(i,t)|json!({"entity_type_id":i+1,"entity_type_name":t.name,"entity_type_description":t.description})));
    let provider = Providers::new(config)?;
    let texts = vec![
        "Gaon works at Saebit.".to_owned(),
        "다온은 누리에서 일한다.".to_owned(),
        "Gaon is employed by Saebit.".to_owned(),
        "The blue kettle is on the table.".to_owned(),
    ];
    let mut contexts = vec![];
    let mut generated = vec![];
    for text in &texts[..2] {
        let context = json!({"episode_content":text,"episode_timestamp":"2026-10-02T00:00:00+00:00","previous_episodes":[],"source_description":"synthetic provider rehearsal","custom_extraction_instructions":"","entity_types":types});
        generated.push(
            provider
                .chat("extract_nodes.extract_text", &context)
                .await?,
        );
        contexts.push(context);
    }
    println!(
        "{}",
        serde_json::to_string(
            &json!({"stage":"llm_complete","scope":"synthetic-only; no DB access","entities":generated})
        )?
    );
    let vectors = provider.embed(&texts).await?;
    println!(
        "{}",
        serde_json::to_string(
            &json!({"scope":"bounded synthetic-only real-provider rehearsal; no DB access","llm_calls":2,"embedding_batches":1,"texts":texts,"contexts":contexts,"entities":generated,"vectors":vectors})
        )?
    );
    Ok(())
}
#[cfg(not(feature = "graphiti-compat"))]
fn main() {
    eprintln!("Requires graphiti-compat feature");
    std::process::exit(1)
}
