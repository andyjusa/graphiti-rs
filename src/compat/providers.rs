use super::{
    config::{CompatConfig, read_secret},
    dto::*,
    journal::Job,
};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::{Map, Value, json};

#[derive(Clone)]
pub struct Providers {
    config: CompatConfig,
    client: reqwest::Client,
}
#[derive(Deserialize)]
struct Extraction {
    entities: Vec<Entity>,
    facts: Vec<Fact>,
}
#[derive(Deserialize)]
struct Entity {
    name: String,
    summary: String,
    #[serde(default)]
    labels: Vec<String>,
    #[serde(default)]
    attributes: Map<String, Value>,
}
#[derive(Deserialize)]
struct Fact {
    source: usize,
    target: usize,
    name: String,
    fact: String,
    #[serde(default)]
    valid_at: Option<String>,
    #[serde(default)]
    attributes: Map<String, Value>,
}
impl Providers {
    pub fn new(config: CompatConfig) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(config.timeout)
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            config,
        })
    }
    pub async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        ensure!(texts.len() <= 256, "embedding batch exceeds bound");
        let mut output = vec![];
        for batch in texts.chunks(8) {
            let mut request=self.client.post(format!("{}/models/gemini-embedding-2:batchEmbedContents",self.config.gemini_base.trim_end_matches('/'))).json(&json!({"requests":batch.iter().map(|text|json!({"model":"models/gemini-embedding-2","content":{"parts":[{"text":text.replace('\n'," ")}]},"outputDimensionality":3072})).collect::<Vec<_>>()}));
            if let Some(file) = &self.config.gemini_key_file {
                request = request.header("x-goog-api-key", read_secret(file)?);
            }
            let value = response(request).await?;
            let embeddings = value
                .get("embeddings")
                .and_then(Value::as_array)
                .context("invalid embedding response")?;
            ensure!(embeddings.len() == batch.len(), "embedding count mismatch");
            for embedding in embeddings {
                let vector: Vec<f32> = serde_json::from_value(
                    embedding
                        .get("values")
                        .cloned()
                        .context("missing embedding values")?,
                )
                .map_err(|_| anyhow::anyhow!("invalid embedding values"))?;
                validate_vector(&vector)?;
                output.push(vector);
            }
        }
        Ok(output)
    }
    pub async fn prepare(&self, job: &Job) -> Result<Prepared> {
        let system = "Extract the supplied episode as JSON with entities:[{name,summary,labels,attributes}] and facts:[{source,target,name,fact,valid_at,attributes}]. source and target are zero-based entity indices. Use only explicit information. Do not invent facts. Max 128 entities and 128 facts. valid_at is an RFC3339 string or null. attributes contain only scalar values or scalar lists.";
        let mut request=self.client.post(format!("{}/chat/completions",self.config.llm_base.trim_end_matches('/'))).json(&json!({"model":self.config.llm_model,"max_tokens":self.config.max_tokens,"temperature":1,"extra_body":{"chat_template_kwargs":{"enable_thinking":false}},"response_format":{"type":"json_object"},"messages":[{"role":"system","content":system},{"role":"user","content":format!("Reference time: {}\nName: {}\nSource: {}\nEpisode: {}",job.created,job.request.name,job.request.source,job.request.episode_body)}]}));
        if let Some(file) = &self.config.llm_key_file {
            request = request.bearer_auth(read_secret(file)?);
        }
        let value = response(request).await?;
        let content = value
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .context("missing extraction response")?;
        let x: Extraction = serde_json::from_value(decode_extraction(content)?)
            .map_err(|_| anyhow::anyhow!("invalid extraction JSON"))?;
        ensure!(
            x.entities.len() <= 128 && x.facts.len() <= 128,
            "extraction exceeds limits"
        );
        for n in &x.entities {
            crate::domain::text(&n.name, 512, "entity name")?;
            ensure!(n.summary.len() <= 8192, "entity summary too long");
            attrs(&n.attributes)?;
        }
        for f in &x.facts {
            ensure!(
                f.source < x.entities.len() && f.target < x.entities.len(),
                "invalid extracted endpoint"
            );
            crate::domain::text(&f.fact, 8192, "fact")?;
            crate::domain::text(&f.name, 512, "fact name")?;
            attrs(&f.attributes)?;
            if let Some(t) = &f.valid_at {
                timestamp(t)?;
            }
        }
        let texts: Vec<String> = x
            .entities
            .iter()
            .map(|n| n.name.clone())
            .chain(x.facts.iter().map(|f| f.fact.clone()))
            .collect();
        let vectors = self.embed(&texts).await?;
        let entities: Vec<LegacyEntity> = x
            .entities
            .into_iter()
            .enumerate()
            .map(|(i, n)| {
                let mut labels = n.labels;
                if !labels.iter().any(|x| x == "Entity") {
                    labels.insert(0, "Entity".into());
                }
                LegacyEntity {
                    uuid: uuid::Uuid::new_v4().to_string(),
                    group_id: job.group.clone(),
                    name: n.name,
                    summary: n.summary,
                    created_at: job.created.clone(),
                    labels,
                    attributes: n.attributes,
                    name_embedding: Some(vectors[i].clone()),
                }
            })
            .collect();
        let facts: Vec<LegacyFact> = x
            .facts
            .into_iter()
            .enumerate()
            .map(|(i, f)| LegacyFact {
                uuid: uuid::Uuid::new_v4().to_string(),
                group_id: job.group.clone(),
                source_node_uuid: entities[f.source].uuid.clone(),
                target_node_uuid: entities[f.target].uuid.clone(),
                name: f.name,
                fact: f.fact,
                created_at: job.created.clone(),
                valid_at: Some(f.valid_at.unwrap_or_else(|| job.created.clone())),
                invalid_at: None,
                expired_at: None,
                episodes: vec![job.id.clone()],
                attributes: f.attributes,
                fact_embedding: Some(vectors[entities.len() + i].clone()),
            })
            .collect();
        let prepared = Prepared {
            baseline: None,
            episode: LegacyEpisode {
                uuid: job.id.clone(),
                group_id: job.group.clone(),
                name: job.request.name.clone(),
                content: job.request.episode_body.clone(),
                source: job.request.source.clone(),
                source_description: job.request.source_description.clone(),
                created_at: job.created.clone(),
                valid_at: job.created.clone(),
                entity_edges: facts.iter().map(|f| f.uuid.clone()).collect(),
            },
            entities,
            facts,
        };
        prepared.validate()?;
        Ok(prepared)
    }
}
/// Observed adapter decoding: fenced/prefixed JSON, first complete value, exact value-envelope.
pub fn decode_extraction(content: &str) -> Result<Value> {
    ensure!(
        content.len() <= 2 * 1024 * 1024,
        "extraction response exceeds bound"
    );
    let mut cleaned = content.trim();
    if cleaned.starts_with("```") {
        if let Some((_, body)) = cleaned.split_once('\n') {
            cleaned = body;
        }
        if let Some(body) = cleaned.strip_suffix("```") {
            cleaned = body.trim_end();
        }
    }
    if let Some(start) = cleaned.find(['{', '[']) {
        cleaned = &cleaned[start..];
    }
    if let Some(end) = cleaned.rfind(['}', ']']) {
        cleaned = &cleaned[..=end];
    }
    let mut parsed: Value = serde_json::Deserializer::from_str(cleaned)
        .into_iter::<Value>()
        .next()
        .context("empty extraction JSON")?
        .map_err(|_| anyhow::anyhow!("invalid extraction JSON"))?;
    if let Some(map) = parsed.as_object() {
        let allowed = [
            "$defs",
            "additionalProperties",
            "definitions",
            "description",
            "properties",
            "required",
            "title",
            "type",
        ];
        if map.get("type") == Some(&json!("object"))
            && map.keys().all(|k| allowed.contains(&k.as_str()))
            && let Some(props) = map.get("properties").and_then(Value::as_object)
            && props.len() == 2
            && props.contains_key("entities")
            && props.contains_key("facts")
            && props
                .values()
                .all(|v| v.as_object().is_some_and(|v| v.contains_key("value")))
        {
            parsed = json!({"entities":props["entities"]["value"],"facts":props["facts"]["value"]});
        }
    }
    ensure!(parsed.is_object(), "extraction response must be an object");
    Ok(parsed)
}
pub fn validate_vector(vector: &[f32]) -> Result<()> {
    ensure!(
        vector.len() == 3072
            && vector.iter().all(|x| x.is_finite())
            && vector.iter().any(|x| *x != 0.0),
        "invalid Gemini embedding dimensions or values"
    );
    Ok(())
}
async fn response(request: reqwest::RequestBuilder) -> Result<Value> {
    let mut response = request
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("model request failed"))?;
    ensure!(
        response.status().is_success(),
        "model returned unsuccessful status"
    );
    let mut bytes = vec![];
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow::anyhow!("model response failed"))?
    {
        ensure!(
            bytes.len() + chunk.len() <= 2 * 1024 * 1024,
            "model response exceeds bound"
        );
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("invalid model JSON"))
}
