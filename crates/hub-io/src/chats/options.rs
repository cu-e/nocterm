//! A hub chat's settings as the configuration options of its session: its model among the
//! agent's models, whether changing tools wait for an approval, and which of the account's
//! servers the chat has attached (one flag per server; the agent signs in only to those).

use nocterm_ai::model::controls::{ConfigCategory, ConfigChoice, ConfigOption, ConfigSetting};
use nocterm_hub_agents_model::HubServer;
use nocterm_hub_proto::v2;

/// The configuration option that chooses the chat's model.
const MODEL: &str = "model";
/// The configuration option that chooses whether changing tools wait for an approval.
const APPROVAL: &str = "approval";
/// The prefix of the options that attach one server each: `server:<id>`.
const SERVER: &str = "server:";
const AUTO: &str = "auto";
const ASK: &str = "ask";

/// What a choice changed, so the hub is told with the call that stores it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Changed {
    /// The model or the approval mode (`SetChatModel`).
    Model,
    /// The attached servers (`SetChatServers`).
    Servers,
}

/// A choice the chat does not take.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Refused(pub(super) &'static str);

/// The chat's settings as last stored on the hub, or as chosen for a chat its first message
/// has not created yet.
#[derive(Clone, Debug)]
pub(super) struct ChatOptions {
    model: String,
    approval: i32,
    models: Vec<String>,
    title: String,
    servers: Vec<String>,
}

impl ChatOptions {
    pub(super) fn of(chat: &v2::Chat, models: &[String]) -> Self {
        Self {
            model: chat.model.clone(),
            approval: chat.approval,
            models: models.to_vec(),
            title: chat.title.clone(),
            servers: chat.server_ids.clone(),
        }
    }

    /// The same settings after the hub stored `chat`.
    pub(super) fn stored(&self, chat: &v2::Chat) -> Self {
        Self::of(chat, &self.models)
    }

    pub(super) fn title(&self) -> &str {
        &self.title
    }

    /// The model and approval mode as `SetChatModel` takes them.
    pub(super) fn model(&self) -> (String, i32) {
        (self.model.clone(), self.approval)
    }

    /// The ids of the attached servers, as `SetChatServers` takes them.
    pub(super) fn servers(&self) -> Vec<String> {
        self.servers.clone()
    }

    /// Whether `chat` shows other options than these.
    pub(super) fn differs(&self, chat: &v2::Chat) -> bool {
        chat.model != self.model
            || chat.approval != self.approval
            || chat.server_ids != self.servers
    }

    /// The options of the chat, with a flag for each server of `offered`, by name. A server
    /// attached but no longer offered stays attached and is not shown.
    pub(super) fn config(&self, offered: &[HubServer]) -> Vec<ConfigOption> {
        let mut options = self.model_options();
        let mut servers: Vec<&HubServer> = offered.iter().collect();
        servers.sort_by_cached_key(|server| (server.name.to_lowercase(), server.id.to_string()));
        options.extend(servers.into_iter().map(|server| {
            let id = server.id.to_string();
            let name = if server.name.is_empty() {
                server.host.clone()
            } else {
                server.name.clone()
            };
            let attached = self.servers.contains(&id);
            ConfigOption::boolean(format!("{SERVER}{id}"), name, attached)
                .description(address(server))
                .category(ConfigCategory::Server)
        }));
        options
    }

    fn model_options(&self) -> Vec<ConfigOption> {
        let mut choices: Vec<ConfigChoice> = self
            .models
            .iter()
            .map(|model| ConfigChoice::new(model.clone(), model.clone()))
            .collect();
        if !self.model.is_empty() && !self.models.contains(&self.model) {
            choices.insert(0, ConfigChoice::new(self.model.clone(), self.model.clone()));
        }
        let ask = v2::ApprovalMode::try_from(self.approval) == Ok(v2::ApprovalMode::Ask);
        vec![
            ConfigOption::select(MODEL, "Model", self.model.clone(), choices)
                .category(ConfigCategory::Model),
            ConfigOption::select(
                APPROVAL,
                "Approval",
                if ask { ASK } else { AUTO },
                vec![
                    ConfigChoice::new(AUTO, "Auto"),
                    ConfigChoice::new(ASK, "Ask before changes"),
                ],
            )
            .category(ConfigCategory::Mode),
        ]
    }

    /// Applies a choice of the option `config`, telling which call stores it.
    pub(super) fn choose(
        &mut self,
        config: &str,
        setting: &ConfigSetting,
    ) -> Result<Changed, Refused> {
        if let Some(server) = config.strip_prefix(SERVER) {
            let ConfigSetting::Boolean(attach) = setting else {
                return Err(Refused("A server is attached or not."));
            };
            self.servers.retain(|id| id != server);
            if *attach {
                self.servers.push(server.to_owned());
            }
            return Ok(Changed::Servers);
        }
        let ConfigSetting::Choice(choice) = setting else {
            return Err(Refused("The option takes a choice."));
        };
        match config {
            MODEL => self.model = choice.to_string(),
            APPROVAL => {
                self.approval = match choice.as_str() {
                    AUTO => v2::ApprovalMode::Auto.into(),
                    ASK => v2::ApprovalMode::Ask.into(),
                    _ => return Err(Refused("Unknown approval mode.")),
                }
            }
            _ => return Err(Refused("The chat has no such option.")),
        }
        Ok(Changed::Model)
    }
}

/// `user@host`, the port when it is not 22, and what keeps agents out when something does.
fn address(server: &HubServer) -> String {
    let mut text = format!("{}@{}", server.user, server.host);
    if server.port != 22 {
        text.push_str(&format!(":{}", server.port));
    }
    let status = server.status;
    if !status.is_ready() && status != nocterm_hub_agents_model::ServerStatus::Unchecked {
        text.push_str(" · ");
        text.push_str(status.label());
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_hub_agents_model::{ServerId, ServerStatus};

    fn server(id: &str, name: &str, status: ServerStatus) -> HubServer {
        HubServer {
            id: ServerId::new(id.to_owned()).unwrap(),
            name: name.to_owned(),
            host: format!("{id}.example"),
            port: 22,
            user: "root".to_owned(),
            host_key_fingerprint: String::new(),
            status,
            status_message: String::new(),
        }
    }

    fn chat(servers: &[&str]) -> ChatOptions {
        let chat = v2::Chat {
            model: "m1".into(),
            server_ids: servers.iter().map(ToString::to_string).collect(),
            ..Default::default()
        };
        ChatOptions::of(&chat, &["m1".into(), "m2".into()])
    }

    fn flags(options: &[ConfigOption]) -> Vec<(String, String, bool)> {
        options
            .iter()
            .filter(|option| option.category == Some(ConfigCategory::Server))
            .map(|option| {
                let on = matches!(
                    option.kind,
                    nocterm_ai::model::controls::ConfigKind::Boolean(true)
                );
                (option.name.clone(), option.description.clone().unwrap(), on)
            })
            .collect()
    }

    #[test]
    fn every_server_of_the_account_is_a_flag_on_while_the_chat_has_it() {
        let offered = [
            server("b", "Web", ServerStatus::Ready),
            server("a", "build", ServerStatus::NoAccess),
        ];
        let options = chat(&["b"]).config(&offered);
        assert_eq!(options[0].category, Some(ConfigCategory::Model));
        assert_eq!(
            flags(&options),
            vec![
                (
                    "build".into(),
                    "root@a.example · No access — reconnect".into(),
                    false
                ),
                ("Web".into(), "root@b.example".into(), true),
            ]
        );
    }

    #[test]
    fn attaching_and_detaching_a_server_is_stored_with_set_chat_servers() {
        let mut options = chat(&["b"]);
        let attach = options.choose("server:a", &ConfigSetting::Boolean(true));
        assert_eq!(attach, Ok(Changed::Servers));
        assert_eq!(options.servers(), vec!["b".to_owned(), "a".to_owned()]);
        options
            .choose("server:b", &ConfigSetting::Boolean(false))
            .unwrap();
        assert_eq!(options.servers(), vec!["a".to_owned()]);
        assert!(
            options
                .choose("server:a", &ConfigSetting::Choice("x".into()))
                .is_err()
        );
    }

    #[test]
    fn the_model_and_the_approval_mode_are_stored_with_set_chat_model() {
        let mut options = chat(&[]);
        let model = options.choose("model", &ConfigSetting::Choice("m2".into()));
        assert_eq!(model, Ok(Changed::Model));
        options
            .choose("approval", &ConfigSetting::Choice("ask".into()))
            .unwrap();
        assert_eq!(
            options.model(),
            ("m2".to_owned(), v2::ApprovalMode::Ask.into())
        );
        assert!(
            options
                .choose("approval", &ConfigSetting::Choice("never".into()))
                .is_err()
        );
        assert!(
            options
                .choose("other", &ConfigSetting::Boolean(true))
                .is_err()
        );
    }

    #[test]
    fn a_chat_stored_with_other_servers_differs() {
        let options = chat(&["a"]);
        let mut stored = v2::Chat {
            model: "m1".into(),
            server_ids: vec!["a".into()],
            ..Default::default()
        };
        assert!(!options.differs(&stored));
        stored.server_ids.clear();
        assert!(options.differs(&stored));
    }
}
