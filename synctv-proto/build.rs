fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Compile all external-facing proto files
    // This includes client API, admin API, and provider protocols
    // Note: cluster.proto is internal and generated in synctv-cluster crate

    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .file_descriptor_set_path("src/descriptor.bin")
        .type_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]")
        .out_dir("src")
        .compile_protos(
            &[
                "../proto/client.proto",
                "../proto/admin.proto",
                "../proto/providers/bilibili.proto",
                "../proto/providers/alist.proto",
                "../proto/providers/emby.proto",
            ],
            &["../proto"],
        )?;

    Ok(())
}
