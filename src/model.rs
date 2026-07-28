use std::env;
use std::path::{Path, PathBuf};

const MODEL_FILES: &[&str] = &[
    "encoder.int8.onnx",
    "decoder.onnx",
    "joiner.int8.onnx",
    "tokens.txt",
];

pub const DEFAULT_MODEL_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-streaming-zipformer-zh-xlarge-int8-2025-06-30.tar.bz2";
pub const ENV_MODEL_URL: &str = "ASR_MODEL_URL";

pub fn resolve_model_url(cli_url: Option<&str>) -> String {
    if let Some(url) = cli_url {
        return url.to_string();
    }
    if let Ok(env_url) = env::var(ENV_MODEL_URL) {
        if !env_url.is_empty() {
            tracing::info!("Using model URL from ${ENV_MODEL_URL}: {env_url}");
            return env_url;
        }
    }
    DEFAULT_MODEL_URL.to_string()
}

pub fn canonicalize(path: impl AsRef<Path>) -> anyhow::Result<PathBuf> {
    let path = path.as_ref();
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(env::current_dir()?.join(path))
    }
}

pub fn verify(dir: &Path) -> bool {
    MODEL_FILES.iter().all(|name| {
        let f = dir.join(name);
        f.exists() && f.metadata().map(|m| m.len() > 0).unwrap_or(false)
    })
}

fn detect_proxy() -> Option<String> {
    for key in &[
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        if let Ok(val) = env::var(key) {
            if !val.is_empty() {
                tracing::info!("Detected proxy from {key}: {val}");
                return Some(val);
            }
        }
    }
    None
}

fn build_client() -> anyhow::Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder();
    if let Some(proxy_url) = detect_proxy() {
        builder = builder.proxy(reqwest::Proxy::all(&proxy_url)?);
    }
    Ok(builder.build()?)
}

pub async fn ensure(dir: &Path, url: &str) -> anyhow::Result<()> {
    if verify(dir) {
        tracing::info!("Model verified at {:?}", dir);
        return Ok(());
    }

    tracing::info!("Model not found at {:?}, source: {url}", dir);
    std::fs::create_dir_all(dir)?;

    if let Some(path) = url.strip_prefix("file://") {
        let archive_path = Path::new(path);
        tracing::info!("Using local archive: {:?}", archive_path);
        if !archive_path.exists() {
            anyhow::bail!("Local archive not found: {:?}", archive_path);
        }
        extract_archive(archive_path, dir)?;
    } else {
        download(dir, url).await?;
    }

    if !verify(dir) {
        anyhow::bail!("Model files incomplete after extraction");
    }

    tracing::info!("Model ready at {:?}", dir);
    Ok(())
}

async fn download(dir: &Path, url: &str) -> anyhow::Result<()> {
    let client = build_client()?;

    tracing::info!("Downloading model archive from {url}");

    let tmp_path = dir.join("model.tar.bz2.part");
    {
        let response = client.get(url).send().await?;
        let total = response.content_length().unwrap_or(0);
        if total > 0 {
            tracing::info!(
                "  total size: {} bytes ({:.1} MB)",
                total,
                total as f64 / 1_048_576.0
            );
        }

        let mut dest = tokio::fs::File::create(&tmp_path).await?;
        let mut downloaded: u64 = 0;
        let mut stream = response.bytes_stream();

        use futures_util::StreamExt;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            tokio::io::AsyncWriteExt::write_all(&mut dest, &chunk).await?;
            downloaded += chunk.len() as u64;
            if total > 0 {
                let pct = downloaded * 100 / total;
                tracing::info!("  progress: {pct}% ({downloaded} / {total} bytes)");
            }
        }
        tokio::io::AsyncWriteExt::flush(&mut dest).await?;
        tracing::info!("Download complete: {downloaded} bytes");
    }

    extract_archive(&tmp_path, dir)?;
    std::fs::remove_file(&tmp_path)?;

    Ok(())
}

fn extract_archive(archive_path: &Path, dir: &Path) -> anyhow::Result<()> {
    tracing::info!("Extracting model files...");
    let file = std::fs::File::open(archive_path)?;
    let decoder = bzip2::read::BzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?;
        let Some(name) = path.file_name() else {
            continue;
        };
        let name_str = name.to_owned().to_string_lossy().into_owned();
        if MODEL_FILES.contains(&name_str.as_str()) {
            let target = dir.join(&name_str);
            entry.unpack(&target)?;
            tracing::info!(
                "  extracted: {name_str} ({} bytes)",
                target.metadata().map(|m| m.len()).unwrap_or(0)
            );
        }
    }
    Ok(())
}
