//! Passive diagnostics only: this never initiates an authentication prompt.
use std::io::Write as _;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    match nocterm_device_unlock::provider().probe() {
        Ok(capability) => writeln!(
            std::io::stdout().lock(),
            "{}: {:?}\n{}",
            capability.label,
            capability.availability,
            capability.detail
        )?,
        Err(error) => {
            return Err(error.into());
        }
    }
    Ok(())
}
