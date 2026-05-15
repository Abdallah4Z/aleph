mod atspi;
mod pipeline;

use aleph_core::embedding::{MiniLmEncoder, SiglipEncoder};
use aleph_core::{Config, ContextExtractor};
use anyhow::Result;
use pipeline::Pipeline;
use tracing_subscriber::EnvFilter;

pub async fn run_daemon(config: &Config) -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive(
                    format!("aleph={}", config.general.log_level)
                        .parse()
                        .unwrap(),
                )
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let data_dir = config.data_dir();
    std::fs::create_dir_all(&data_dir)?;

    let extractor = atspi::AtSpiExtractor;
    let rx = extractor.subscribe_focus().await?;

    let text_encoder: Box<dyn aleph_core::TextEncoder> = {
        let cache = config.models_dir().join("all-MiniLM-L6-v2");
        if config.encoders.text && cache.join("model.safetensors").exists() {
            match MiniLmEncoder::from_dir(&cache) {
                Ok(enc) => {
                    tracing::info!("MiniLM encoder loaded from {:?}", cache);
                    Box::new(enc) as Box<dyn aleph_core::TextEncoder>
                }
                Err(e) => {
                    tracing::error!("Failed to load MiniLM: {}", e);
                    return Err(e);
                }
            }
        } else {
            tracing::info!("MiniLM weights not found at {:?}. Using hash encoder.", cache);
            Box::new(HashEncoder)
        }
    };

    let vision_encoder: Box<dyn aleph_core::VisionEncoder> = {
        let cache = config.models_dir().join("siglip");
        if config.encoders.vision {
            match SiglipEncoder::from_dir(&cache) {
                Ok(enc) => Box::new(enc),
                Err(e) => {
                    tracing::warn!("SigLIP encoder unavailable: {}. Vision disabled.", e);
                    Box::new(NoopVisionEncoder)
                }
            }
        } else {
            Box::new(NoopVisionEncoder)
        }
    };

    let mut pipeline = Pipeline::new(data_dir, text_encoder, vision_encoder).await?;
    pipeline.run(rx).await?;

    Ok(())
}

struct HashEncoder;

impl aleph_core::TextEncoder for HashEncoder {
    fn encode(&self, text: &str) -> Result<Vec<f32>> {
        let mut vec = Vec::with_capacity(aleph_core::embedding::TEXT_DIM);
        for i in 0..aleph_core::embedding::TEXT_DIM {
            let hash = xxhash_rust::xxh3::xxh3_64(&[text.as_bytes(), &i.to_le_bytes()].concat());
            let val = (hash as f32 / u64::MAX as f32) * 2.0 - 1.0;
            vec.push(val);
        }
        let norm = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        Ok(vec.iter().map(|x| x / (norm + 1e-12)).collect())
    }
}

struct NoopVisionEncoder;

impl aleph_core::VisionEncoder for NoopVisionEncoder {
    fn encode_image(&self, _image: &image::DynamicImage) -> Result<Vec<f32>> {
        Err(anyhow::anyhow!("SigLIP vision model not installed"))
    }
}
