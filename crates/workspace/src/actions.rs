gpui_kit::actions!(
    workspace,
    [
        /// Open the menu of connections to start a new tab from.
        NewTab,
        /// Close the active tab.
        CloseTab,
        /// Close other tabs in the active pane.
        CloseOtherTabs,
        /// Close tabs to the left of the active tab in its pane.
        CloseTabsLeft,
        /// Close tabs to the right of the active tab in its pane.
        CloseTabsRight,
        /// Close all central tabs, retaining the local terminal.
        CloseAllTabs,
        /// Switch to the tab on the right, wrapping around.
        NextTab,
        /// Switch to the tab on the left, wrapping around.
        PreviousTab,
        /// Show or hide the sidebar.
        ToggleSidebar,
        /// Show or hide the AI panel.
        ToggleRightPanel,
        /// Expand or restore the AI panel in the workspace body.
        ToggleRightPanelMaximized,
        /// Open AI settings.
        OpenAiSettings,
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
        /// Copy the current terminal selection.
        EditCopy,
        /// Paste the clipboard into the focused terminal.
        EditPaste,
        /// Select all terminal content.
        SelectAll,
        /// Clear the terminal selection.
        ClearSelection,
        /// Open terminal search.
        Find,
        /// Select the next search match.
        FindNext,
        /// Select the previous search match.
        FindPrevious,
        /// Search for the current terminal selection.
        FindNextSelection,
        /// Disconnect the focused session.
        DisconnectSession,
        /// Reconnect the focused session.
        ReconnectSession,
        /// Edit options of the focused session.
        SessionSettings,
        /// Start recording terminal output.
        StartRecording,
        /// Stop recording terminal output.
        StopRecording,
        /// Copy the displayed connection name or alias.
        CopyConnectionName,
        /// Open another application window.
        NewWindow,
        /// Close this window.
        CloseWindow,
        /// Show application information.
        About,
        /// Open the directory containing connection profiles.
        ProfilesDirectory,
        /// Open the terminal recordings directory.
        LogsDirectory,
        /// Edit default session options.
        DefaultSessionSettings,
        /// Open SSH settings.
        OpenSSHSettings,
        /// Quit the application.
        Quit,
    ]
);
