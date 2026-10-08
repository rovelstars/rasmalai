use crate::LinkError;

pub fn unsupported() -> LinkError {
    LinkError::Native("native backend not yet implemented for COFF targets".to_string())
}
