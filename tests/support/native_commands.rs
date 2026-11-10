use serde_json::{json, Value};
use std::{os::unix::net::UnixStream, time::Duration};
use trailboss_runtime::protocol::{
    self, endpoint::*, ClientMessage, ClientSurfaceSize, ServerMessage,
};

/// The palette's non-activating observer transport, not the runnable API catalog.
pub fn catalog(core: &super::support::Core) -> Value {
    core.request("plugin.list", json!({}));
    let socket = core.home.join(format!(
        "trailboss/sessions/{}/trailboss-client.sock",
        core.name
    ));
    let mut endpoint = UnixStream::connect(socket).unwrap();
    endpoint
        .set_read_timeout(Some(Duration::from_secs(8)))
        .unwrap();
    let hello = EndpointClientHello {
        generation: ENDPOINT_PROTOCOL_GENERATION,
        cell_width_px: 0,
        cell_height_px: 0,
        surface_size: ClientSurfaceSize { cols: 80, rows: 24 },
        pixel_mouse: false,
        direct_graphics: false,
        endpoint_keybindings: false,
        mouse_capture: false,
        surface_active: false,
        surface_reuse: false,
        surface_delta: false,
        surface_scroll: false,
        session_switch: false,
        tab_transfer: false,
        snapshot_codecs: vec![SNAPSHOT_CODEC_V1.into()],
        surface_codecs: vec![SURFACE_CODEC_V1.into()],
        input_codecs: vec![INPUT_CODEC_V1.into()],
        blob_codecs: vec![BLOB_CODEC_V1.into()],
    };
    protocol::write_message(
        &mut endpoint,
        &ClientMessage::EndpointControl {
            kind: ENDPOINT_HELLO_KIND.into(),
            data: serde_json::to_string(&hello).unwrap(),
        },
    )
    .unwrap();
    loop {
        if let ServerMessage::EndpointControl { kind, data } =
            protocol::read_message::<_, ServerMessage>(&mut endpoint, protocol::MAX_FRAME_SIZE)
                .unwrap()
        {
            if kind == PLUGIN_UI_KIND {
                let projection: EndpointPluginUi = serde_json::from_str(&data).unwrap();
                // Immutable old-release compatibility fixtures predate this field.
                if projection.commands.is_empty() {
                    return core.request("ui.command.list", json!({}));
                }
                return json!({"type":"ui_command_list","boot_id":projection.boot_id,"commands":projection.commands,
                    "capabilities":core.request("ui.command.list",json!({}))["capabilities"]});
            }
        }
    }
}
