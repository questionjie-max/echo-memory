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

        // v1.1：系统内放声采集（ScreenCaptureKit）。权限判定用 CoreGraphics 的
        // 屏幕捕获预检 API（macOS 15 起授权项为「屏幕与系统声音录制」）。
        cc::Build::new()
            .file("native/system_audio_bridge.m")
            .include("native")
            .flag("-fobjc-arc")
            .compile("echo_memory_system_audio_bridge");
        println!("cargo:rustc-link-lib=framework=ScreenCaptureKit");
        println!("cargo:rustc-link-lib=framework=CoreGraphics");
        println!("cargo:rustc-link-lib=framework=CoreMedia");
        println!("cargo:rerun-if-changed=native/system_audio_bridge.h");
        println!("cargo:rerun-if-changed=native/system_audio_bridge.m");
    }
}
