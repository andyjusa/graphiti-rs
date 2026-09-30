use crate::embeddings::validate_vectors;
use anyhow::{Context, Result, ensure};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use std::{path::Path, sync::Arc};
use tokenizers::{Tokenizer, TruncationParams};
pub struct LocalEmbedder {
    model: BertModel,
    tokenizer: Tokenizer,
    identity: String,
    dimension: usize,
    pub worker: Arc<tokio::sync::Semaphore>,
}
impl LocalEmbedder {
    pub fn load(directory: &str, identity: &str, dimension: usize) -> Result<Self> {
        let path = Path::new(directory);
        let config_bytes =
            std::fs::read(path.join("config.json")).context("cannot read local config.json")?;
        let tokenizer_bytes = std::fs::read(path.join("tokenizer.json"))
            .context("cannot read local tokenizer.json")?;
        let mut config: Config = serde_json::from_slice(&config_bytes)?;
        ensure!(
            config.hidden_size == dimension,
            "local model hidden_size differs from EMBEDDING_DIM"
        );
        ensure!(
            config.max_position_embeddings >= 256,
            "local model must support 256 tokens"
        );
        config.hidden_dropout_prob = 0.0;
        let mut tokenizer = Tokenizer::from_bytes(&tokenizer_bytes)
            .map_err(|e| anyhow::anyhow!("local tokenizer: {e}"))?;
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: 256,
                ..Default::default()
            }))
            .map_err(|e| anyhow::anyhow!("tokenizer truncation: {e}"))?;
        tokenizer.with_padding(None);
        let weights = std::fs::read(path.join("model.safetensors"))
            .context("cannot read local model.safetensors")?;
        use sha2::Digest;
        let digest = crate::domain::stable_id(&[
            identity,
            &format!("{:x}", sha2::Sha256::digest(&weights)),
            &format!("{:x}", sha2::Sha256::digest(&config_bytes)),
            &format!("{:x}", sha2::Sha256::digest(&tokenizer_bytes)),
            "candle-0.11:cpu-f32:dropout-0:mean-l2-256:v1",
        ]);
        let vb = VarBuilder::from_buffered_safetensors(weights, DType::F32, &Device::Cpu)?;
        let model = BertModel::load(vb, &config)?;
        Ok(Self {
            model,
            tokenizer,
            identity: format!("{identity}:{digest}:mean-l2-256"),
            dimension,
            worker: Arc::new(tokio::sync::Semaphore::new(1)),
        })
    }
    pub fn space(&self) -> String {
        format!("local:{}:{}", self.identity, self.dimension)
    }
    pub fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut output = vec![];
        // One sentence at a time keeps peak activation memory independent of caller batch size.
        for text in texts {
            let encoded = self
                .tokenizer
                .encode(text.as_str(), true)
                .map_err(|e| anyhow::anyhow!("tokenization: {e}"))?;
            let ids = Tensor::new(encoded.get_ids(), &Device::Cpu)?.unsqueeze(0)?;
            let types = Tensor::new(encoded.get_type_ids(), &Device::Cpu)?.unsqueeze(0)?;
            let mask = Tensor::new(encoded.get_attention_mask(), &Device::Cpu)?.unsqueeze(0)?;
            let hidden = self
                .model
                .forward(&ids, &types, Some(&mask))?
                .squeeze(0)?
                .to_vec2::<f32>()?;
            let mut pooled = vec![0.0; self.dimension];
            let mut count = 0f32;
            for (token, &active) in hidden.iter().zip(encoded.get_attention_mask()) {
                if active != 0 {
                    for (value, &x) in pooled.iter_mut().zip(token) {
                        *value += x;
                    }
                    count += 1.0;
                }
            }
            ensure!(count > 0.0, "no non-padding tokens");
            for value in &mut pooled {
                *value /= count;
            }
            let norm = pooled.iter().map(|x| x * x).sum::<f32>().sqrt();
            if norm > 0.0 {
                for value in &mut pooled {
                    *value /= norm;
                }
            }
            output.push(pooled);
        }
        validate_vectors(&output, texts.len(), self.dimension)?;
        Ok(output)
    }
}
