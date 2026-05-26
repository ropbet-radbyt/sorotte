use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoLandmarkV3 {
    pub bucket: u32,
    pub hash64: u64,
    pub t_ms: u32,
    pub kind: u8,
    pub weight: u8,
}
