fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Only compile cluster.proto - stream.proto is now internal to synctv-stream
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .out_dir("src/grpc/proto")
        .compile_protos(&["../proto/cluster.proto"], &["../proto"])?;
    Ok(())
}
