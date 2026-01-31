fn main() -> Result<(), Box<dyn std::error::Error>> {
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
    Ok(())
}
