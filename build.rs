fn main() {
    // Compile using protox (no system protoc needed), bridging the prost version
    // difference by serialising the FileDescriptorSet to bytes with protox's
    // own prost version and deserialising with prost-build's prost-types.

    let fds = protox::compile(["proto/vector_tile.proto"], ["proto/"]).unwrap();

    // Serialise with protox's prost (0.14)
    let fds_bytes = {
        use protox::prost::Message as _;
        fds.encode_to_vec()
    };

    // Deserialise with prost-build's prost-types (0.13) and compile
    let fds_013 = {
        use prost::Message as _;
        prost_types::FileDescriptorSet::decode(fds_bytes.as_slice()).unwrap()
    };

    prost_build::Config::new()
        .compile_fds(fds_013)
        .unwrap();
}
