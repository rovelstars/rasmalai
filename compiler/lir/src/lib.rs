#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

pub mod accum;
pub mod arc_opt;
pub mod bce;
pub mod codegen_hash;
pub mod escape;
pub mod instr;
pub mod licm;
pub mod loopdep;
pub mod lower;
pub mod opt;
pub mod parallel;
pub mod perflint;
pub mod reach;
pub mod sroa;
pub mod tco;
pub mod temp_sweep;
pub mod verify;
