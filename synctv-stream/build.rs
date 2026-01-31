fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Compile stream.proto for stream relay service
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(
            &["../proto/stream.proto"],
            &["../proto"],
        )?;

    Ok(())
}
