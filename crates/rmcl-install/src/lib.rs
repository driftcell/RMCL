//! Pure installation planning for Minecraft and its components.

mod coordinate;
mod error;
mod inherit;
mod plan;
mod vanilla;

pub use coordinate::{MavenCoordinate, MavenCoordinateError};
pub use error::InstallPlanError;
pub use inherit::resolve_version;
pub use plan::{InstallPlan, PlannedFile, PlannedFileKind};
pub use rmcl_domain::ExtractionPlan;
pub use vanilla::{plan_asset_objects, plan_asset_objects_with_base, plan_vanilla};
