fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR")?);
    tonic_prost_build::configure()
        .type_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]")
        .type_attribute(".", "#[serde(default)]")
        .file_descriptor_set_path(out.join("descriptor.bin"))
        .compile_protos(
            &["../../proto/geoledger/v1/geoledger.proto"],
            &["../../proto"],
        )?;
    println!("cargo:rerun-if-changed=../../proto/geoledger/v1/geoledger.proto");
    Ok(())
}
