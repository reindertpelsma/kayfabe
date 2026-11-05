//! The refresh stress arm (`--refresh-stress`): the Windows refresh shape reproduced with Linux
//! RM verbs, so the gpu-vast CI can watch the host stitch across drivers and dies without Enscape.
//!
//! | file | what |
//! |---|---|
//! | [`params`] | flags, safe defaults (24 x 4,096 pages), validation by name |
//! | [`plan`] | the memory budget, decided before any allocation |
//! | [`rng`] | seeded permutation and sampling |
//! | [`census`] | run census and the qualification (NOTRUN when the memory is not fragmented) |
//! | [`oracle`] | signatures, sample plan, decoy, tallies |
//! | [`backend`] | the one trait all hardware and OS access goes through |
//! | [`run`] | the orchestration (pin, map, one invalidate, oracle, unmap, free) |
//! | [`verdict`] | the gates as a pure function of the evidence |
//! | [`compare`] | the native-versus-guest ratio from two logs |
//! | [`calibration`] | every threshold, flagged CALIBRATION-NEEDED where unmeasured |

pub mod backend;
pub mod calibration;
pub mod census;
pub mod compare;
pub mod oracle;
pub mod params;
pub mod plan;
pub mod rng;
pub mod run;
pub mod verdict;

#[cfg(test)]
pub(crate) mod fake;
#[cfg(test)]
mod scenarios;
