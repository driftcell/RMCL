//! Pure installation planning for Minecraft and its components.

mod coordinate;
mod error;
mod plan;
mod vanilla;

pub use coordinate::{MavenCoordinate, MavenCoordinateError};
pub use error::InstallPlanError;
pub use plan::{ExtractionPlan, InstallPlan, PlannedFile, PlannedFileKind};
pub use vanilla::{plan_asset_objects, plan_vanilla};
