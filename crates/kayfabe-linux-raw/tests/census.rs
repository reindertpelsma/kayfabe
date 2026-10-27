//! Census assertions run in a process with no unrelated ioctl writers.

const PRIOR_IOCTLS: u64 = 1;

include!("common/census.rs");
