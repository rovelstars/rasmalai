use crate::LinkError;

pub fn unsupported() -> LinkError {
    LinkError::Native("native backend not yet implemented for Mach-O targets".to_string())
}
