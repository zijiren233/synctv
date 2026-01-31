fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .out_dir("src/grpc/proto")
        .compile_protos(
            &["../proto/cluster.proto", "../proto/stream.proto"],
            &["../proto"],
        )?;
    Ok(())
}
