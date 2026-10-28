//! The navigation mesh of the original's server (CMaNGOS "mmaps"): the surface that creatures walk on, built tile by tile from the terrain and the
//! collision of buildings with the numbers of its tool (`contrib/mmap`, `MapBuilder`), and the path queries on it (`PathFinder`).

mod build;
mod cache;
mod mesh;
mod pathfinder;
mod query;
#[cfg(test)]
pub(crate) mod tests;

pub use build::{build, Tile, AREA_GROUND, AREA_STEEP, AREA_WATER, BORDER_YARDS, HALF_WORLD, TILE};
pub use cache::NavCache;
pub use mesh::{from_nav, to_nav, MeshTile, NavMesh, NAV_GROUND, NAV_MAGMA_SLIME, NAV_WATER};
pub use query::{Filter, Query};
pub use pathfinder::*;
