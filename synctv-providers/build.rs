fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_dir = "../proto";
    let providers_dir = format!("{}/providers", proto_dir);

    // Configure tonic build to generate both server and client code
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(
            &[
                format!("{}/alist.proto", providers_dir),
                format!("{}/bilibili.proto", providers_dir),
                format!("{}/emby.proto", providers_dir),
            ],
            &[proto_dir],
        )?;

    // Trigger rebuild if proto files change
    println!("cargo:rerun-if-changed={}/alist.proto", providers_dir);
    println!("cargo:rerun-if-changed={}/bilibili.proto", providers_dir);
    println!("cargo:rerun-if-changed={}/emby.proto", providers_dir);

    Ok(())
}
