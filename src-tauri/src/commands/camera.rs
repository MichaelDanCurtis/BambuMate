//! The camera commands. Live view sends frames over a Channel as raw bytes
//! (see `camera::wire`); stills come back as `data:` URLs.

use base64::Engine;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, State};

use crate::camera::target::camera_pref;
use crate::camera::transport::OffReason;
use crate::camera::{wire, CameraHub, CameraState, LIVE_VIEW_PREF};

#[tauri::command]
pub async fn camera_state(
    app: AppHandle,
    hub: State<'_, CameraHub>,
) -> Result<CameraState, String> {
    if !camera_pref(&app, LIVE_VIEW_PREF) {
        return Ok(CameraState::off(OffReason::Disabled));
    }
    Ok(hub.probe().await)
}

#[tauri::command]
pub async fn camera_watch(
    app: AppHandle,
    hub: State<'_, CameraHub>,
    on_frame: Channel<InvokeResponseBody>,
) -> Result<u64, String> {
    if !camera_pref(&app, LIVE_VIEW_PREF) {
        return Err(OffReason::Disabled.message().to_string());
    }
    // `send` fails once the webview has dropped the channel: that ends the
    // watch the next time a frame is offered.
    Ok(hub.watch(Box::new(move |frame| {
        on_frame
            .send(InvokeResponseBody::Raw(wire::encode(frame)))
            .is_ok()
    })))
}

#[tauri::command]
pub async fn camera_unwatch(hub: State<'_, CameraHub>, id: u64) -> Result<(), String> {
    hub.unwatch(id);
    Ok(())
}

#[tauri::command]
pub async fn camera_still(
    app: AppHandle,
    hub: State<'_, CameraHub>,
) -> Result<Option<String>, String> {
    if !camera_pref(&app, LIVE_VIEW_PREF) {
        return Ok(None);
    }
    Ok(hub.still().await.map(|jpeg| {
        format!(
            "data:image/jpeg;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(jpeg)
        )
    }))
}

#[tauri::command]
pub async fn camera_retry(hub: State<'_, CameraHub>) -> Result<CameraState, String> {
    Ok(hub.retry())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::{json, Value};

    use crate::camera::session::{CameraState, CameraStatus};
    use crate::camera::transport::{OffReason, Transport};
    use crate::camera::CameraError;

    // The frontend's mirrors (`src/camera/types.rs`) are tested against this
    // file. Run with `UPDATE_WIRE_FIXTURE=1` to rewrite it after a
    // deliberate wire change.
    #[test]
    fn the_wire_fixture_holds_what_the_real_types_serialize() {
        let state = |transport, status| CameraState {
            transport,
            status,
            last_frame_at: Some(1_761_600_000),
        };
        let reasons = [
            OffReason::NotConfigured,
            OffReason::NotConnected,
            OffReason::Waiting,
            OffReason::NoCamera,
            OffReason::LiveviewOff,
            OffReason::Unsupported,
            OffReason::Disabled,
        ];
        let errors = [
            CameraError::Auth,
            CameraError::Tls,
            CameraError::Refused,
            CameraError::Busy,
            CameraError::Other {
                message: "The camera stream ended.".into(),
            },
        ];
        let fixture = json!({
            "idle": state(None, CameraStatus::Idle),
            "connecting": state(Some(Transport::Rtsps), CameraStatus::Connecting),
            "streaming_rtsps": state(Some(Transport::Rtsps), CameraStatus::Streaming),
            "streaming_jpeg": state(Some(Transport::Jpeg), CameraStatus::Streaming),
            "off": reasons.iter().map(|r| CameraState::off(*r)).collect::<Vec<_>>(),
            "error": errors.iter().map(|e| state(Some(Transport::Jpeg), CameraStatus::Error {
                error: e.clone(), message: e.message(),
            })).collect::<Vec<_>>(),
        });
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/camera/wire.json");
        if std::env::var_os("UPDATE_WIRE_FIXTURE").is_some() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(
                &path,
                serde_json::to_string_pretty(&fixture).unwrap() + "\n",
            )
            .unwrap();
        }
        let on_disk: Value = serde_json::from_str(
            &std::fs::read_to_string(&path).expect("src/camera/wire.json is missing"),
        )
        .unwrap();
        assert_eq!(
            on_disk, fixture,
            "the wire changed: rerun with UPDATE_WIRE_FIXTURE=1 and update the frontend mirrors"
        );
    }
}
