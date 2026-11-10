//! The same bundled integration programs are used by ordinary and contained shells.
pub(super) fn bundled(stem: &str) -> Option<(&'static str, &'static str)> {
    Some(match stem {
        "bash" => (
            "bashrc",
            include_str!("../../../assets/shell-integration/bash.bash"),
        ),
        "zsh" => (
            ".zshrc",
            include_str!("../../../assets/shell-integration/zsh.zsh"),
        ),
        "fish" => (
            "integration.fish",
            include_str!("../../../assets/shell-integration/fish.fish"),
        ),
        "pwsh" | "powershell" => (
            "integration.ps1",
            include_str!("../../../assets/shell-integration/powershell.ps1"),
        ),
        _ => return None,
    })
}
