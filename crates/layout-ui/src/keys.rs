//! The keys of the editor, apart from what they do.

use gpui_kit::Keystroke;

/// What a key asks of the selected item or of the history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeyCommand {
    Undo,
    Redo,
    /// One place earlier (-1) or later (+1) in its list.
    Move(i32),
    /// One zone toward the left (-1) or the right (+1).
    Zone(i32),
    /// To the next panel group.
    NextGroup,
    /// A separator after the item.
    Separator,
    /// Out of the bar, into the tray.
    Remove,
}

pub(crate) fn command(stroke: &Keystroke) -> Option<KeyCommand> {
    let modifiers = &stroke.modifiers;
    let key = stroke.key.as_str();
    if (modifiers.control || modifiers.platform) && !modifiers.alt {
        return match (key, modifiers.shift) {
            ("z", false) => Some(KeyCommand::Undo),
            ("z", true) | ("y", false) => Some(KeyCommand::Redo),
            _ => None,
        };
    }
    if modifiers.alt && !modifiers.control && !modifiers.platform && !modifiers.shift {
        return match key {
            "left" => Some(KeyCommand::Move(-1)),
            "right" => Some(KeyCommand::Move(1)),
            "up" => Some(KeyCommand::Zone(-1)),
            "down" => Some(KeyCommand::Zone(1)),
            _ => None,
        };
    }
    if modifiers.control || modifiers.platform || modifiers.alt {
        return None;
    }
    let typed = stroke.key_char.as_deref().unwrap_or(key);
    match typed {
        "g" | "G" => Some(KeyCommand::NextGroup),
        "|" => Some(KeyCommand::Separator),
        "delete" => Some(KeyCommand::Remove),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stroke(text: &str) -> KeyCommand {
        command(&Keystroke::parse(text).unwrap()).unwrap()
    }

    #[test]
    fn the_keys_name_their_commands() {
        assert_eq!(stroke("alt-left"), KeyCommand::Move(-1));
        assert_eq!(stroke("alt-right"), KeyCommand::Move(1));
        assert_eq!(stroke("alt-up"), KeyCommand::Zone(-1));
        assert_eq!(stroke("alt-down"), KeyCommand::Zone(1));
        assert_eq!(stroke("g"), KeyCommand::NextGroup);
        assert_eq!(stroke("delete"), KeyCommand::Remove);
        assert_eq!(command(&Keystroke::parse("backspace").unwrap()), None);
        assert_eq!(stroke("ctrl-z"), KeyCommand::Undo);
        assert_eq!(stroke("cmd-z"), KeyCommand::Undo);
        assert_eq!(stroke("ctrl-shift-z"), KeyCommand::Redo);
        assert!(command(&Keystroke::parse("ctrl-g").unwrap()).is_none());
    }
}
