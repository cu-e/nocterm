//! Initialization extracts only consumed facts, keeping original pinned defaults.
use super::{authentication, copy, error::DecodeError, lists, value::Value};
use nocterm_ai::{
    AgentInfo,
    model::{
        json::deferred::JsonDecodeError,
        native::{NativeList, NativeOwned},
        session::AgentCapabilities,
    },
};
type Error = DecodeError<JsonDecodeError>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Restore {
    Unsupported,
    Load,
    Resume,
}
pub(super) struct Initialized {
    version: u16,
    info: NativeOwned<AgentInfo>,
    restore: Restore,
}
impl Initialized {
    pub(super) fn version(&self) -> u16 {
        self.version
    }
    pub(super) fn info(&self) -> &AgentInfo {
        self.info.get()
    }
    pub(super) fn restore(&self) -> Restore {
        self.restore
    }
    pub(super) fn into_info(self) -> NativeOwned<AgentInfo> {
        self.info
    }
}
fn default_fields<'a, const N: usize>(
    value: Option<Value<'a>>,
    names: [&str; N],
) -> Result<[Option<Value<'a>>; N], Error> {
    match value {
        Some(value) => {
            Ok(lists::shape(value.fields(names))?.unwrap_or_else(|| std::array::from_fn(|_| None)))
        }
        None => Ok(std::array::from_fn(|_| None)),
    }
}
fn boolean(value: Option<Value<'_>>) -> bool {
    value.is_some_and(|value| value.raw() == "true")
}
fn present(value: Option<Value<'_>>) -> Result<bool, Error> {
    match value {
        Some(value) if value.raw() != "null" => {
            Ok(lists::shape(value.fields(["_meta"]))?.is_some())
        }
        _ => Ok(false),
    }
}
fn capabilities(
    value: Option<Value<'_>>,
    fallback: &Value<'_>,
) -> Result<(NativeOwned<AgentCapabilities>, Restore), Error> {
    let [load, prompt, _mcp, session, _auth, _meta] = default_fields(
        value,
        [
            "loadSession",
            "promptCapabilities",
            "mcpCapabilities",
            "sessionCapabilities",
            "auth",
            "_meta",
        ],
    )?;
    let [image, audio, embedded, _meta] =
        default_fields(prompt, ["image", "audio", "embeddedContext", "_meta"])?;
    let [_list, _delete, _directories, fork, resume, close, _meta] = default_fields(
        session,
        [
            "list",
            "delete",
            "additionalDirectories",
            "fork",
            "resume",
            "close",
            "_meta",
        ],
    )?;
    let restore = if present(resume)? {
        Restore::Resume
    } else if boolean(load) {
        Restore::Load
    } else {
        Restore::Unsupported
    };
    Ok((
        nocterm_ai::model::native::session::capabilities(
            boolean(image),
            boolean(audio),
            boolean(embedded),
            restore != Restore::Unsupported,
            present(fork)?,
            present(close)?,
            fallback.ledger(),
        )?,
        restore,
    ))
}
fn implementation<'a>(value: Value<'a>) -> Result<(Value<'a>, Value<'a>), Error> {
    let [name, _title, version, _meta] = value.fields(["name", "title", "version", "_meta"])?;
    let name = Value::required(name, "implementation.name")?;
    let version = Value::required(version, "implementation.version")?;
    if !name.raw().starts_with('"') || !version.raw().starts_with('"') {
        return Err(DecodeError::Field("implementation strings"));
    }
    Ok((name, version))
}
pub(super) fn initialized(value: Value<'_>, fallback_name: &str) -> Result<Initialized, Error> {
    let [version, capabilities_value, auth, info, _meta] = value.fields([
        "protocolVersion",
        "agentCapabilities",
        "authMethods",
        "agentInfo",
        "_meta",
    ])?;
    let version = Value::required(version, "protocolVersion")?
        .raw()
        .parse::<u16>()
        .map_err(|_| DecodeError::Field("protocolVersion"))?;
    let (capabilities, restore) = capabilities(capabilities_value, &value)?;
    let (name, version_text) = match info {
        Some(info) => match lists::shape(implementation(info))? {
            Some((name, version)) => (name.text()?, version.text()?),
            None => (
                copy::text(value.budget, fallback_name)?,
                copy::text(value.budget, "")?,
            ),
        },
        None => (
            copy::text(value.budget, fallback_name)?,
            copy::text(value.budget, "")?,
        ),
    };
    let auth = match auth {
        Some(auth) => authentication::methods(auth)?,
        None => NativeList::new(value.ledger(), 0)?.finish(),
    };
    Ok(Initialized {
        version,
        info: nocterm_ai::model::native::session::agent_info(
            name,
            version_text,
            capabilities,
            auth,
        )?,
        restore,
    })
}
