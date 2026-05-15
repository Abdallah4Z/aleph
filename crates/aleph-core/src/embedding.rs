//! Text and vision encoders backed by Candle (pure Rust ML inference).
//!
//! ## Text
//!
//! [`MiniLmEncoder`] wraps `sentence-transformers/all-MiniLM-L6-v2` via `candle-transformers`.
//! Output dimension: 384. Input is L2-normalized before return.
//!
//! ## Vision
//!
//! [`SiglipEncoder`] wraps `google/siglip-base-patch16-224` via `candle-transformers`.
//! Output dimension: 768. Input is L2-normalized before return.

use anyhow::Result;
use candle_core::{Device, Module, Tensor};
use candle_transformers::models::bert::{BertModel, Config, DTYPE};
use std::path::Path;
use tokenizers::Tokenizer;

pub const TEXT_DIM: usize = 384;

/// Converts a text string into a fixed-size embedding vector.
pub trait TextEncoder: Send + Sync {
    /// Encode `text` into a normalized 384-dimensional vector.
    fn encode(&self, text: &str) -> Result<Vec<f32>>;
}

/// [`all-MiniLM-L6-v2`](https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2)
/// text encoder loaded from local weight files.
pub struct MiniLmEncoder {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
}

impl MiniLmEncoder {
    /// Load weights and tokenizer from a local directory.
    ///
    /// Expected files:
    /// - `config.json`
    /// - `tokenizer.json`
    /// - `model.safetensors` (or `pytorch_model.bin` as fallback)
    pub fn from_dir<P: AsRef<Path>>(model_dir: P) -> Result<Self> {
        let device = Device::Cpu;
        let model_dir = model_dir.as_ref();

        let config_path = model_dir.join("config.json");
        let weights_path = model_dir.join("model.safetensors");
        let tokenizer_path = model_dir.join("tokenizer.json");

        let config: Config = serde_json::from_str(&std::fs::read_to_string(&config_path)?)?;
        let tokenizer = Tokenizer::from_file(tokenizer_path).map_err(|e| anyhow::anyhow!(e))?;

        let vb = if weights_path.exists() {
            unsafe {
                candle_nn::VarBuilder::from_mmaped_safetensors(&[weights_path], DTYPE, &device)?
            }
        } else {
            let bin_path = model_dir.join("pytorch_model.bin");
            candle_nn::VarBuilder::from_pth(&bin_path, DTYPE, &device)?
        };

        let model = BertModel::load(vb, &config)?;

        Ok(Self {
            model,
            tokenizer,
            device,
        })
    }
}

impl TextEncoder for MiniLmEncoder {
    fn encode(&self, text: &str) -> Result<Vec<f32>> {
        let tokens = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| anyhow::anyhow!(e))?;
        let token_ids = tokens.get_ids().iter().map(|&x| x as u32).collect::<Vec<_>>();

        let input_ids = Tensor::new(&token_ids[..], &self.device)?.unsqueeze(0)?;
        let token_type_ids = input_ids.zeros_like()?;
        let attention_mask = input_ids.ones_like()?;

        let embeddings = self
            .model
            .forward(&input_ids, &token_type_ids, Some(&attention_mask))?;

        // Mean pooling
        let pooled = embeddings.mean(1)?;
        let pooled = pooled.squeeze(0)?;
        let vec = pooled.to_vec1::<f32>()?;

        // L2 normalize
        let norm = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        let normalized = vec.iter().map(|x| x / (norm + 1e-12)).collect();

        Ok(normalized)
    }
}

// ---------------------------------------------------------------------------
// Vision
// ---------------------------------------------------------------------------

pub const IMAGE_DIM: usize = 768;

/// Converts an image into a fixed-size embedding vector.
pub trait VisionEncoder: Send + Sync {
    /// Encode `image` into a normalized 768-dimensional vector.
    fn encode_image(&self, image: &image::DynamicImage) -> Result<Vec<f32>>;
}

/// [`google/siglip-base-patch16-224`](https://huggingface.co/google/siglip-base-patch16-224)
/// vision encoder loaded from local weight files.
pub struct SiglipEncoder {
    model: candle_transformers::models::siglip::VisionModel,
    device: Device,
}

impl SiglipEncoder {
    /// Load SigLIP vision weights from a local directory.
    ///
    /// Expected files:
    /// - `config.json`
    /// - `model.safetensors`
    pub fn from_dir<P: AsRef<Path>>(model_dir: P) -> Result<Self> {
        let device = Device::Cpu;
        let model_dir = model_dir.as_ref();

        let weights_path = model_dir.join("model.safetensors");
        if !weights_path.exists() {
            anyhow::bail!("SigLIP weights not found at {}", weights_path.display());
        }

        // Build vision config from known siglip-base-patch16-224 parameters
        // The config.json lacks full vision_config details, so we use defaults
        let vision_config = candle_transformers::models::siglip::VisionConfig {
            hidden_size: 768,
            intermediate_size: 3072,
            num_hidden_layers: 12,
            num_attention_heads: 12,
            num_channels: 3,
            image_size: 224,
            patch_size: 16,
            hidden_act: candle_nn::Activation::GeluPytorchTanh,
            layer_norm_eps: 1e-6,
        };

        let vb = unsafe {
            candle_nn::VarBuilder::from_mmaped_safetensors(
                &[&weights_path],
                candle_transformers::models::bert::DTYPE,
                &device,
            )?
        };

        // The weights are stored under "vision_model." prefix in the HF safetensors
        let model = candle_transformers::models::siglip::VisionModel::new(
            &vision_config,
            true,
            vb.pp("vision_model"),
        )?;

        tracing::info!("SigLIP vision encoder loaded from {:?}", model_dir);
        Ok(Self { model, device })
    }

    /// Preprocess an image: resize to 224×224, normalize to [-1, 1].
    fn preprocess(&self, image: &image::DynamicImage) -> Result<Tensor> {
        use image::imageops::FilterType;

        let image = image.resize_exact(224, 224, FilterType::CatmullRom);
        let rgb = image.to_rgb8();
        let (w, h) = (rgb.width() as usize, rgb.height() as usize);

        // CHW layout: [C, H, W] flat array, then unsqueeze batch dim
        let mut data = vec![0.0f32; 3 * h * w];
        for (x, y, pixel) in rgb.enumerate_pixels() {
            let idx = y as usize * w + x as usize;
            // Normalize from [0, 255] to [-1, 1]: (x/255 - 0.5) / 0.5
            data[idx] = (pixel[0] as f32 / 255.0 - 0.5) / 0.5;
            data[h * w + idx] = (pixel[1] as f32 / 255.0 - 0.5) / 0.5;
            data[2 * h * w + idx] = (pixel[2] as f32 / 255.0 - 0.5) / 0.5;
        }

        let tensor = Tensor::from_slice(&data, &[1, 3, h, w], &self.device)?;
        Ok(tensor)
    }
}

impl VisionEncoder for SiglipEncoder {
    fn encode_image(&self, image: &image::DynamicImage) -> Result<Vec<f32>> {
        let pixel_values = self.preprocess(image)?;
        let features = self.model.forward(&pixel_values)?;

        // features shape: [1, hidden_size] — take first (only) batch item
        let vec: Vec<f32> = features.squeeze(0)?.to_vec1()?;

        // L2 normalize
        let norm = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        let normalized = vec.iter().map(|x| x / (norm + 1e-12)).collect();

        Ok(normalized)
    }
}
