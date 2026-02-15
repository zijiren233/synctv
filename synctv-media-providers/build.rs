fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Compile media provider proto files to src/proto/
    // - Server: For the standalone media-provider-server binary
    // - Client: For synctv-core inter-service communication
    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .type_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]")
        .file_descriptor_set_path("src/proto/descriptor.bin")
        .out_dir("src/proto")
        .compile_protos(
            &[
                "proto/alist.proto",
                "proto/bilibili.proto",
                "proto/emby.proto",
            ],
            &["proto"],
        )?;

    println!("cargo:rerun-if-changed=proto/alist.proto");
    println!("cargo:rerun-if-changed=proto/bilibili.proto");
    println!("cargo:rerun-if-changed=proto/emby.proto");

    Ok(())
}
