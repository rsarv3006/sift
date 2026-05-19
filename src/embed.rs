use std::path::PathBuf;

use anyhow::{bail, Context, Result};

/// Controls which embedding backend to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedBackend {
    /// Try candle (local) first, fall back to API on failure.
    Auto,
    /// Use candle for local inference only.
    Local,
    /// Use API only.
    Api,
}

/// Configuration for the embedding system.
#[derive(Debug, Clone)]
pub struct EmbedConfig {
    /// Which backend(s) to use.
    pub backend: EmbedBackend,
    /// Path to a local model directory (e.g. downloaded all-MiniLM-L6-v2).
    pub model_path: Option<PathBuf>,
    /// API key for the fallback embedding API.
    pub api_key: Option<String>,
    /// API endpoint URL (default: OpenAI-compatible).
    pub api_url: Option<String>,
    /// Model name for the API (e.g. "text-embedding-3-small").
    pub api_model: Option<String>,
}

impl EmbedConfig {
    pub fn from_env() -> Self {
        Self {
            backend: match std::env::var("SIFT_EMBED_BACKEND")
                .as_deref()
                .unwrap_or("auto")
            {
                "local" => EmbedBackend::Local,
                "api" => EmbedBackend::Api,
                _ => EmbedBackend::Auto,
            },
            model_path: std::env::var("SIFT_EMBED_MODEL_PATH").ok().map(Into::into),
            api_key: std::env::var("SIFT_EMBED_API_KEY").ok(),
            api_url: std::env::var("SIFT_EMBED_API_URL").ok(),
            api_model: Some(
                std::env::var("SIFT_EMBED_API_MODEL")
                    .unwrap_or_else(|_| "text-embedding-3-small".into()),
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// Embedder trait
// ---------------------------------------------------------------------------

pub trait Embedder {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>;
}

impl Embedder for AutoEmbedder {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        #[cfg(feature = "candle")]
        if let Some(wrapper) = &self.local {
            return wrapper.inner.embed_texts(texts);
        }
        self.api.embed(texts)
    }
}

// ---------------------------------------------------------------------------
// API embedder (always available)
// ---------------------------------------------------------------------------

pub struct ApiEmbedder {
    api_key: String,
    api_url: String,
    model: String,
    client: reqwest::blocking::Client,
}

impl ApiEmbedder {
    pub fn new(config: &EmbedConfig) -> Result<Self> {
        let api_key = config
            .api_key
            .clone()
            .or_else(|| std::env::var("OPENAI_API_KEY").ok())
            .unwrap_or_default();
        let api_url = config
            .api_url
            .clone()
            .unwrap_or_else(|| "https://api.openai.com/v1/embeddings".into());
        let model = config.api_model.clone().unwrap_or_else(|| "text-embedding-3-small".into());
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()?;
        Ok(Self { api_key, api_url, model, client })
    }
}

impl Embedder for ApiEmbedder {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        #[derive(serde::Serialize)]
        struct Request<'a> {
            input: Vec<&'a str>,
            model: &'a str,
        }
        #[derive(serde::Deserialize)]
        struct Response {
            data: Vec<Data>,
        }
        #[derive(serde::Deserialize)]
        struct Data {
            embedding: Vec<f32>,
        }

        let mut req = self.client.post(&self.api_url);
        if !self.api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", self.api_key));
        }
        let resp = req
            .json(&Request { input: texts.to_vec(), model: &self.model })
            .send()
            .context("API embedding request failed")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            bail!("API embedding error ({}): {}", status, body);
        }

        let body: Response = resp.json().context("Failed to parse API embedding response")?;
        if body.data.len() != texts.len() {
            bail!(
                "API returned {} embeddings for {} texts",
                body.data.len(),
                texts.len()
            );
        }
        Ok(body.data.into_iter().map(|d| d.embedding).collect())
    }
}

// ---------------------------------------------------------------------------
// Candle-based local embedder (feature-gated)
// ---------------------------------------------------------------------------

#[cfg(feature = "candle")]
pub mod local {
    use std::path::Path;
    use anyhow::{Context, Result};
    use candle_core::{Device, Tensor};
    use candle_nn::VarBuilder;
    use candle_transformers::models::bert::{BertModel, Config, DTYPE};
    use hf_hub::api::sync::Api;
    use tokenizers::Tokenizer;

    pub struct LocalEmbedder {
        model: BertModel,
        tokenizer: Tokenizer,
        device: Device,
    }

    impl LocalEmbedder {
        pub fn new(model_path: Option<&Path>) -> Result<Self> {
            let device = Device::cuda_if_available(0).unwrap_or(Device::Cpu);

            let (model, tokenizer) = if let Some(path) = model_path {
                let tokenizer_path = path.join("tokenizer.json");
                let model_path = path.join("model.safetensors");
                let config_path = path.join("config.json");
                let tokenizer = Tokenizer::from_file(tokenizer_path)
                    .map_err(|e| anyhow::anyhow!("Failed to load tokenizer: {}", e))?;
                let config_s = std::fs::read_to_string(config_path)
                    .context("Failed to read config.json")?;
                let config: Config = serde_json::from_str(&config_s)
                    .context("Failed to parse config.json")?;
                let vb = unsafe {
                    VarBuilder::from_mmaped_safetensors(&[model_path], DTYPE, &device)
                        .context("Failed to load model.safetensors")?
                };
                let model = BertModel::load(vb, &config)?;
                (model, tokenizer)
            } else {
                let api = Api::new().context("Failed to init hf-hub API")?;
                let repo = api.model("sentence-transformers/all-MiniLM-L6-v2".into());
                let tokenizer_path = repo.get("tokenizer.json")?;
                let model_path = repo.get("model.safetensors")?;
                let config_path = repo.get("config.json")?;
                let tokenizer = Tokenizer::from_file(tokenizer_path)
                    .map_err(|e| anyhow::anyhow!("Failed to load tokenizer: {}", e))?;
                let config_s = std::fs::read_to_string(config_path)
                    .context("Failed to read config.json")?;
                let config: Config = serde_json::from_str(&config_s)
                    .context("Failed to parse config.json")?;
                let vb = unsafe {
                    VarBuilder::from_mmaped_safetensors(&[model_path], DTYPE, &device)
                        .context("Failed to load model.safetensors")?
                };
                let model = BertModel::load(vb, &config)?;
                (model, tokenizer)
            };

            Ok(Self { model, tokenizer, device })
        }

        fn mean_pool(&self, token_embeddings: &Tensor, attention_mask: &Tensor) -> Result<Tensor> {
            let mask = attention_mask
                .unsqueeze(2)?
                .to_dtype(candle_core::DType::F32)?;
            let sum_embeddings = (token_embeddings * &mask)?.sum(1)?;
            let sum_mask = mask.sum(1)?;
            let mean = (&sum_embeddings / &sum_mask)?;
            Ok(mean)
        }

        fn normalize(&self, v: &Tensor) -> Result<Tensor> {
            let norm = v.sqr()?.sum_keepdim(1)?.sqrt()?;
            Ok((v / norm)?)
        }

        pub fn embed_texts(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
            let max_length = 128;
            let mut all_embeddings = Vec::with_capacity(texts.len());

            for text in texts {
                let encoding = self
                    .tokenizer
                    .encode(*text, true)
                    .map_err(|e| anyhow::anyhow!("Tokenization failed: {}", e))?;

                let token_ids = encoding
                    .get_ids()
                    .iter()
                    .map(|&id| id as u32)
                    .collect::<Vec<_>>();
                let attention = encoding
                    .get_attention_mask()
                    .iter()
                    .map(|&m| m as u32)
                    .collect::<Vec<_>>();

                let token_ids = if token_ids.len() > max_length {
                    let mut t = vec![token_ids[0]];
                    t.extend_from_slice(&token_ids[1..max_length - 1]);
                    t.push(token_ids[token_ids.len() - 1]);
                    t
                } else {
                    token_ids
                };

                let seq_len = token_ids.len();
                let input = Tensor::new(token_ids.as_slice(), &self.device)?.unsqueeze(0)?;
                let mask = if attention.len() > token_ids.len() {
                    Tensor::new(&attention[..seq_len], &self.device)?.unsqueeze(0)?
                } else {
                    Tensor::new(attention.as_slice(), &self.device)?.unsqueeze(0)?
                };
                let type_ids = input.zeros_like()?;

                let output = self.model.forward(&input, &type_ids, Some(&mask))?;
                let pooled = self.mean_pool(&output, &mask)?;
                let normalized = self.normalize(&pooled)?;

                let vec: Vec<f32> = normalized.to_vec1()?;
                all_embeddings.push(vec);
            }

            Ok(all_embeddings)
        }
    }
}

// ---------------------------------------------------------------------------
// Auto embedder: try local, fall back to API
// ---------------------------------------------------------------------------

#[allow(dead_code)]
pub struct AutoEmbedder {
    local: Option<LocalWrapper>,
    api: ApiEmbedder,
}

struct LocalWrapper {
    #[cfg(feature = "candle")]
    inner: local::LocalEmbedder,
}

impl AutoEmbedder {
    pub fn new(config: &EmbedConfig) -> Result<Self> {
        let local = match config.backend {
            EmbedBackend::Api => None,
            _ => Self::try_local(config),
        };
        let api = ApiEmbedder::new(config)?;
        Ok(Self { local, api })
    }

    #[cfg(feature = "candle")]
    fn try_local(config: &EmbedConfig) -> Option<LocalWrapper> {
        match local::LocalEmbedder::new(config.model_path.as_deref()) {
            Ok(inner) => {
                eprintln!("[sift] using local embedding model (candle)");
                Some(LocalWrapper { inner })
            }
            Err(e) => {
                eprintln!("[sift] local model unavailable ({}), falling back to API", e);
                None
            }
        }
    }

    #[cfg(not(feature = "candle"))]
    fn try_local(_config: &EmbedConfig) -> Option<LocalWrapper> {
        None
    }

}

// ---------------------------------------------------------------------------
// Cosine similarity
// ---------------------------------------------------------------------------

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    let dot: f64 = a.iter().zip(b).map(|(x, y)| *x as f64 * *y as f64).sum();
    let na: f64 = a.iter().map(|x| *x as f64 * *x as f64).sum::<f64>().sqrt();
    let nb: f64 = b.iter().map(|x| *x as f64 * *x as f64).sum::<f64>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

pub fn top_k_similar(query: &[f32], candidates: &[(usize, &[f32])], k: usize) -> Vec<(usize, f64)> {
    let mut scores: Vec<(usize, f64)> = candidates
        .iter()
        .map(|(id, vec)| (*id, cosine_similarity(query, vec)))
        .collect();
    scores.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scores.truncate(k);
    scores
}
