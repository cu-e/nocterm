//! Full neutral event copies preserve every original event field and ordering.
use super::{NativeCopies, NativeList, NativeOwned};
use crate::{
    model::{
        auth::ProviderPermission, controls::Change, json::deferred::JsonDecodeError, notice::*,
    },
    usage::Window,
};
pub(super) struct NoticeCopies<'a, 'l> {
    copies: &'a NativeCopies<'l>,
}
impl<'a, 'l> NoticeCopies<'a, 'l> {
    pub(super) fn new(copies: &'a NativeCopies<'l>) -> Self {
        Self { copies }
    }
    pub(super) fn notice(
        &self,
        value: &SessionNotice,
    ) -> Result<NativeOwned<SessionNotice>, JsonDecodeError> {
        let c = self.copies;
        let update = match &value.update {
            SessionUpdate::UserChunk(value) => {
                crate::model::native::notice::user_chunk(c.content(value)?)
            }
            SessionUpdate::AgentChunk(value) => {
                crate::model::native::notice::agent_chunk(c.content(value)?)
            }
            SessionUpdate::ThoughtChunk(value) => {
                crate::model::native::notice::thought_chunk(c.content(value)?)
            }
            SessionUpdate::ToolStarted(value) => {
                crate::model::native::notice::tool_started(c.tool(value)?)
            }
            SessionUpdate::ToolChanged(value) => {
                crate::model::native::notice::tool_changed(c.patch(value)?)
            }
            SessionUpdate::Plan(values) => crate::model::native::notice::plan(self.plan(values)?),
            SessionUpdate::Commands(values) => {
                crate::model::native::notice::commands(self.commands(values)?)
            }
            SessionUpdate::Mode(value) => crate::model::native::notice::mode(
                crate::model::native::controls::mode_id(c.text(value.as_str())?)?,
            ),
            SessionUpdate::Configuration(values) => {
                crate::model::native::notice::configuration(c.configurations(values, None)?)
            }
            SessionUpdate::Info(value) => {
                let presence = match value {
                    Change::Keep => NativeOwned::<String>::not_supplied(c.ledger)?,
                    Change::Clear => NativeOwned::<String>::null(c.ledger)?,
                    Change::Set(value) => c.text(value)?.presence(),
                };
                crate::model::native::notice::info(crate::model::native::notice::from_presence(
                    presence,
                ))
            }
            SessionUpdate::Usage(value) => crate::model::native::notice::usage(self.usage(value)?),
        };
        crate::model::native::notice::notice(
            crate::model::native::controls::session_id(c.text(value.session.as_str())?)?,
            update,
        )
    }
    pub(super) fn permission(
        &self,
        value: &ProviderPermission,
    ) -> Result<NativeOwned<ProviderPermission>, JsonDecodeError> {
        let c = self.copies;
        let mut choices = NativeList::new(c.ledger, value.choices.len())?;
        for value in &value.choices {
            choices.push(crate::model::native::auth::permission_choice(
                crate::model::native::auth::permission_option_id(c.text(value.id.as_str())?)?,
                c.text(&value.label)?,
                value.effect,
                c.extensions(value.extensions.as_ref())?,
            )?)?;
        }
        crate::model::native::auth::permission(
            crate::model::native::controls::session_id(c.text(value.session.as_str())?)?,
            c.patch(&value.tool)?,
            choices.finish(),
            c.extensions(value.extensions.as_ref())?,
        )
    }
    fn plan(&self, values: &[PlanStep]) -> Result<NativeOwned<Vec<PlanStep>>, JsonDecodeError> {
        let mut output = NativeList::new(self.copies.ledger, values.len())?;
        for value in values {
            output.push(crate::model::native::notice::plan_step(
                self.copies.text(&value.content)?,
                value.priority,
                value.status,
            ))?;
        }
        Ok(output.finish())
    }
    fn commands(
        &self,
        values: &[AdvertisedCommand],
    ) -> Result<NativeOwned<Vec<AdvertisedCommand>>, JsonDecodeError> {
        let c = self.copies;
        let mut output = NativeList::new(c.ledger, values.len())?;
        for value in values {
            output.push(crate::model::native::notice::command(
                c.text(&value.name)?,
                c.text(&value.description)?,
                c.optional_text(value.input_hint.as_deref())?,
            )?)?;
        }
        Ok(output.finish())
    }
    fn usage(&self, value: &UsageNotice) -> Result<NativeOwned<UsageNotice>, JsonDecodeError> {
        let c = self.copies;
        let cost = match &value.context.cost {
            Some(value) => NativeOwned::supplied(crate::model::native::usage::cost(
                value.amount,
                c.text(&value.currency)?,
            )),
            None => NativeOwned::absent(c.ledger)?,
        };
        let context =
            crate::model::native::usage::context(value.context.used, value.context.size, cost);
        let mut limits = NativeList::new(c.ledger, value.rate_limits.len())?;
        for limit in &value.rate_limits {
            let window = match &limit.window {
                Window::FiveHour => crate::model::native::usage::window(300, c.ledger)?,
                Window::Weekly => crate::model::native::usage::window(10080, c.ledger)?,
                Window::Minutes(minutes) => {
                    NativeOwned::from_parts(Window::Minutes(*minutes), c.ledger.reserve(0)?)
                }
                Window::WeeklyModel(value) => {
                    crate::model::native::usage::weekly_model(c.text(value)?)
                }
                Window::Extra => crate::model::native::usage::extra(c.ledger)?,
            };
            limits.push(crate::model::native::usage::rate_limit(
                window,
                limit.used,
                limit.resets_at,
                limit.reached,
            ))?;
        }
        crate::model::native::notice::usage_notice(context, limits.finish())
    }
}
