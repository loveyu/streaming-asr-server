use std::path::PathBuf;

use crate::config::Config;

pub struct AsrEngine {
    recognizer: sherpa_onnx::OnlineRecognizer,
}

pub struct AsrStream {
    stream: sherpa_onnx::OnlineStream,
    sample_rate: i32,
    segment: u32,
}

fn model_path(dir: &PathBuf, name: &str) -> String {
    dir.join(name).to_string_lossy().into_owned()
}

impl AsrEngine {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        let model_dir = &config.model_dir;

        let mut cfg = sherpa_onnx::OnlineRecognizerConfig::default();
        cfg.model_config.transducer.encoder =
            Some(model_path(model_dir, "encoder.int8.onnx"));
        cfg.model_config.transducer.decoder =
            Some(model_path(model_dir, "decoder.onnx"));
        cfg.model_config.transducer.joiner =
            Some(model_path(model_dir, "joiner.int8.onnx"));
        cfg.model_config.tokens = Some(model_path(model_dir, "tokens.txt"));
        cfg.decoding_method = Some(config.decoding_method.clone());
        cfg.max_active_paths = config.max_active_paths;
        cfg.enable_endpoint = true;
        cfg.rule1_min_trailing_silence = config.endpoint_silence;
        cfg.rule3_min_utterance_length = config.endpoint_max_utterance;

        if let Some(threads) = config.num_threads {
            cfg.model_config.num_threads = threads;
        }

        let sid = sherpa_onnx::git_sha1();
        let ver = sherpa_onnx::version();
        tracing::info!("sherpa-onnx: version={ver} git={sid}");
        tracing::info!(
            "  num_threads: {}  decoding_method: {}",
            cfg.model_config.num_threads,
            cfg.decoding_method.as_deref().unwrap_or("default"),
        );

        let recognizer = sherpa_onnx::OnlineRecognizer::create(&cfg)
            .ok_or_else(|| anyhow::anyhow!("Failed to create OnlineRecognizer"))?;

        tracing::info!("OnlineRecognizer created");

        Ok(Self {
            recognizer,
        })
    }

    pub fn create_stream(&self, sample_rate: i32) -> AsrStream {
        let stream = self.recognizer.create_stream();
        AsrStream {
            stream,
            sample_rate,
            segment: 0,
        }
    }

    pub fn accept_waveform(&self, stream: &mut AsrStream, pcm: &[i16]) -> Option<String> {
        let samples: Vec<f32> = pcm.iter().map(|&s| s as f32 / 32768.0).collect();
        stream.stream.accept_waveform(stream.sample_rate, &samples);

        while self.recognizer.is_ready(&stream.stream) {
            self.recognizer.decode(&stream.stream);
        }

        self.recognizer
            .get_result(&stream.stream)
            .map(|r| r.text)
    }

    pub fn flush(&self, stream: &mut AsrStream) -> Option<(String, Vec<String>, Vec<f64>)> {
        stream.stream.input_finished();

        while self.recognizer.is_ready(&stream.stream) {
            self.recognizer.decode(&stream.stream);
        }

        self.recognizer.get_result(&stream.stream).map(|r| {
            (
                r.text,
                r.tokens,
                r.timestamps
                    .unwrap_or_default()
                    .into_iter()
                    .map(|t| t as f64)
                    .collect(),
            )
        })
    }

    pub fn reset(&self, stream: &mut AsrStream) {
        self.recognizer.reset(&stream.stream);
        stream.segment += 1;
    }
}
