//! The work of one file tool call over a terminal's filesystem, and the answer the agent reads.
use std::sync::Arc;

use nocterm_session::{
    EntryKind, FsError, RemoteFs,
    fs::{DownloadReadError, UploadMode, read_bounded},
};
use serde_json::{Value, json};

use super::{
    DeletePath, EditFile, MAX_FILE_LOAD_BYTES, MAX_FILE_READ_BYTES, MAX_LISTED_ENTRIES,
    MAX_READ_FILE_LINES, ReadFile, WriteFile,
};
use crate::{TerminalCall, ToolFailure};

/// What redaction leaves where it hid text.
const REDACTED: &str = "[REDACTED";
/// The chunk a file is read or written in.
const CHUNK: usize = 32 * 1024;

/// Runs the file tool call `call` on `fs`. Text read is redacted when `redact` is set, and a
/// write that would put a redaction marker into a file is then refused: hidden text must never
/// be replaced by its marker.
pub async fn run(
    call: TerminalCall,
    fs: Arc<dyn RemoteFs>,
    redact: bool,
) -> Result<Value, ToolFailure> {
    let fs = fs.as_ref();
    match call {
        TerminalCall::ListFiles(request) => list(fs, &request.path).await,
        TerminalCall::ReadFile(request) => read(fs, &request, redact).await,
        TerminalCall::WriteFile(request) => write(fs, &request, redact).await,
        TerminalCall::EditFile(request) => edit(fs, &request).await,
        TerminalCall::MakeDirectory(request) => {
            fs.create_dir(&request.path).await.map_err(failed)?;
            Ok(json!({"path": request.path, "created": true}))
        }
        TerminalCall::DeletePath(request) => delete(fs, &request).await,
        TerminalCall::MovePath(request) => {
            fs.rename(&request.from, &request.to)
                .await
                .map_err(failed)?;
            Ok(json!({"from": request.from, "to": request.to}))
        }
        _ => Err("This call is not a file tool.".into()),
    }
}

fn failed(error: FsError) -> ToolFailure {
    error.to_string().into()
}

fn kind(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Directory => "directory",
        EntryKind::File => "file",
        EntryKind::Other => "other",
    }
}

/// Folders first, then files, each by name; at most [`MAX_LISTED_ENTRIES`].
async fn list(fs: &dyn RemoteFs, path: &str) -> Result<Value, ToolFailure> {
    let mut entries = fs.read_dir(path).await.map_err(failed)?;
    entries.sort_by(|a, b| {
        (a.kind != EntryKind::Directory, &a.name).cmp(&(b.kind != EntryKind::Directory, &b.name))
    });
    let truncated = entries.len() > MAX_LISTED_ENTRIES;
    let listed: Vec<Value> = entries
        .iter()
        .take(MAX_LISTED_ENTRIES)
        .map(
            |entry| match entry.size.filter(|_| entry.kind == EntryKind::File) {
                Some(size) => json!({"name": entry.name, "type": kind(entry.kind), "size": size}),
                None => json!({"name": entry.name, "type": kind(entry.kind)}),
            },
        )
        .collect();
    Ok(json!({"path": path, "entries": listed, "truncated": truncated}))
}

/// The whole file as text, at most [`MAX_FILE_LOAD_BYTES`].
async fn load(fs: &dyn RemoteFs, path: &str) -> Result<String, ToolFailure> {
    let mut download = fs.download(path).await.map_err(failed)?;
    let bytes = read_bounded(download.as_mut(), MAX_FILE_LOAD_BYTES, CHUNK).await;
    let _ = download.close().await;
    let bytes = bytes.map_err(|error| -> ToolFailure {
        match error {
            DownloadReadError::TooLarge { .. } => format!(
                "{path}: larger than {} KiB; copy it with transfer_files instead",
                MAX_FILE_LOAD_BYTES / 1024
            )
            .into(),
            DownloadReadError::Read(error) => failed(error),
            other => other.to_string().into(),
        }
    })?;
    String::from_utf8(bytes)
        .map_err(|_| format!("{path}: not UTF-8 text; copy it with transfer_files instead").into())
}

/// Lines `start_line..` of the file, as many as fit [`MAX_FILE_READ_BYTES`]; a longer single
/// line is cut and said to be.
async fn read(fs: &dyn RemoteFs, request: &ReadFile, redact: bool) -> Result<Value, ToolFailure> {
    let text = load(fs, &request.path).await?;
    let start = request.start_line.unwrap_or(1);
    let max = request.max_lines.unwrap_or(MAX_READ_FILE_LINES);
    let total = text.split_inclusive('\n').count();
    let (mut content, mut shown, mut cut) = (String::new(), 0, false);
    for line in text.split_inclusive('\n').skip(start - 1).take(max) {
        if content.len() + line.len() > MAX_FILE_READ_BYTES {
            if shown == 0 {
                let mut end = MAX_FILE_READ_BYTES;
                while !line.is_char_boundary(end) {
                    end -= 1;
                }
                content.push_str(&line[..end]);
                (shown, cut) = (1, true);
            }
            break;
        }
        content.push_str(line);
        shown += 1;
    }
    let next = start + shown;
    let mut answer = json!({
        "path": request.path,
        "start_line": start,
        "lines": shown,
        "total_lines": total,
        "content": content,
    });
    if next <= total {
        answer["next_start_line"] = json!(next);
    }
    if cut {
        answer["line_cut"] = json!(true);
    }
    if redact {
        let hidden = crate::redact::redact(&content);
        if hidden != content {
            answer["content"] = json!(hidden);
            answer["redacted"] = json!(true);
            answer["note"] = json!(
                "Text shown as [REDACTED] is hidden, not the file's text: change other parts with edit_file."
            );
        }
    }
    Ok(answer)
}

/// Writes `bytes` as the whole file at `path`.
async fn store(
    fs: &dyn RemoteFs,
    path: &str,
    bytes: &[u8],
    mode: UploadMode,
) -> Result<(), ToolFailure> {
    let refused = |error: FsError| -> ToolFailure {
        match error {
            FsError::AlreadyExists { .. } if mode == UploadMode::Create => format!(
                "{path}: already exists; set overwrite to replace it, or change it with edit_file"
            )
            .into(),
            error => failed(error),
        }
    };
    let mut upload = fs.upload(path, mode).await.map_err(refused)?;
    for chunk in bytes.chunks(CHUNK) {
        if let Err(error) = upload.write(chunk.to_vec()).await {
            let _ = upload.cancel().await;
            return Err(refused(error));
        }
    }
    upload.finish().await.map_err(refused)?;
    Ok(())
}

async fn write(fs: &dyn RemoteFs, request: &WriteFile, redact: bool) -> Result<Value, ToolFailure> {
    if redact && request.content.contains(REDACTED) {
        return Err(
            "The content holds a [REDACTED] marker, which would replace hidden text: change the file with edit_file instead."
                .into(),
        );
    }
    let mode = if request.overwrite {
        UploadMode::Replace
    } else {
        UploadMode::Create
    };
    store(fs, &request.path, request.content.as_bytes(), mode).await?;
    Ok(json!({"path": request.path, "bytes": request.content.len()}))
}

/// Replaces `old_text` in the file's real text, which redaction never touched.
async fn edit(fs: &dyn RemoteFs, request: &EditFile) -> Result<Value, ToolFailure> {
    let path = &request.path;
    let text = load(fs, path).await?;
    let found = text.matches(request.old_text.as_str()).count();
    if found == 0 {
        return Err(format!(
            "{path}: old_text was not found. Read the file again and copy the text exactly; text shown as [REDACTED] cannot be matched."
        )
        .into());
    }
    if found > 1 && !request.replace_all {
        return Err(format!(
            "{path}: old_text occurs {found} times; include more surrounding text, or set replace_all."
        )
        .into());
    }
    let (changed, replaced) = if request.replace_all {
        (text.replace(&request.old_text, &request.new_text), found)
    } else {
        (text.replacen(&request.old_text, &request.new_text, 1), 1)
    };
    if changed.len() > MAX_FILE_LOAD_BYTES {
        return Err(format!(
            "{path}: the edited file would be larger than {} KiB",
            MAX_FILE_LOAD_BYTES / 1024
        )
        .into());
    }
    store(fs, path, changed.as_bytes(), UploadMode::Replace).await?;
    Ok(json!({"path": path, "replacements": replaced, "bytes": changed.len()}))
}

/// Removes a folder as a folder and anything else as a file.
async fn delete(fs: &dyn RemoteFs, request: &DeletePath) -> Result<Value, ToolFailure> {
    let path = &request.path;
    let directory = match fs.stat(path).await {
        Ok(Some(entry)) => entry.kind == EntryKind::Directory && !entry.is_symlink,
        Ok(None) => return Err(failed(FsError::NotFound { path: path.clone() })),
        Err(FsError::Unsupported(_)) => false,
        Err(error) => return Err(failed(error)),
    };
    if directory {
        fs.remove_dir(path).await.map_err(failed)?;
    } else {
        fs.remove_file(path).await.map_err(failed)?;
    }
    Ok(json!({"path": path, "deleted": if directory { "directory" } else { "file" }}))
}
