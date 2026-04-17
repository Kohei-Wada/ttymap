fn main() {
    let fds = protox::compile(["proto/vector_tile.proto"], ["proto/"]).unwrap();
    prost_build::Config::new().compile_fds(fds).unwrap();
}
