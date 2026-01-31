fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Compile core proto files (client, admin, cluster)
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .file_descriptor_set_path("src/grpc/proto/descriptor.bin")
        .out_dir("src/grpc/proto")
        .compile_protos(
            &[
                "../proto/client.proto",
                "../proto/admin.proto",
                "../proto/cluster.proto",
            ],
            &["../proto"],
        )?;

    // Compile provider proto files to providers/proto directory
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .out_dir("src/grpc/providers/proto")
        .compile_protos(
            &[
                "../proto/providers/bilibili.proto",
                "../proto/providers/alist.proto",
                "../proto/providers/emby.proto",
            ],
            &["../proto"],
        )?;

    Ok(())
}
