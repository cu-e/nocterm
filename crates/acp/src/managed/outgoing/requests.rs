//! Used request shapes serialize borrowed fields directly into prepaid wire bytes.
use super::{
    Error, SourceBudget, call,
    content::{Content, Read, Text},
};
use agent_client_protocol::managed::{Envelope, MessageId};
use nocterm_ai::{
    history::physical::DispatchPayload,
    model::session::{ConfigurationSelection, ExternalToolEndpoint},
};
use serde::{
    Serialize, Serializer,
    ser::{SerializeMap, SerializeSeq},
};
use std::path::Path;
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Initialize {
    protocol_version: u16,
    client_capabilities: Capabilities,
    client_info: Implementation,
}
#[derive(Serialize)]
struct Implementation {
    name: &'static str,
    version: &'static str,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Capabilities {
    fs: FileSystem,
    terminal: bool,
    auth: Auth,
    session: Session,
}
#[derive(Serialize)]
struct Empty {}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileSystem {
    read_text_file: bool,
    write_text_file: bool,
}
#[derive(Serialize)]
struct Auth {
    terminal: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Session {
    config_options: BooleanCapability,
}
#[derive(Serialize)]
struct BooleanCapability {
    boolean: Empty,
}
pub(in crate::managed) fn initialize(
    budget: &SourceBudget,
    id: &MessageId<SourceBudget>,
    terminal: bool,
) -> Result<Envelope<SourceBudget>, Error> {
    call(
        budget,
        Some(id),
        "initialize",
        Initialize {
            protocol_version: 1,
            client_info: Implementation {
                name: "nocterm",
                version: "0",
            },
            client_capabilities: Capabilities {
                fs: FileSystem {
                    read_text_file: false,
                    write_text_file: false,
                },
                terminal: false,
                auth: Auth { terminal },
                session: Session {
                    config_options: BooleanCapability { boolean: Empty {} },
                },
            },
        },
    )
}
struct Endpoints<'a>(&'a [ExternalToolEndpoint]);
impl Serialize for Endpoints<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(self.0.len()))?;
        for value in self.0 {
            seq.serialize_element(&Endpoint(value))?;
        }
        seq.end()
    }
}
struct Pairs<'a>(&'a [(String, String)]);
impl Serialize for Pairs<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Pair<'a> {
            name: &'a str,
            value: &'a str,
        }
        let mut seq = s.serialize_seq(Some(self.0.len()))?;
        for (name, value) in self.0 {
            seq.serialize_element(&Pair { name, value })?;
        }
        seq.end()
    }
}
struct Endpoint<'a>(&'a ExternalToolEndpoint);
impl Serialize for Endpoint<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(None)?;
        match self.0 {
            ExternalToolEndpoint::LocalCommand { name, launch } => {
                map.serialize_entry("name", name)?;
                map.serialize_entry("command", &launch.program)?;
                map.serialize_entry("args", &launch.args)?;
                map.serialize_entry("env", &Pairs(&launch.env))?;
            }
            ExternalToolEndpoint::Http {
                name,
                endpoint,
                headers,
            }
            | ExternalToolEndpoint::EventStream {
                name,
                endpoint,
                headers,
            } => {
                map.serialize_entry(
                    "type",
                    if matches!(self.0, ExternalToolEndpoint::Http { .. }) {
                        "http"
                    } else {
                        "sse"
                    },
                )?;
                map.serialize_entry("name", name)?;
                map.serialize_entry("url", endpoint)?;
                map.serialize_entry("headers", &Pairs(headers))?;
            }
        }
        map.end()
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Open<'a> {
    cwd: &'a Path,
    mcp_servers: Endpoints<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<&'a str>,
}
pub(in crate::managed) fn open(
    budget: &SourceBudget,
    id: &MessageId<SourceBudget>,
    method: &str,
    cwd: &Path,
    endpoints: &[ExternalToolEndpoint],
    session: Option<&str>,
) -> Result<Envelope<SourceBudget>, Error> {
    if cwd.to_str().is_none() {
        return Err(super::DecodeError::InvalidPath);
    }
    for endpoint in endpoints {
        if let ExternalToolEndpoint::LocalCommand { launch, .. } = endpoint
            && launch.program.to_str().is_none()
        {
            return Err(super::DecodeError::InvalidPath);
        }
    }
    call(
        budget,
        Some(id),
        method,
        Open {
            cwd,
            mcp_servers: Endpoints(endpoints),
            session_id: session,
        },
    )
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionOnly<'a> {
    session_id: &'a str,
}
pub(in crate::managed) fn session(
    budget: &SourceBudget,
    id: Option<&MessageId<SourceBudget>>,
    method: &str,
    session: &str,
) -> Result<Envelope<SourceBudget>, Error> {
    call(
        budget,
        id,
        method,
        SessionOnly {
            session_id: session,
        },
    )
}
pub(in crate::managed) fn mode(
    budget: &SourceBudget,
    id: &MessageId<SourceBudget>,
    session: &str,
    mode: &str,
) -> Result<Envelope<SourceBudget>, Error> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Mode<'a> {
        session_id: &'a str,
        mode_id: &'a str,
    }
    call(
        budget,
        Some(id),
        "session/set_mode",
        Mode {
            session_id: session,
            mode_id: mode,
        },
    )
}
pub(in crate::managed) fn authenticate(
    budget: &SourceBudget,
    id: &MessageId<SourceBudget>,
    method: &str,
) -> Result<Envelope<SourceBudget>, Error> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Authenticate<'a> {
        method_id: &'a str,
    }
    call(
        budget,
        Some(id),
        "authenticate",
        Authenticate { method_id: method },
    )
}
pub(in crate::managed) fn model(
    budget: &SourceBudget,
    id: &MessageId<SourceBudget>,
    session: &str,
    model: &str,
) -> Result<Envelope<SourceBudget>, Error> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Model<'a> {
        session_id: &'a str,
        model_id: &'a str,
    }
    call(
        budget,
        Some(id),
        "session/set_model",
        Model {
            session_id: session,
            model_id: model,
        },
    )
}
struct Configuration<'a>(&'a ConfigurationSelection);
impl Serialize for Configuration<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let fields = if matches!(
            &self.0.value,
            nocterm_ai::session_config::ConfigValue::Flag(_)
        ) {
            4
        } else {
            3
        };
        let mut map = s.serialize_map(Some(fields))?;
        map.serialize_entry("sessionId", self.0.session.as_str())?;
        map.serialize_entry("configId", self.0.option.as_str())?;
        match &self.0.value {
            nocterm_ai::session_config::ConfigValue::Flag(value) => {
                map.serialize_entry("type", "boolean")?;
                map.serialize_entry("value", value)?
            }
            nocterm_ai::session_config::ConfigValue::Value(value) => {
                map.serialize_entry("value", value)?
            }
        }
        map.end()
    }
}
pub(in crate::managed) fn configuration(
    budget: &SourceBudget,
    id: &MessageId<SourceBudget>,
    value: &ConfigurationSelection,
) -> Result<Envelope<SourceBudget>, Error> {
    call(
        budget,
        Some(id),
        "session/set_config_option",
        Configuration(value),
    )
}
struct Prompt<'a>(&'a DispatchPayload);
impl Serialize for Prompt<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(2))?;
        map.serialize_entry("sessionId", &Read(self.0.session()))?;
        map.serialize_entry("prompt", &Blocks(self.0))?;
        map.end()
    }
}
struct Blocks<'a>(&'a DispatchPayload);
impl Serialize for Blocks<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let context = self.0.context();
        let history = self.0.history();
        let prompt = self.0.prompt();
        let mut seq = s.serialize_seq(Some(
            1 + prompt.images().len()
                + usize::from(context.is_some())
                + usize::from(history.is_some()),
        ))?;
        if let Some(value) = context {
            seq.serialize_element(&Text::new(&value))?;
        }
        if let Some(value) = history {
            seq.serialize_element(&Text::new(&value))?;
        }
        seq.serialize_element(&Text::new(&prompt.text()))?;
        for value in prompt.images() {
            seq.serialize_element(&Content(value))?;
        }
        seq.end()
    }
}
pub(in crate::managed) fn prompt(
    budget: &SourceBudget,
    id: &MessageId<SourceBudget>,
    value: &DispatchPayload,
) -> Result<Envelope<SourceBudget>, Error> {
    for content in value.prompt().images() {
        if let nocterm_ai::history::physical::ContentSlot::Media(media) = content
            && media.kind() == nocterm_ai::model::content::MediaKind::Audio
            && media.uri().is_some()
        {
            return Err(super::DecodeError::Model(
                nocterm_ai::model::diagnostic::ModelError::Unsupported("audio source URI"),
            ));
        }
    }
    call(budget, Some(id), "session/prompt", Prompt(value))
}
