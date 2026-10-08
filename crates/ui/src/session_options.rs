//! Shared standard controls for global defaults and per-profile inheritance.
use gpui_kit::{
    App, Context, Entity, Focusable as _, Subscription, Window,
    component::{
        ActiveTheme as _, IndexPath,
        input::{Input, InputEvent, InputState},
        select::{SearchableVec, Select, SelectEvent, SelectState},
        v_flex,
    },
    div,
    prelude::*,
};
use nocterm_settings::{Charset, LoggingOptions, ProxyConfig, SessionOptions, TERM_PRESETS};
type Choice = Entity<SelectState<SearchableVec<&'static str>>>;
pub struct SessionOptionsEditor {
    inherit: bool,
    term: Choice,
    charset: Choice,
    proxy: Choice,
    logging: Choice,
    custom: Entity<InputState>,
    host: Entity<InputState>,
    port: Entity<InputState>,
    directory: Entity<InputState>,
    limit: Entity<InputState>,
    remote_dns: bool,
    _subscriptions: Vec<Subscription>,
}
impl SessionOptionsEditor {
    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    pub fn new(
        options: SessionOptions,
        inherit: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let choice = |mut values: Vec<&'static str>,
                      selected: &str,
                      window: &mut Window,
                      cx: &mut Context<Self>| {
            if inherit {
                values.insert(0, "Inherit");
            }
            let index = values
                .iter()
                .position(|value| *value == selected)
                .unwrap_or(0);
            cx.new(|cx| {
                SelectState::new(
                    SearchableVec::new(values),
                    Some(IndexPath::new(index)),
                    window,
                    cx,
                )
            })
        };
        let mut terms = TERM_PRESETS.to_vec();
        terms.push("Custom");
        let term = choice(
            terms,
            match options.term.as_deref() {
                None if inherit => "Inherit",
                None => TERM_PRESETS[0],
                Some(value) if TERM_PRESETS.contains(&value) => value,
                Some(_) => "Custom",
            },
            window,
            cx,
        );
        let charset = choice(
            Charset::ALL.iter().map(|c| c.label()).collect(),
            options.charset.map(Charset::label).unwrap_or(if inherit {
                "Inherit"
            } else {
                "UTF-8"
            }),
            window,
            cx,
        );
        let proxy = choice(
            vec!["Direct", "HTTP CONNECT", "SOCKS5"],
            match options.proxy {
                None if inherit => "Inherit",
                Some(ProxyConfig::HttpConnect { .. }) => "HTTP CONNECT",
                Some(ProxyConfig::Socks5 { .. }) => "SOCKS5",
                _ => "Direct",
            },
            window,
            cx,
        );
        let logging = choice(
            vec!["Manual only", "Automatic"],
            match options.logging.as_ref() {
                None if inherit => "Inherit",
                Some(log) if log.auto_start => "Automatic",
                _ => "Manual only",
            },
            window,
            cx,
        );
        let (proxy_host, proxy_port, remote_dns) = match options.proxy {
            Some(ProxyConfig::HttpConnect { host, port }) => (host, port, true),
            Some(ProxyConfig::Socks5 {
                host,
                port,
                remote_dns,
            }) => (host, port, remote_dns),
            _ => (String::new(), 1080, true),
        };
        let log = options.logging.unwrap_or_default();
        let field = |text: String, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).default_value(text))
        };
        let custom = field(options.term.unwrap_or_default(), window, cx);
        let host = field(proxy_host, window, cx);
        let port = field(proxy_port.to_string(), window, cx);
        let directory = field(
            log.directory
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
            window,
            cx,
        );
        let limit = field(log.max_file_mib.to_string(), window, cx);
        let mut subscriptions = Vec::new();
        for select in [&term, &charset, &proxy, &logging] {
            subscriptions.push(cx.subscribe(
                select,
                |_, _, _: &SelectEvent<SearchableVec<&'static str>>, cx| cx.notify(),
            ));
        }
        for input in [&custom, &host, &port, &directory, &limit] {
            subscriptions.push(cx.subscribe(input, |_, _, _: &InputEvent, cx| cx.notify()));
        }
        Self {
            inherit,
            term,
            charset,
            proxy,
            logging,
            custom,
            host,
            port,
            directory,
            limit,
            remote_dns,
            _subscriptions: subscriptions,
        }
    }
    fn selected(choice: &Choice, cx: &App) -> &'static str {
        choice
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or("Inherit")
    }
    pub fn options(&self, cx: &App) -> Result<SessionOptions, String> {
        let term = match Self::selected(&self.term, cx) {
            "Inherit" => None,
            "Custom" => Some(self.custom.read(cx).value().trim().to_owned()),
            value => Some(value.to_owned()),
        };
        let charset = Charset::ALL
            .into_iter()
            .find(|c| c.label() == Self::selected(&self.charset, cx));
        let proxy = match Self::selected(&self.proxy, cx) {
            "Inherit" => None,
            "Direct" => Some(ProxyConfig::Direct),
            kind => {
                let host = self.host.read(cx).value().trim().to_owned();
                let port = self
                    .port
                    .read(cx)
                    .value()
                    .trim()
                    .parse()
                    .map_err(|_| "Proxy port must be from 1 to 65535.".to_owned())?;
                Some(if kind == "HTTP CONNECT" {
                    ProxyConfig::HttpConnect { host, port }
                } else {
                    ProxyConfig::Socks5 {
                        host,
                        port,
                        remote_dns: self.remote_dns,
                    }
                })
            }
        };
        let logging = match Self::selected(&self.logging, cx) {
            "Inherit" => None,
            kind => Some(LoggingOptions {
                auto_start: kind == "Automatic",
                directory: Some(self.directory.read(cx).value().trim().to_owned())
                    .filter(|text| !text.is_empty())
                    .map(Into::into),
                max_file_mib: self
                    .limit
                    .read(cx)
                    .value()
                    .trim()
                    .parse()
                    .map_err(|_| "Log size must be from 1 to 1024 MiB.".to_owned())?,
            }),
        };
        let options = SessionOptions {
            term,
            charset,
            proxy,
            logging,
        };
        options.validate()?;
        Ok(options)
    }
    pub fn reset(&mut self, options: SessionOptions, window: &mut Window, cx: &mut Context<Self>) {
        let was_focused = [&self.term, &self.charset, &self.proxy, &self.logging]
            .iter()
            .any(|choice| {
                choice
                    .read(cx)
                    .focus_handle(cx)
                    .contains_focused(window, cx)
            })
            || [
                &self.custom,
                &self.host,
                &self.port,
                &self.directory,
                &self.limit,
            ]
            .iter()
            .any(|input| input.read(cx).focus_handle(cx).contains_focused(window, cx));
        *self = Self::new(options, self.inherit, window, cx);
        if was_focused {
            window.focus(&self.term.read(cx).focus_handle(cx), cx);
        }
        cx.notify();
    }
    fn field(label: &'static str, input: &Entity<InputState>) -> impl IntoElement {
        v_flex()
            .gap_1()
            .child(div().text_sm().child(label))
            .child(Input::new(input))
    }
}

impl gpui_kit::Focusable for SessionOptionsEditor {
    fn focus_handle(&self, cx: &App) -> gpui_kit::FocusHandle {
        self.term.read(cx).focus_handle(cx)
    }
}
impl Render for SessionOptionsEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let proxy = Self::selected(&self.proxy, cx);
        let logging = Self::selected(&self.logging, cx);
        v_flex().gap_3().w_full()
            .when(self.inherit,|form|form.child(div().text_xs().text_color(cx.theme().muted_foreground).child("Inherit uses the current global setting on the next connection.")))
            .child(div().text_sm().child("Terminal type (TERM)"))
            .child(Select::new(&self.term).id("session-term").w_full())
            .when(Self::selected(&self.term,cx)=="Custom",|form|form.child(Self::field("Custom TERM",&self.custom)))
            .child(div().text_sm().child("Text charset"))
            .child(Select::new(&self.charset).id("session-charset").w_full())
            .child(div().text_sm().child("Proxy"))
            .child(Select::new(&self.proxy).id("session-proxy").w_full())
            .when(matches!(proxy,"HTTP CONNECT"|"SOCKS5"),|form|form.child(Self::field("Proxy host",&self.host)).child(Self::field("Proxy port",&self.port))
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Proxy authentication is not supported yet."))
                .when(proxy=="SOCKS5",|form|form.child(gpui_kit::component::button::Button::new("proxy-dns").label(if self.remote_dns {"DNS through proxy"}else{"DNS on this computer"}).on_click(cx.listener(|this,_,_,cx|{this.remote_dns = !this.remote_dns;cx.notify();})))))
            .child(div().text_sm().child("Session output logging"))
            .child(Select::new(&self.logging).id("session-logging").w_full())
            .when(logging!="Inherit",|form|form.child(Self::field("Log directory (empty: default)",&self.directory)).child(Self::field("Maximum log size (MiB)",&self.limit)).child(div().text_xs().text_color(cx.theme().muted_foreground).child("Logs contain received terminal output, including escape sequences and any sensitive text the remote program prints. Authentication prompts and sent input are excluded.")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{TestAppContext, WindowOptions, test::TestWindowExt as _};
    #[gpui_kit::test]
    fn native_term_choice_and_custom_validation_preserve_inheritance(cx: &mut TestAppContext) {
        let (handle, editor) = cx.update(|cx| {
            gpui_kit::init(cx);
            crate::init(
                crate::DesignTokens::builtin(),
                crate::SettingsStore::in_memory(Default::default()),
                cx,
            );
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| SessionOptionsEditor::new(Default::default(), true, window, cx))
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                editor.read(cx).options(cx).unwrap(),
                SessionOptions::default()
            );
            window.within("session-term").click("input", cx);
            window.press("down", cx);
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert_eq!(
                editor.read(cx).options(cx).unwrap().term.as_deref(),
                Some("xterm-256color")
            );
            editor.update(cx, |editor, cx| {
                editor.term.update(cx, |select, cx| {
                    select.set_selected_value(&"Custom", window, cx)
                });
                editor
                    .custom
                    .update(cx, |input, cx| input.set_value("invalid TERM", window, cx));
                assert!(editor.options(cx).is_err());
                editor
                    .custom
                    .update(cx, |input, cx| input.set_value("ansi", window, cx));
                assert_eq!(editor.options(cx).unwrap().term.as_deref(), Some("ansi"));
                editor.charset.update(cx, |select, cx| {
                    select.set_selected_value(&"Windows-1251", window, cx)
                });
                assert_eq!(
                    editor.options(cx).unwrap().charset,
                    Some(Charset::Windows1251)
                );
                editor.reset(Default::default(), window, cx);
                assert_eq!(editor.options(cx).unwrap(), SessionOptions::default());
            });
        })
        .unwrap();
    }
}
