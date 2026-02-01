fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Compile cluster.proto from the proto directory
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .out_dir("src/grpc/proto")
        .compile_protos(&["proto/cluster.proto"], &["proto"])?;
    Ok(())
}
