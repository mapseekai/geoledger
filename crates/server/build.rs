fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::path::PathBuf::from(std::env::var("OUT_DIR")?);
    tonic_prost_build::configure()
        .file_descriptor_set_path(output.join("spatial_version_descriptor.bin"))
        .compile_protos(&["proto/spatial_version.proto"], &["proto"])?;
    println!("cargo:rerun-if-changed=proto/spatial_version.proto");
    Ok(())
}
