#![forbid(unsafe_code)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    volo_build::Builder::thrift()
        .add_service("idl/geoledger.thrift")
        .write()?;
    println!("cargo:rerun-if-changed=idl/geoledger.thrift");
    Ok(())
}
