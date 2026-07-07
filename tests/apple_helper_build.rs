#[path = "../build_support/apple_helper.rs"]
mod apple_helper;

#[test]
fn swift_target_triple_maps_release_arches_to_macos15() {
    assert_eq!(
        apple_helper::swift_target_triple("aarch64", apple_helper::DEFAULT_MACOS_DEPLOYMENT_TARGET)
            .unwrap(),
        "arm64-apple-macosx15.0"
    );
    assert_eq!(
        apple_helper::swift_target_triple("x86_64", "15.0").unwrap(),
        "x86_64-apple-macosx15.0"
    );
}

#[test]
fn swift_target_triple_rejects_invalid_inputs() {
    assert!(apple_helper::swift_target_triple("armv7", "15.0").is_err());
    assert!(apple_helper::swift_target_triple("aarch64", "").is_err());
}

#[test]
fn macos_helper_targets_include_asr_and_capture_helpers() {
    let targets = apple_helper::macos_helper_targets();
    assert_eq!(targets.len(), 2);
    assert_eq!(targets[0].output_name, "apple_helper");
    assert_eq!(
        targets[0].source_path,
        "src/asr/providers/apple_helper.swift"
    );
    assert_eq!(targets[1].output_name, "apple_capture_helper");
    assert_eq!(
        targets[1].source_path,
        "src/voice/apple_capture_helper.swift"
    );
}

#[test]
fn apple_capture_helper_uses_default_other_audio_ducking_without_advanced_ducking() {
    let source = std::fs::read_to_string("src/voice/apple_capture_helper.swift").unwrap();

    assert!(
        source.contains("voiceProcessingOtherAudioDuckingConfiguration"),
        "Apple voice-processing capture must configure other-audio ducking explicitly"
    );
    assert!(
        source.contains("enableAdvancedDucking: false"),
        "advanced ducking should stay disabled for dictation capture"
    );
    assert!(
        source.contains("duckingLevel: .default"),
        "other-audio ducking should use Apple's default level for dictation capture"
    );
}

#[test]
fn apple_capture_server_keeps_warm_capture_without_prestarting_voice_processing() {
    let source = std::fs::read_to_string("src/voice/apple_capture_helper.swift").unwrap();
    let run_server_start = source.find("private func runServer()").unwrap();
    let run_lifecycle_start = source.find("private func runLifecycleSmoke").unwrap();
    let run_server = &source[run_server_start..run_lifecycle_start];

    assert!(
        run_server.contains("let capture = try VoiceProcessedCapture"),
        "server mode should keep a warm capture object for fast repeated starts"
    );
    assert!(
        !run_server.contains("capture = nil"),
        "server mode should not destroy the warm capture after each stop"
    );
    assert!(
        !run_server.contains("configureVoiceProcessing"),
        "server mode must not enable voice processing before start"
    );
}

#[test]
fn apple_capture_server_requests_permission_before_ready_without_engine() {
    let source = std::fs::read_to_string("src/voice/apple_capture_helper.swift").unwrap();
    let run_server_start = source.find("private func runServer()").unwrap();
    let run_lifecycle_start = source.find("private func runLifecycleSmoke").unwrap();
    let run_server = &source[run_server_start..run_lifecycle_start];
    let permission_index = run_server.find("try ensureRecordPermission()").unwrap();
    let ready_index = run_server.find("\"event\": \"server_ready\"").unwrap();

    assert!(
        permission_index < ready_index,
        "server mode should handle slow TCC permission before reporting ready"
    );
    assert!(
        !run_server.contains("AVAudioEngine()"),
        "permission preflight must not initialize AVAudioEngine directly in runServer"
    );
}

#[test]
fn apple_capture_server_rejects_duplicate_start() {
    let source = std::fs::read_to_string("src/voice/apple_capture_helper.swift").unwrap();
    let run_server_start = source.find("private func runServer()").unwrap();
    let run_lifecycle_start = source.find("private func runLifecycleSmoke").unwrap();
    let run_server = &source[run_server_start..run_lifecycle_start];

    assert!(
        run_server.contains("try capture.start()"),
        "duplicate start should be rejected by the warm capture object"
    );
    assert!(
        source.contains("guard !running else"),
        "VoiceProcessedCapture.start should reject duplicate starts"
    );
}

#[test]
fn apple_capture_stop_disables_voice_processing() {
    let source = std::fs::read_to_string("src/voice/apple_capture_helper.swift").unwrap();
    let stop_start = source.find("func stop()").unwrap();
    let accept_start = source.find("private func accept").unwrap();
    let stop_body = &source[stop_start..accept_start];

    assert!(
        stop_body.contains("disableVoiceProcessing(input)"),
        "stop must disable voice processing so idle helper does not keep ducking other audio"
    );
    assert!(
        source.contains("setVoiceProcessingEnabled(false)"),
        "helper must explicitly disable AVAudio voice processing after stop"
    );
}

#[test]
fn apple_capture_lifecycle_smoke_disables_voice_processing_between_rounds() {
    let source = std::fs::read_to_string("src/voice/apple_capture_helper.swift").unwrap();
    let lifecycle_start = source.find("private func runLifecycleSmoke").unwrap();
    let capture_start = source
        .find("private final class VoiceProcessedCapture")
        .unwrap();
    let lifecycle_body = &source[lifecycle_start..capture_start];

    assert!(
        lifecycle_body.contains("try configureVoiceProcessing(input)"),
        "lifecycle smoke should measure per-round voice-processing enable cost"
    );
    assert!(
        lifecycle_body.contains("disableVoiceProcessing(input)"),
        "lifecycle smoke should release voice processing before its idle gap"
    );
}
