//! Ubunye additions to the UI (new files only; upstream files call in with one line each).
//!
//! - [`kopano`]: the client for `kopanod`, the Kopano service on this computer (co-editing).
//! - [`kopano_panel`]: the Kopano window (Window ▸ Kopano) and its commands.

pub mod kopano;
pub mod kopano_panel;

#[cfg(test)]
mod tests_kopano;
