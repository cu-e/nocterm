//! Early CLI dispatch; this path must run before GUI setup.
pub(crate) fn run_from_environment() -> Result<(), String> {
    let endpoint = std::env::var("NOCTERM_BRIDGE_ENDPOINT")
        .map_err(|_| "Missing bridge endpoint".to_owned())?;
    let token =
        std::env::var("NOCTERM_BRIDGE_TOKEN").map_err(|_| "Missing bridge token".to_owned())?;
    nocterm_acp::run_relay(std::io::stdin(), std::io::stdout(), &endpoint, &token)
        .map_err(|e| e.to_string())
}
