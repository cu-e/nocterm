//! What the panel offers to do with a container, a Compose project or an
//! image, decided once for both the row's buttons and its menu.

use nocterm_containers::{Action, Container, Group, State};
use nocterm_ui::IconName;

/// One thing to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    /// Follow the container's log in a tab.
    Logs,
    /// Enter the container with a shell in a tab.
    Shell,
    Act(Action),
}

impl Op {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Logs => "View Logs",
            Self::Shell => "Attach Shell",
            Self::Act(action) => action.label(),
        }
    }

    pub(crate) fn icon(self) -> IconName {
        match self {
            Self::Logs => IconName::ScrollText,
            Self::Shell => IconName::SquareTerminal,
            Self::Act(Action::Start | Action::Unpause) => IconName::Play,
            Self::Act(Action::Pause) => IconName::Pause,
            Self::Act(Action::Stop) => IconName::Square,
            Self::Act(Action::Restart) => IconName::RotateCw,
            Self::Act(Action::Remove | Action::RemoveImage) => IconName::Trash,
        }
    }

    /// Whether the row shows it as a button while hovered; the rest are in
    /// its menu only.
    pub(crate) fn inline(self) -> bool {
        !matches!(self, Self::Act(Action::Pause | Action::Restart))
    }

    /// Whether it destroys something, and is asked about first.
    pub(crate) fn destructive(self) -> bool {
        matches!(self, Self::Act(Action::Remove | Action::RemoveImage))
    }

    /// A name for element ids.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Logs => "logs",
            Self::Shell => "shell",
            Self::Act(Action::Start) => "start",
            Self::Act(Action::Stop) => "stop",
            Self::Act(Action::Restart) => "restart",
            Self::Act(Action::Pause) => "pause",
            Self::Act(Action::Unpause) => "unpause",
            Self::Act(Action::Remove) => "remove",
            Self::Act(Action::RemoveImage) => "remove-image",
        }
    }
}

/// What a container in its state allows, in menu order.
pub(crate) fn for_container(container: &Container) -> Vec<Op> {
    let state = &container.state;
    let mut ops = vec![Op::Logs];
    if *state == State::Running {
        ops.push(Op::Shell);
    }
    if !state.is_live() {
        ops.push(Op::Act(Action::Start));
    }
    match state {
        State::Paused => ops.push(Op::Act(Action::Unpause)),
        State::Running => ops.push(Op::Act(Action::Pause)),
        _ => {}
    }
    if state.is_live() {
        ops.extend([Op::Act(Action::Stop), Op::Act(Action::Restart)]);
    }
    ops.push(Op::Act(Action::Remove));
    ops
}

/// What a Compose project allows, done to each of its containers.
pub(crate) fn for_project(group: &Group<'_>) -> Vec<Op> {
    let live = group
        .containers
        .iter()
        .filter(|c| c.state.is_live())
        .count();
    let mut ops = Vec::new();
    if live < group.containers.len() {
        ops.push(Op::Act(Action::Start));
    }
    if live > 0 {
        ops.extend([Op::Act(Action::Stop), Op::Act(Action::Restart)]);
    }
    ops.push(Op::Act(Action::Remove));
    ops
}

/// What an image allows.
pub(crate) const FOR_IMAGE: [Op; 1] = [Op::Act(Action::RemoveImage)];

#[cfg(test)]
mod tests {
    use super::*;

    fn container(state: State) -> Container {
        Container {
            id: "id".into(),
            name: "web".into(),
            image: "nginx".into(),
            state,
            status: String::new(),
            project: None,
            service: None,
        }
    }

    #[test]
    fn a_container_offers_what_its_state_allows() {
        use Action::*;
        assert_eq!(
            for_container(&container(State::Running)),
            [
                Op::Logs,
                Op::Shell,
                Op::Act(Pause),
                Op::Act(Stop),
                Op::Act(Restart),
                Op::Act(Remove)
            ]
        );
        assert_eq!(
            for_container(&container(State::Paused)),
            [
                Op::Logs,
                Op::Act(Unpause),
                Op::Act(Stop),
                Op::Act(Restart),
                Op::Act(Remove)
            ]
        );
        assert_eq!(
            for_container(&container(State::Exited)),
            [Op::Logs, Op::Act(Start), Op::Act(Remove)]
        );
    }

    #[test]
    fn a_project_offers_starting_what_is_stopped_and_stopping_what_runs() {
        let running = container(State::Running);
        let exited = container(State::Exited);
        let group = |containers| Group {
            project: Some("shop"),
            containers,
        };
        let start = Op::Act(Action::Start);
        let stop = Op::Act(Action::Stop);
        let all_up = for_project(&group(vec![&running]));
        assert!(!all_up.contains(&start) && all_up.contains(&stop));
        let all_down = for_project(&group(vec![&exited]));
        assert!(all_down.contains(&start) && !all_down.contains(&stop));
        let mixed = for_project(&group(vec![&running, &exited]));
        assert!(mixed.contains(&start) && mixed.contains(&stop));
    }

    #[test]
    fn only_what_destroys_is_asked_about() {
        let destructive: Vec<_> = for_container(&container(State::Running))
            .into_iter()
            .chain(FOR_IMAGE)
            .filter(|op| op.destructive())
            .collect();
        assert_eq!(
            destructive,
            [Op::Act(Action::Remove), Op::Act(Action::RemoveImage)]
        );
    }
}
