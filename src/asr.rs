use crate::config::Config;

pub struct AsrEngine {
    #[allow(dead_code)]
    config: Config,
}

pub struct AsrStream {
    #[allow(dead_code)]
    segment: u32,
}

impl AsrEngine {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        tracing::info!("Loading ASR model from {:?}", config.model);
        tracing::info!("  decoding_method: {}", config.decoding_method);
        tracing::info!("  max_active_paths: {}", config.max_active_paths);
        tracing::info!(
            "  num_threads: {}",
            config.num_threads.unwrap_or_else(|| {
                std::thread::available_parallelism()
                    .map(|n| n.get() as i32)
                    .unwrap_or(4)
            })
        );

        Ok(Self { config })
    }

    pub fn create_stream(&self) -> AsrStream {
        AsrStream { segment: 0 }
    }

    pub fn accept_waveform(&self, _stream: &mut AsrStream, _pcm: &[i16]) -> Option<String> {
        None
    }

    pub fn flush(&self, _stream: &mut AsrStream) -> Option<(String, Vec<String>, Vec<f64>)> {
        None
    }

    pub fn reset(&self, _stream: &mut AsrStream) {
        _stream.segment += 1;
    }
}
