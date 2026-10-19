//! Typed SDK mappings. Only claim projection is used at
//! runtime; the remaining mappings are original oracles for managed tests.
#[cfg(test)]
pub(crate) mod auth;
#[cfg(test)]
mod content;
#[cfg(test)]
pub(crate) mod controls;
pub(crate) mod identity;
#[cfg(test)]
pub(crate) mod notice;
#[cfg(test)]
pub(crate) mod outbound;
#[cfg(test)]
pub(crate) mod session;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tools;
#[cfg(test)]
mod usage;
