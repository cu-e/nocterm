/// Controls the optional presentation tail used to keep a hardware display awake.
#[derive(Clone, Copy)]
pub(crate) struct InputPresentationPolicy {
    windows_software_renderer: bool,
}

impl InputPresentationPolicy {
    pub(crate) fn new(is_windows: bool, software_emulated: Option<bool>) -> Self {
        Self {
            // Unknown adapters retain upstream behavior.
            windows_software_renderer: is_windows && software_emulated == Some(true),
        }
    }

    pub(crate) fn keep_presenting(self, high_rate_input: bool) -> bool {
        high_rate_input && !self.windows_software_renderer
    }

    pub(crate) fn needs_present(
        self,
        required: bool,
        explicit: bool,
        high_rate_input: impl FnOnce() -> bool,
    ) -> bool {
        required || explicit || (!self.windows_software_renderer && high_rate_input())
    }
}

#[cfg(test)]
mod tests {
    use super::InputPresentationPolicy;

    #[test]
    fn unchanged_software_scene_does_not_sustain_input_presentation() {
        let policy = InputPresentationPolicy::new(true, Some(true));
        assert!(!policy.keep_presenting(true));
        assert!(!policy.needs_present(false, false, || true));
    }

    #[test]
    fn hardware_unknown_and_other_platforms_keep_upstream_input_behavior() {
        for (is_windows, software_emulated) in [
            (true, Some(false)),
            (true, None),
            (false, Some(true)),
            (false, Some(false)),
            (false, None),
        ] {
            let policy = InputPresentationPolicy::new(is_windows, software_emulated);
            assert!(policy.keep_presenting(true));
            assert!(policy.needs_present(false, false, || true));
            assert!(!policy.keep_presenting(false));
            assert!(!policy.needs_present(false, false, || false));
        }
    }

    #[test]
    fn required_and_explicit_presentation_survive_software_policy() {
        for software_emulated in [Some(true), Some(false), None] {
            let policy = InputPresentationPolicy::new(true, software_emulated);
            for (required, explicit) in [(true, false), (false, true), (true, true)] {
                assert!(policy.needs_present(required, explicit, || {
                    panic!("explicit requests must not query the input tracker")
                }));
            }
        }
    }

    #[test]
    fn recovery_can_change_the_cached_renderer_classification() {
        let mut policy = InputPresentationPolicy::new(true, Some(false));
        assert!(policy.keep_presenting(true));
        policy = InputPresentationPolicy::new(true, Some(true));
        assert!(!policy.keep_presenting(true));
        policy = InputPresentationPolicy::new(true, None);
        assert!(policy.keep_presenting(true));
        policy = InputPresentationPolicy::new(true, Some(false));
        assert!(policy.keep_presenting(true));
    }
}
