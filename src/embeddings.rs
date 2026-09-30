use crate::{config::Config, models::ModelApi};
use anyhow::{Context, Result, bail, ensure};
use serde_json::json;
#[cfg(feature = "local-embeddings")]
use std::sync::Arc;
#[cfg(feature = "local-embeddings")]
pub mod local;
#[derive(Clone)]
pub enum Embedder {
    Disabled,
    External {
        api: ModelApi,
        model: String,
        dimension: usize,
    },
    #[cfg(feature = "local-embeddings")]
    Local(Arc<local::LocalEmbedder>),
}
impl Embedder {
    pub async fn new(c: &Config) -> Result<Self> {
        match c.embedding_provider.as_str() {
            "disabled" => Ok(Self::Disabled),
            "external" => Ok(Self::External {
                api: ModelApi::new(c)?,
                model: c.embedding_model.clone(),
                dimension: c.embedding_dim,
            }),
            "local" => {
                #[cfg(feature = "local-embeddings")]
                {
                    let directory = c
                        .local_model_dir
                        .clone()
                        .context("LOCAL_MODEL_DIR missing")?;
                    let model = c.embedding_model.clone();
                    let dim = c.embedding_dim;
                    let backend = tokio::task::spawn_blocking(move || {
                        local::LocalEmbedder::load(&directory, &model, dim)
                    })
                    .await??;
                    Ok(Self::Local(Arc::new(backend)))
                }
                #[cfg(not(feature = "local-embeddings"))]
                {
                    bail!("rebuild with --features local-embeddings to use local embeddings")
                }
            }
            _ => bail!("unknown embedding provider"),
        }
    }
    pub fn space(&self) -> Option<String> {
        match self {
            Self::Disabled => None,
            Self::External {
                model,
                dimension,
                api,
            } => Some(format!(
                "external:{}:{model}:{dimension}",
                api.endpoint_identity()
            )),
            #[cfg(feature = "local-embeddings")]
            Self::Local(local) => Some(local.space()),
        }
    }
    pub async fn embed(&self, texts: &[String]) -> Result<Option<Vec<Vec<f32>>>> {
        ensure!(
            texts.len() <= 256
                && texts
                    .iter()
                    .all(|s| !s.trim().is_empty() && s.len() <= 8192),
            "embedding input exceeds limits"
        );
        if texts.is_empty() {
            return Ok(if matches!(self, Self::Disabled) {
                None
            } else {
                Some(vec![])
            });
        }
        match self {
            Self::Disabled => Ok(None),
            Self::External {
                api,
                model,
                dimension,
            } => {
                let mut output = vec![];
                for chunk in texts.chunks(8) {
                    let response = api
                        .post("embeddings", json!({"model":model,"input":chunk}))
                        .await?;
                    let data = response
                        .get("data")
                        .and_then(|v| v.as_array())
                        .context("embedding API missing data")?;
                    ensure!(data.len() == chunk.len(), "embedding count mismatch");
                    let mut indexed = vec![None; chunk.len()];
                    for item in data {
                        let index = item
                            .get("index")
                            .and_then(|v| v.as_u64())
                            .context("missing embedding index")?
                            as usize;
                        ensure!(
                            index < chunk.len() && indexed[index].is_none(),
                            "invalid or duplicate embedding index"
                        );
                        indexed[index] = Some(serde_json::from_value::<Vec<f32>>(
                            item.get("embedding")
                                .context("missing embedding vector")?
                                .clone(),
                        )?);
                    }
                    output.extend(
                        indexed
                            .into_iter()
                            .collect::<Option<Vec<_>>>()
                            .context("incomplete embedding response")?,
                    );
                }
                validate_vectors(&output, texts.len(), *dimension)?;
                Ok(Some(output))
            }
            #[cfg(feature = "local-embeddings")]
            Self::Local(local) => {
                let permit = local.worker.clone().acquire_owned().await?;
                let backend = local.clone();
                let texts = texts.to_vec();
                let result = tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    backend.embed(&texts)
                })
                .await??;
                Ok(Some(result))
            }
        }
    }
}
pub fn validate_vectors(v: &[Vec<f32>], count: usize, dimension: usize) -> Result<()> {
    ensure!(v.len() == count, "embedding count mismatch");
    ensure!(
        dimension > 0
            && v.iter()
                .all(|v| v.len() == dimension && v.iter().all(|x| x.is_finite())),
        "embedding dimension mismatch or non-finite values"
    );
    Ok(())
}
pub fn cosine(a: &[f32], b: &[f32]) -> Result<f32> {
    ensure!(
        !a.is_empty() && a.len() == b.len(),
        "embedding dimension mismatch"
    );
    ensure!(
        a.iter().chain(b).all(|x| x.is_finite()),
        "non-finite embedding"
    );
    let (dot, aa, bb) = a
        .iter()
        .zip(b)
        .fold((0.0f64, 0.0f64, 0.0f64), |(d, aa, bb), (&a, &b)| {
            let a = a as f64;
            let b = b as f64;
            (d + a * b, aa + a * a, bb + b * b)
        });
    Ok(if aa == 0.0 || bb == 0.0 {
        0.0
    } else {
        (dot / (aa * bb).sqrt()) as f32
    })
}
