//! Domain types and rules for Braydr. See `docs/product-spec.md`.

pub mod error;
pub mod launch;
pub mod model;
pub mod notification;
pub mod notify;
pub mod read;
pub mod schedule;
pub mod text;
pub mod validate;

pub use error::{Error, Result};
pub use model::*;
