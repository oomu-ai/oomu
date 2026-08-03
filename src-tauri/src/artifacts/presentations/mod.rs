mod agent_tool;
mod checker_setup;
mod commands;
mod contract;
mod exact_package_preview;
mod export;
mod fixture;
mod ir;
mod layout_xml;
mod native_preview;
mod ooxml;
mod package_metadata;
mod policy;
mod provenance;
mod repository;
mod review;
mod revision;
mod schema;
mod service;
mod template;
mod verification;
mod xml;
pub(crate) mod zip;

pub(crate) use agent_tool::register_task_tool as register_presentation_task_tool;
pub use commands::*;
pub use contract::*;
pub use export::*;
pub use fixture::*;
pub use ir::*;
pub use ooxml::*;
pub use policy::*;
pub use provenance::*;
pub use repository::*;
pub use review::*;
pub use revision::*;
pub use template::*;
pub use verification::*;

pub const PRESENTATION_SCHEMA_SQL: &str = schema::PRESENTATION_SCHEMA_SQL;

#[cfg(test)]
mod tests;
