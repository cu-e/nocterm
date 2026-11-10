# 0163. Windows agents start without a console window and with the system's search path

Status: accepted.

## Context

Local agents such as Codex start through `npx`. On Windows two things went wrong:

- The agent environment allow-list compared names case-sensitively. Windows spells the
  search path `Path`, so it was dropped and `npx` was never found unless the user typed
  its absolute path.
- The release build is a GUI application without a console. Every console program it
  started (`npx.cmd`, `node`, `taskkill`) opened its own console window.

## Decision

- On Windows the allow-list and the explicitly inherited names match without case, and every
  name is stored in upper case, so a settings override replaces the inherited value rather
  than adding a second spelling. The Windows system locations Node.js, npm and npx read
  (`SystemDrive`, `ProgramFiles`, `ProgramData`, `USERNAME`, `NVM_SYMLINK`, `VOLTA_HOME` and
  similar) are allowed; none carries a credential.
- When a bare command is not on `PATH`, the usual Node.js install locations are searched:
  nvm-windows, Volta, `Program Files\nodejs`, the per-user Node.js install, npm's global
  directory, Scoop and Chocolatey on Windows; Homebrew and Volta on macOS. A command found
  there has its directory put first on the agent's `PATH`, so `npx.cmd` finds `node`.
- A resolved program keeps the ordinary `C:\` form instead of a verbatim `\\?\` path, which
  `cmd.exe` cannot run.
- The agent process and the `taskkill` that stops its tree start with `CREATE_NO_WINDOW`.
  The agent gets a hidden console, which its own tools inherit, so they open no windows
  either.

## Consequences

- A default Codex or Claude launch works on Windows without an absolute path when Node.js is
  installed in one of the usual places.
- No console window appears when a chat starts or stops an agent.
- Interactive sign-in still runs in Nocterm's own terminal tab.
