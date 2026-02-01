fn main() -> Result<(), Box<dyn std::error::Error>> {
    // All proto files are now generated in synctv-proto crate
    // No proto generation needed here

    // Copy descriptor set from synctv-proto for gRPC reflection
    let proto_descriptor = std::path::Path::new("../synctv-proto/src/descriptor.bin");
    let reflection_descriptor = std::path::Path::new("src/grpc/proto/descriptor.bin");

    if proto_descriptor.exists() {
        std::fs::create_dir_all(reflection_descriptor.parent().unwrap())?;
        std::fs::copy(proto_descriptor, reflection_descriptor)?;
    }

    Ok(())
}

