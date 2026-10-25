fn main() {
    tauri_build::build();

    #[cfg(target_os = "macos")]
    {
        cc::Build::new()
            .file("native/recording_bridge.m")
            .include("native")
            .flag("-fobjc-arc")
            .compile("echo_memory_recording_bridge");
        println!("cargo:rustc-link-lib=framework=AVFoundation");
        println!("cargo:rustc-link-lib=framework=AVFAudio");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rerun-if-changed=native/recording_bridge.h");
        println!("cargo:rerun-if-changed=native/recording_bridge.m");
    }
}
