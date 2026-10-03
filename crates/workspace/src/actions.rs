gpui_kit::actions!(
    workspace,
    [
        /// Open the menu of connections to start a new tab from.
        NewTab,
        /// Close the active tab.
        CloseTab,
        /// Switch to the tab on the right, wrapping around.
        NextTab,
        /// Switch to the tab on the left, wrapping around.
        PreviousTab,
        /// Show or hide the sidebar.
        ToggleSidebar,
        /// Show the next sidebar panel, wrapping around.
        NextPanel,
        /// Open the settings.
        OpenSettings,
        /// Move the active tab one position to the left in its pane.
        MoveTabLeft,
        /// Move the active tab one position to the right in its pane.
        MoveTabRight,
        /// Move the active tab into a new pane on the left.
        SplitLeft,
        /// Move the active tab into a new pane on the right.
        SplitRight,
        /// Move the active tab into a new pane above.
        SplitUp,
        /// Move the active tab into a new pane below.
        SplitDown,
        /// Focus the next pane in layout order.
        NextPane,
        /// Focus the preceding pane in layout order.
        PreviousPane,
        /// Edit the active tab's display alias.
        RenameTab,
        /// Show or hide the local terminal without ending its process.
        ToggleLocalTerminal,
        /// Close the bottom local terminal and end its process.
        CloseLocalTerminal,
        /// Open the encrypted credential vault.
        OpenVault,
        /// Quit the application.
        Quit,
    ]
);
