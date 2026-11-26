//! Molecular docking with AutoDock Vina.
//!
//! The pipeline is four stages behind traits (`stages`), currently backed by Python
//! (`python`) with Vina as the engine (`vina`). `env` owns the tools, `report` writes a
//! self-contained HTML figure, and `manifest` records what was run.

pub mod cli;
pub mod env;
pub mod manifest;
pub mod python;
pub mod report;
pub mod stages;
pub mod vina;
