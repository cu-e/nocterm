//! Explicit requests for external local programs, without process ownership here.
use std::{
    io,
    path::{Path, PathBuf},
};

/// A native program and literal arguments; no shell parsing or interpolation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramOpenRequest {
    program: String,
    args: Vec<String>,
    cwd: Option<PathBuf>,
}

impl ProgramOpenRequest {
    pub fn new(
        program: String,
        args: Vec<String>,
        cwd: Option<PathBuf>,
    ) -> Result<Self, ProgramOpenError> {
        if program.is_empty() || program.contains('\0') {
            return Err(ProgramOpenError::InvalidProgram);
        }
        if let Some(index) = args.iter().position(|arg| arg.contains('\0')) {
            return Err(ProgramOpenError::InvalidArgument { index });
        }
        Ok(Self { program, args, cwd })
    }
    pub fn program(&self) -> &str {
        &self.program
    }
    pub fn args(&self) -> &[String] {
        &self.args
    }
    pub fn cwd(&self) -> Option<&Path> {
        self.cwd.as_deref()
    }
    pub fn into_parts(self) -> (String, Vec<String>, Option<PathBuf>) {
        (self.program, self.args, self.cwd)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProgramOpenError {
    #[error("Choose a nonempty program without NUL characters.")]
    InvalidProgram,
    #[error("Program argument {index} contains a NUL character.")]
    InvalidArgument { index: usize },
    #[error("There are already {limit} external programs awaiting exit.")]
    Busy { limit: usize },
    #[error("Could not start the program ownership worker: {source}")]
    Worker {
        #[source]
        source: io::Error,
    },
    #[error("Could not start {program}: {source}")]
    Start {
        program: String,
        #[source]
        source: io::Error,
    },
    #[error("The program launch acknowledgement was interrupted.")]
    CompletionDropped,
}

pub type ProgramOpenFuture = futures::future::BoxFuture<'static, Result<(), ProgramOpenError>>;

/// Success acknowledges physical spawn. Accepted children outlive the receipt.
/// Constructing an unpolled future must launch nothing.
pub trait ProgramOpener: Send + Sync + 'static {
    fn open(&self, request: ProgramOpenRequest) -> ProgramOpenFuture;
}

#[cfg(test)]
mod tests;
