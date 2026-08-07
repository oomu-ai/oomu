mod address;
pub(crate) mod agent_tool;
mod command_io;
mod commands;
mod contract;
mod evidence;
mod exact_package_preview;
mod fixture;
mod ir;
mod native_preview;
mod ooxml;
mod policy;
mod preview;
mod provenance;
mod recalc;
mod repository;
mod review;
mod review_events;
mod review_format;
mod revision;
mod sheet_print_xml;
mod sheet_xml;
mod style_xml;
mod template;
mod template_inspection;
mod validation;
mod validation_budget;
mod validation_primitives;
mod verification;
pub(crate) use crate::foundation::office_zip as zip;

pub use commands::*;
pub(crate) use contract::artifact_workbook_contract;
pub use evidence::*;
pub use fixture::{deterministic_fixture, write_deterministic_fixture};
pub use ir::*;
pub use ooxml::{build_workbook, WorkbookBuildOutput};
pub(crate) use provenance::{
    bind_workbook_provenance, resolve_workbook_evidence, BoundWorkbookEvidence,
};
pub use recalc::recalculate_supported_formulas;
pub use review::*;
pub use revision::{
    revise_imported_xlsx, revise_range, ImportedPackageRevision, WorkbookRangeRevision,
    WorkbookRevisionError, WorkbookRevisionErrorCode,
};
pub use template::*;
pub use validation::{validate_workbook, ValidatedWorkbook};
pub use verification::{verify_workbook_bytes, WorkbookVerification};

pub const WORKBOOK_IR_SCHEMA_VERSION: u16 = 1;
pub const WORKBOOK_BUILDER_IDENTITY: &str = "oomu-workbook-builder/1.0.0+ooxml-store-v1";
pub const WORKBOOK_RECALC_ENGINE: &str = "oomu-bounded-formula-engine";
pub const WORKBOOK_RECALC_ENGINE_VERSION: &str = "1.0.0";
