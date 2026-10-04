//! Normalize older ACP model selectors at the transport boundary.
use nocterm_ai::{AgentError, acp};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(crate) struct Models(HashMap<String, LegacyModels>);
struct LegacyModels {
    id: String,
    options: Vec<acp::SessionConfigOption>,
}

impl Models {
    pub(crate) fn session(&mut self, raw: Value) -> Result<acp::NewSessionResponse, AgentError> {
        let mut response: acp::NewSessionResponse = serde_json::from_value(raw.clone())
            .map_err(|_| AgentError::Io("Invalid ACP session response".into()))?;
        if response.session_id.0.is_empty() {
            return Err(AgentError::Io("Invalid ACP session ID".into()));
        }
        self.0.remove(response.session_id.0.as_ref());
        let mut options = response.config_options.clone().unwrap_or_default();
        if options
            .iter()
            .any(|option| option.category == Some(acp::SessionConfigOptionCategory::Model))
        {
            return Ok(response);
        }
        let Some(models) = raw.get("models") else {
            return Ok(response);
        };
        let Some(current) = models.get("currentModelId").and_then(Value::as_str) else {
            return Ok(response);
        };
        let Some(values) = models.get("availableModels").and_then(Value::as_array) else {
            return Ok(response);
        };
        if values.len() > 512 {
            return Ok(response);
        }
        let mut seen = HashSet::new();
        let values = values
            .iter()
            .filter_map(|value| {
                let id = value.get("modelId")?.as_str()?;
                let name = value.get("name")?.as_str()?;
                if id.is_empty()
                    || id.len() > 1024
                    || name.trim().is_empty()
                    || name.len() > 1024
                    || !seen.insert(id)
                {
                    return None;
                }
                Some(acp::SessionConfigSelectOption::new(
                    id.to_owned(),
                    name.to_owned(),
                ))
            })
            .collect::<Vec<_>>();
        if !values.iter().any(|value| value.value.0.as_ref() == current) {
            return Ok(response);
        }
        let mut id = "__nocterm_legacy_model".to_owned();
        while options.iter().any(|option| option.id.0.as_ref() == id) {
            id.push('_');
        }
        options.push(
            acp::SessionConfigOption::select(id.clone(), "Model", current.to_owned(), values)
                .category(acp::SessionConfigOptionCategory::Model),
        );
        self.0.insert(
            response.session_id.to_string(),
            LegacyModels {
                id,
                options: options.clone(),
            },
        );
        response.config_options = Some(options);
        Ok(response)
    }
    pub(crate) fn selection(
        &self,
        request: &acp::SetSessionConfigOptionRequest,
    ) -> Result<Option<String>, AgentError> {
        let Some(models) = self.0.get(request.session_id.0.as_ref()) else {
            return Ok(None);
        };
        if models.id != request.config_id.0.as_ref() {
            return Ok(None);
        }
        let acp::SessionConfigOptionValue::ValueId { value } = &request.value else {
            return Err(AgentError::Io("Invalid model selection".into()));
        };
        let valid = models.options.iter().any(|option| match &option.kind {
            acp::SessionConfigKind::Select(select) if option.id.0.as_ref() == models.id => {
                match &select.options {
                    acp::SessionConfigSelectOptions::Ungrouped(values) => {
                        values.iter().any(|candidate| &candidate.value == value)
                    }
                    _ => false,
                }
            }
            _ => false,
        });
        if !valid {
            return Err(AgentError::Io("Unavailable model selection".into()));
        }
        Ok(Some(value.to_string()))
    }
    pub(crate) fn selected(
        &mut self,
        request: &acp::SetSessionConfigOptionRequest,
    ) -> Vec<acp::SessionConfigOption> {
        let Some(models) = self.0.get_mut(request.session_id.0.as_ref()) else {
            return Vec::new();
        };
        for option in &mut models.options {
            if option.id.0.as_ref() == models.id
                && let acp::SessionConfigKind::Select(select) = &mut option.kind
                && let acp::SessionConfigOptionValue::ValueId { value } = &request.value
            {
                select.current_value = value.clone();
            }
        }
        models.options.clone()
    }
    pub(crate) fn retain(
        &mut self,
        session: &acp::SessionId,
        mut options: Vec<acp::SessionConfigOption>,
    ) -> Vec<acp::SessionConfigOption> {
        if options
            .iter()
            .any(|option| option.category == Some(acp::SessionConfigOptionCategory::Model))
        {
            self.0.remove(session.0.as_ref());
        } else if let Some(models) = self.0.get_mut(session.0.as_ref()) {
            if let Some(model) = models
                .options
                .iter()
                .find(|option| option.id.0.as_ref() == models.id)
            {
                options.push(model.clone());
            }
            models.options = options.clone();
        }
        options
    }
    pub(crate) fn remove(&mut self, session: &acp::SessionId) {
        self.0.remove(session.0.as_ref());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn legacy() -> Value {
        json!({"sessionId":"session", "models":{"currentModelId":"provider:first", "availableModels":[{"modelId":"provider:first","name":"First"},{"modelId":"provider:second","name":"Second"}]}})
    }
    #[test]
    fn legacy_selector_preserves_ids_and_updates_only_on_success() {
        let mut models = Models::default();
        let response = models.session(legacy()).unwrap();
        let options = response.config_options.unwrap();
        assert_eq!(options.len(), 1);
        assert_eq!(
            options[0].category,
            Some(acp::SessionConfigOptionCategory::Model)
        );
        let request = acp::SetSessionConfigOptionRequest::new(
            "session",
            options[0].id.clone(),
            "provider:second",
        );
        assert_eq!(
            models.selection(&request).unwrap().as_deref(),
            Some("provider:second")
        );
        let before = models.retain(&response.session_id, Vec::new());
        let acp::SessionConfigKind::Select(select) = &before[0].kind else {
            panic!("select")
        };
        assert_eq!(select.current_value.0.as_ref(), "provider:first");
        let after = models.selected(&request);
        let acp::SessionConfigKind::Select(select) = &after[0].kind else {
            panic!("select")
        };
        assert_eq!(select.current_value.0.as_ref(), "provider:second");
        assert!(
            models
                .selection(&acp::SetSessionConfigOptionRequest::new(
                    "session",
                    options[0].id.clone(),
                    "unknown"
                ))
                .is_err()
        );
        models.remove(&response.session_id);
        assert!(models.selection(&request).unwrap().is_none());
    }
    #[test]
    fn native_model_wins_and_legacy_id_does_not_collide() {
        let mut raw = legacy();
        raw["configOptions"] = json!([{"id":"__nocterm_legacy_model","name":"Other","type":"select","currentValue":"a","options":[{"value":"a","name":"A"}]}]);
        let mut models = Models::default();
        let options = models.session(raw.clone()).unwrap().config_options.unwrap();
        assert_eq!(options[1].id.0.as_ref(), "__nocterm_legacy_model_");
        raw["configOptions"][0]["category"] = json!("model");
        assert_eq!(
            models.session(raw).unwrap().config_options.unwrap().len(),
            1
        );
    }
    #[test]
    fn invalid_legacy_metadata_adds_no_controls() {
        let mut models = Models::default();
        for invalid in [
            json!(null),
            json!({"currentModelId":"unknown","availableModels":[]}),
            json!({"currentModelId":"a","availableModels":[{"modelId":"a","name":""}]}),
        ] {
            let raw = json!({"sessionId":"session","models":invalid});
            assert!(
                models
                    .session(raw)
                    .unwrap()
                    .config_options
                    .unwrap_or_default()
                    .is_empty()
            );
        }
    }
}
