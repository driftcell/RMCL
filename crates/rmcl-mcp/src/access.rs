#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessLevel {
    ReadOnly,
    ReadWrite,
}

impl AccessLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::ReadWrite => "read_write",
        }
    }
}
