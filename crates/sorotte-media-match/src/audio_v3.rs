use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioLandmarkV3 {
    pub hash: u32,
    pub t_ms: u32,
    pub weight: u8,
}
