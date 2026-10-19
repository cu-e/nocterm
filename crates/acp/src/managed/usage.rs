//! Provider account metadata is read from retained spans without a DTO/cache copy.
use super::{error::DecodeError, lists, value::Value};
use nocterm_ai::{
    model::{
        json::deferred::{JsonDecodeError, literal::LiteralBuilder},
        native::{NativeList, NativeOwned},
        notice::UsageNotice,
        usage::Cost,
    },
    usage::RateLimit,
};
type Error = DecodeError<JsonDecodeError>;
pub(super) fn update(value: Value<'_>) -> Result<NativeOwned<UsageNotice>, Error> {
    let [used, size, cost, meta] = value.fields(["used", "size", "cost", "_meta"])?;
    let used = Value::required(used, "usage.used")?
        .raw()
        .parse::<u64>()
        .map_err(|_| DecodeError::Field("usage.used"))?;
    let size = Value::required(size, "usage.size")?
        .raw()
        .parse::<u64>()
        .map_err(|_| DecodeError::Field("usage.size"))?;
    let cost = match cost {
        Some(cost) => match lists::shape(cost_shape(cost))? {
            Some((amount, currency)) => NativeOwned::supplied(
                nocterm_ai::model::native::usage::cost(amount, currency.text()?),
            ),
            None => NativeOwned::<Cost>::absent(value.ledger())?,
        },
        None => NativeOwned::<Cost>::absent(value.ledger())?,
    };
    let limit = limit(meta, &value)?;
    let mut limits = NativeList::new(value.ledger(), usize::from(limit.is_some()))?;
    if let Some(limit) = limit {
        limits.push(limit)?;
    }
    Ok(nocterm_ai::model::native::notice::usage_notice(
        nocterm_ai::model::native::usage::context(used, size, cost),
        limits.finish(),
    )?)
}
fn cost_shape(value: Value<'_>) -> Result<(f64, Value<'_>), Error> {
    let [amount, currency, _meta] = value.fields(["amount", "currency", "_meta"])?;
    let amount = Value::required(amount, "cost.amount")?
        .raw()
        .parse::<f64>()
        .map_err(|_| DecodeError::Field("cost.amount"))?;
    let currency = Value::required(currency, "cost.currency")?;
    if !amount.is_finite() || !currency.raw().starts_with('"') {
        return Err(DecodeError::Field("cost"));
    }
    Ok((amount, currency))
}
fn constant(value: &Value<'_>, text: &str) -> Result<NativeOwned<String>, Error> {
    let mut builder = LiteralBuilder::new(value.ledger())?;
    let mut native = builder.exact_text(text.len())?;
    native.append(text)?;
    builder.string(native.finish()?)?;
    Ok(NativeOwned::text(builder.finish_native()?.into_text()?))
}
fn limit(
    meta: Option<Value<'_>>,
    root: &Value<'_>,
) -> Result<Option<NativeOwned<RateLimit>>, Error> {
    let Some(meta) = meta.filter(|value| value.raw().starts_with('{')) else {
        return Ok(None);
    };
    let [info] = meta.fields(["_claude/rateLimit"])?;
    let Some(info) = info.filter(|value| value.raw().starts_with('{')) else {
        return Ok(None);
    };
    let [kind, used, resets, status] =
        info.fields(["rateLimitType", "utilization", "resetsAt", "status"])?;
    let Some(kind) = kind else {
        return Ok(None);
    };
    let window = if kind.equals("five_hour")? {
        nocterm_ai::model::native::usage::window(300, root.ledger())?
    } else if kind.equals("seven_day")? {
        nocterm_ai::model::native::usage::window(10080, root.ledger())?
    } else if kind.equals("seven_day_opus")? {
        nocterm_ai::model::native::usage::weekly_model(constant(root, "Opus")?)
    } else if kind.equals("seven_day_sonnet")? {
        nocterm_ai::model::native::usage::weekly_model(constant(root, "Sonnet")?)
    } else if kind.equals("overage")? || kind.equals("seven_day_overage_included")? {
        nocterm_ai::model::native::usage::extra(root.ledger())?
    } else {
        return Ok(None);
    };
    let used = used
        .and_then(|value| value.raw().parse::<f64>().ok())
        .filter(|value| value.is_finite())
        .map(|value| value.clamp(0., 1.));
    let resets = resets.and_then(|value| value.raw().parse::<u64>().ok());
    let reached = match status {
        Some(status) => status.equals("rejected")?,
        None => false,
    };
    Ok(Some(nocterm_ai::model::native::usage::rate_limit(
        window, used, resets, reached,
    )))
}
