fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    if std::env::var_os("CARGO_FEATURE_TEST_SERVER").is_some() {
        println!("cargo:rerun-if-changed=src/test_server/proto/addsvc.proto");

        let protoc = protoc_bin_vendored::protoc_bin_path().expect("failed to find protoc");
        std::env::set_var("PROTOC", protoc);

        tonic_build::configure()
            .build_server(true)
            .build_client(false)
            .file_descriptor_set_path(
                std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR not set"))
                    .join("addsvc_descriptor.bin"),
            )
            .compile(
                &["src/test_server/proto/addsvc.proto"],
                &["src/test_server/proto"],
            )
            .expect("failed to compile test gRPC proto");
    }
}
