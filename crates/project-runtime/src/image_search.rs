use futures::{FutureExt as _, pin_mut, select_biased};
use nocterm_containers::Engine;
use nocterm_project_providers::{
    ImageSearch, ImageSearchError, ImageSearchFuture, ImageSearchQuery, RegistryImage,
    WorkspaceImage,
};
use nocterm_session::{ExecError, HostExec, local_fs::CancelFlag};
use serde::Deserialize;
use std::{sync::Arc, time::Duration};

const LIMIT: usize = 25;
const BYTE_LIMIT: usize = 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Default)]
pub struct NativeImageSearch;
impl ImageSearch for NativeImageSearch {
    fn search(
        &self,
        exec: Arc<dyn HostExec>,
        engine: Engine,
        query: ImageSearchQuery,
        cancel: CancelFlag,
    ) -> ImageSearchFuture<Vec<RegistryImage>> {
        async move { search(exec, engine, query, cancel, TIMEOUT).await }.boxed()
    }
}

async fn search(
    exec: Arc<dyn HostExec>,
    engine: Engine,
    query: ImageSearchQuery,
    cancel: CancelFlag,
    timeout: Duration,
) -> Result<Vec<RegistryImage>, ImageSearchError> {
    if cancel.is_cancelled() {
        return Err(ImageSearchError::Canceled);
    }
    let format = match engine {
        Engine::Docker => "{{json .}}",
        Engine::Podman => "json",
    };
    let request = engine.command().args([
        "search",
        "--limit",
        "25",
        "--no-trunc",
        "--format",
        format,
        query.as_str(),
    ]);
    let operation = async {
        let mut output = exec.exec(request).await.map_err(|error| match error {
            ExecError::NotFound(_) | ExecError::Unsupported => ImageSearchError::Unavailable,
            error => ImageSearchError::Execution(error),
        })?;
        let mut bytes = Vec::new();
        while let Some(chunk) = output.next().await {
            if bytes.len().saturating_add(chunk.len()) > BYTE_LIMIT {
                return Err(ImageSearchError::OutputLimit);
            }
            bytes.extend(chunk);
        }
        let exit = output.exit().await;
        if !exit.success() {
            return Err(failure(exit.status, &exit.stderr));
        }
        decode(engine, &bytes)
    }
    .fuse();
    let deadline = async_io::Timer::after(timeout).fuse();
    pin_mut!(operation, deadline);
    loop {
        let tick = async_io::Timer::after(Duration::from_millis(50)).fuse();
        pin_mut!(tick);
        select_biased! {
            _ = deadline => return Err(ImageSearchError::TimedOut),
            _ = tick => if cancel.is_cancelled() { return Err(ImageSearchError::Canceled); },
            result = operation => return if cancel.is_cancelled() { Err(ImageSearchError::Canceled) } else { result },
        }
    }
}
fn failure(status: Option<u32>, stderr: &str) -> ImageSearchError {
    let message = stderr.to_ascii_lowercase();
    if message.contains("429")
        || message.contains("too many requests")
        || message.contains("toomanyrequests")
    {
        ImageSearchError::RateLimited {
            cooldown: Duration::from_secs(60),
        }
    } else if status == Some(127) {
        ImageSearchError::Unavailable
    } else {
        ImageSearchError::CommandFailed { status }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DockerResult {
    name: String,
    description: String,
    is_official: String,
    star_count: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PodmanResult {
    name: String,
    description: String,
    #[serde(default)]
    official: String,
    #[serde(default)]
    stars: u64,
}

fn decode(engine: Engine, bytes: &[u8]) -> Result<Vec<RegistryImage>, ImageSearchError> {
    let mut images = match engine {
        Engine::Docker => {
            let text = std::str::from_utf8(bytes).map_err(|_| ImageSearchError::InvalidResponse)?;
            text.lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| {
                    let row: DockerResult = serde_json::from_str(line).map_err(codec)?;
                    let official = match row.is_official.as_str() {
                        "true" => true,
                        "false" | "" => false,
                        _ => return Err(ImageSearchError::InvalidResponse),
                    };
                    let stars = row
                        .star_count
                        .parse()
                        .map_err(|_| ImageSearchError::InvalidResponse)?;
                    Ok(RegistryImage {
                        reference: docker_reference(row.name),
                        description: row.description,
                        official,
                        stars: Some(stars),
                    })
                })
                .collect::<Result<Vec<_>, _>>()?
        }
        Engine::Podman => {
            if bytes.iter().all(u8::is_ascii_whitespace) {
                return Ok(Vec::new());
            }
            let rows: Vec<PodmanResult> = serde_json::from_slice(bytes).map_err(codec)?;
            rows.into_iter()
                .map(|row| RegistryImage {
                    reference: row.name,
                    description: row.description,
                    official: matches!(row.official.as_str(), "[OK]" | "true"),
                    stars: Some(row.stars),
                })
                .collect()
        }
    };
    if images.iter().any(|image| {
        image.reference.split('/').any(str::is_empty)
            || !image.reference.contains('/')
            || !image
                .reference
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-/:[]".contains(&byte))
            || WorkspaceImage::Custom(image.reference.clone())
                .validate()
                .is_err()
    }) {
        return Err(ImageSearchError::InvalidResponse);
    }
    images.truncate(LIMIT);
    Ok(images)
}
fn docker_reference(name: String) -> String {
    if name.contains('/') {
        format!("docker.io/{name}")
    } else {
        format!("docker.io/library/{name}")
    }
}
fn codec(error: serde_json::Error) -> ImageSearchError {
    error.into()
}

#[cfg(test)]
mod tests;
