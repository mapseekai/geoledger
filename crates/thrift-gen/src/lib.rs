//! Generated Volo Thrift bindings; edit the IDL, not this output.
#![deny(unsafe_code)]
// Only upstream-generated code may use unsafe; the server uses typed dispatch.
#[allow(unsafe_code, clippy::all)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/volo_gen.rs"));
}
pub use generated::volo_gen::*;
