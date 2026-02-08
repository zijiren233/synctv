fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Configure tonic build to generate both client and server implementations
    // - Server: For exposing provider APIs to external clients (parse, browse, etc.)
    // - Client: For internal use by synctv-core (cross-provider communication)
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(
            &[
                "proto/alist.proto",
                "proto/bilibili.proto",
                "proto/emby.proto",
            ],
            &["proto"],
        )?;

    // Trigger rebuild if proto files change
    println!("cargo:rerun-if-changed=proto/alist.proto");
    println!("cargo:rerun-if-changed=proto/bilibili.proto");
    println!("cargo:rerun-if-changed=proto/emby.proto");

    Ok(())
}
