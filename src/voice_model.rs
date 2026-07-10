use std::{
    env, fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, TryRecvError},
};

use anyhow::{Context, Result, anyhow, bail};
use directories::ProjectDirs;
use sha2::{Digest, Sha256};

pub const MODEL_DOWNLOAD_BYTES: u64 = 59_707_625;
pub const MODEL_DISPLAY_SIZE: &str = "57 MiB";
const MODEL_FILE_NAME: &str = "ggml-base-q5_1.bin";
const MODEL_SHA256: &str = "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898";
const MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base-q5_1.bin";

pub fn model_ready() -> bool {
    if let Some(path) = env::var_os("GRIDBASH_VOICE_MODEL")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return path.is_file();
    }
    default_model_path().is_ok_and(|path| model_file_looks_complete(&path))
}

pub fn configured_model_path() -> Result<PathBuf> {
    if let Some(path) = env::var_os("GRIDBASH_VOICE_MODEL")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return Ok(path);
    }
    default_model_path()
}

pub fn ensure_model(cancel_rx: &Receiver<()>) -> Result<PathBuf> {
    let custom = env::var_os("GRIDBASH_VOICE_MODEL")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    if let Some(path) = custom {
        if path.is_file() {
            return Ok(path);
        }
        bail!("configured voice model does not exist: {}", path.display());
    }

    let path = default_model_path()?;
    if model_file_looks_complete(&path) {
        return Ok(path);
    }
    download_default_model(&path, cancel_rx)?;
    Ok(path)
}

fn default_model_path() -> Result<PathBuf> {
    ProjectDirs::from("", "", "GridBash")
        .map(|dirs| dirs.data_local_dir().join("models").join(MODEL_FILE_NAME))
        .ok_or_else(|| anyhow!("failed to resolve the GridBash model directory"))
}

fn model_file_looks_complete(path: &Path) -> bool {
    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() == MODEL_DOWNLOAD_BYTES)
}

fn download_default_model(path: &Path, cancel_rx: &Receiver<()>) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("voice model path has no parent directory"))?;
    fs::create_dir_all(parent).with_context(|| {
        format!(
            "failed to create voice model directory {}",
            parent.display()
        )
    })?;
    let partial = path.with_extension("bin.part");

    let result = download_to_partial(&partial, cancel_rx);
    if let Err(error) = result {
        let _ = fs::remove_file(&partial);
        return Err(error);
    }

    fs::rename(&partial, path).with_context(|| {
        format!(
            "failed to install downloaded voice model at {}",
            path.display()
        )
    })
}

fn download_to_partial(path: &Path, cancel_rx: &Receiver<()>) -> Result<()> {
    let mut response = reqwest::blocking::get(MODEL_URL)
        .context("failed to download the offline voice model")?
        .error_for_status()
        .context("voice model download returned an error")?;
    let mut file = fs::File::create(path)
        .with_context(|| format!("failed to create model download {}", path.display()))?;
    let mut digest = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];

    loop {
        match cancel_rx.try_recv() {
            Ok(()) | Err(TryRecvError::Disconnected) => {
                bail!("voice model download canceled");
            }
            Err(TryRecvError::Empty) => {}
        }

        let count = response
            .read(&mut buffer)
            .context("failed while downloading the voice model")?;
        if count == 0 {
            break;
        }
        file.write_all(&buffer[..count])
            .context("failed to write the voice model download")?;
        digest.update(&buffer[..count]);
        total = total.saturating_add(count as u64);
    }
    file.sync_all()
        .context("failed to flush the voice model download")?;

    if total != MODEL_DOWNLOAD_BYTES {
        bail!(
            "voice model download was incomplete: expected {MODEL_DOWNLOAD_BYTES} bytes, received {total}"
        );
    }
    let actual = format!("{:x}", digest.finalize());
    if actual != MODEL_SHA256 {
        bail!("voice model checksum mismatch");
    }
    Ok(())
}
