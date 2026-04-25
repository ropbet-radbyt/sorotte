use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomRef {
    pub name: String,
}

impl RoomRef {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}
