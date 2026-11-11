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
        println!("cargo:rustc-link-lib=framework=AudioToolbox");
        println!("cargo:rustc-link-lib=framework=CoreAudio");
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

        // 钥匙串写入（Security 框架）。CLI 的 stdin 传密码在 macOS 15 上
        // 会静默写空密码，-w 参数又会把密钥暴露在 ps 里，故走原生 API。
        cc::Build::new()
            .file("native/keychain_bridge.m")
            .include("native")
            .flag("-fobjc-arc")
            .compile("echo_memory_keychain_bridge");
        println!("cargo:rustc-link-lib=framework=Security");
        println!("cargo:rerun-if-changed=native/keychain_bridge.h");
        println!("cargo:rerun-if-changed=native/keychain_bridge.m");
    }
}
