use crate::{
    catalog,
    live2d::{self, Live2dScene},
    model::{inspect_model, inspect_model_inner, ArtMeshItem, ModelInspectRequest},
    session::{apply_art_mesh_alias, validate_session, DisplayResponse, DisplaySession},
};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, RwLock},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone)]
pub struct DisplayState {
    pub session: Option<DisplaySession>,
    pub scene: Option<Live2dScene>,
    pub available_art_meshes: Vec<ArtMeshItem>,
    pub picker: Option<PickerSession>,
}

impl DisplayState {
    pub fn new() -> Self {
        Self {
            session: None,
            scene: None,
            available_art_meshes: Vec::new(),
            picker: None,
        }
    }

    pub fn pick_artmesh_at(
        &mut self,
        _x: f64,
        y: f64,
        width: u32,
        height: u32,
        commit_filter: bool,
    ) {
        if self.picker.is_none()
            || self.available_art_meshes.is_empty()
            || width == 0
            || height == 0
        {
            return;
        }
        let index = ((y / height as f64).clamp(0.0, 0.999_999)
            * self.available_art_meshes.len() as f64) as usize;
        let item = self
            .available_art_meshes
            .get(index)
            .map(|mesh| apply_art_mesh_alias(self.session.as_ref(), mesh));
        if let Some(picker) = &mut self.picker {
            picker.status = "active".into();
            picker.hovered = item.clone();
            if commit_filter {
                picker.filtered = item.into_iter().collect();
            }
        }
    }

    pub fn picker_purpose_label(&self) -> Option<String> {
        let picker = self.picker.as_ref()?;
        let purpose = picker.purpose.as_deref()?;
        describe_picker_purpose(purpose, self.session.as_ref())
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PickerCreateRequest {
    pub purpose: Option<String>,
    #[serde(default)]
    pub selected_ids: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PickerSession {
    pub session_id: String,
    pub status: String,
    pub purpose: Option<String>,
    pub selected: Vec<ArtMeshItem>,
    pub filtered: Vec<ArtMeshItem>,
    pub hovered: Option<ArtMeshItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Default for DisplayState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: serde_json::Value,
}

pub fn spawn_http_server(addr: String, state: Arc<RwLock<DisplayState>>) -> std::io::Result<()> {
    let listener = TcpListener::bind(&addr)?;
    listener.set_nonblocking(true)?;
    thread::Builder::new()
        .name("nanavts-display-http".into())
        .spawn(move || loop {
            match listener.accept() {
                Ok((stream, _)) => handle_stream(stream, state.clone()),
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(8));
                }
                Err(err) => eprintln!("display http accept failed: {err}"),
            }
        })?;
    Ok(())
}

pub fn route_request(request_text: &str, state: &Arc<RwLock<DisplayState>>) -> HttpResponse {
    let Some((method, path)) = request_line(request_text) else {
        return json_response(
            400,
            serde_json::json!({ "ok": false, "error": "bad_request" }),
        );
    };

    if method == "OPTIONS" {
        return json_response(200, serde_json::json!({ "ok": true }));
    }
    if method == "GET" && path == "/schema" {
        return json_response(200, catalog::effect_schema());
    }
    if method == "POST" && path == "/model/inspect" {
        return handle_model_inspect(request_text);
    }
    if method == "POST" && path == "/session" {
        return handle_session(request_text, state);
    }
    if method == "POST" && path == "/artmesh-picker/sessions" {
        return handle_picker_create(request_text, state);
    }
    if method == "GET" && path.starts_with("/artmesh-picker/sessions/") {
        return handle_picker_get(&path, state);
    }
    json_response(
        404,
        serde_json::json!({ "ok": false, "error": "not_found" }),
    )
}

fn handle_model_inspect(request_text: &str) -> HttpResponse {
    let Some(body) = request_body(request_text) else {
        return json_response(
            400,
            serde_json::json!({ "ok": false, "availableArtMeshes": [], "warnings": [], "error": "bad_request" }),
        );
    };
    let request = match serde_json::from_str::<ModelInspectRequest>(body) {
        Ok(request) => request,
        Err(_) => {
            return json_response(
                400,
                serde_json::json!({ "ok": false, "availableArtMeshes": [], "warnings": [], "error": "invalid_json" }),
            )
        }
    };
    let response = inspect_model(request.model_json_path);
    let status = if response.ok { 200 } else { 400 };
    json_response(
        status,
        serde_json::to_value(response).expect("serializable model inspect response"),
    )
}

fn handle_stream(mut stream: TcpStream, state: Arc<RwLock<DisplayState>>) {
    let mut buffer = Vec::new();
    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
    let mut chunk = [0_u8; 4096];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => {
                buffer.extend_from_slice(&chunk[..count]);
                if request_complete(&buffer) {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    let request_text = String::from_utf8_lossy(&buffer);
    let response = route_request(&request_text, &state);
    let _ = write_json_response(&mut stream, response);
}

fn handle_session(request_text: &str, state: &Arc<RwLock<DisplayState>>) -> HttpResponse {
    let Some(body) = request_body(request_text) else {
        return json_response(
            400,
            serde_json::json!({ "ok": false, "error": "bad_request" }),
        );
    };
    let parsed = match serde_json::from_str::<DisplaySession>(body) {
        Ok(parsed) => parsed,
        Err(_) => {
            return json_response(
                400,
                serde_json::json!({ "ok": false, "error": "invalid_json" }),
            )
        }
    };
    if parsed.channels.is_empty() && parsed.effect_parts.is_empty() {
        return json_response(
            400,
            serde_json::json!({ "ok": false, "error": "invalid_session_shape" }),
        );
    }

    let warnings = match validate_session(&parsed) {
        Ok(warnings) => warnings,
        Err(error) => {
            return json_response(
                400,
                serde_json::json!({
                    "ok": false,
                    "error": error,
                    "renderer": "wgpu",
                    "channelCount": parsed.channels.len(),
                    "modelLoaded": false,
                    "warnings": []
                }),
            )
        }
    };

    let scene = if let Some(model) = &parsed.live2d_model {
        match live2d::load_scene(&model.model_json_path) {
            Ok(scene) => Some(scene),
            Err(error) => {
                {
                    let mut guard = state.write().expect("display state poisoned");
                    guard.session = None;
                    guard.scene = None;
                    guard.available_art_meshes.clear();
                    guard.picker = None;
                }
                return json_response(
                    400,
                    serde_json::json!({
                        "ok": false,
                        "error": error,
                        "renderer": "wgpu",
                        "channelCount": parsed.channels.len(),
                        "modelLoaded": false,
                        "warnings": []
                    }),
                );
            }
        }
    } else {
        None
    };

    {
        let mut guard = state.write().expect("display state poisoned");
        guard.available_art_meshes = scene
            .as_ref()
            .map(|scene| {
                scene
                    .art_meshes
                    .clone()
                    .into_iter()
                    .map(ArtMeshItem::from)
                    .collect()
            })
            .or_else(|| {
                parsed.live2d_model.as_ref().and_then(|model| {
                    inspect_model_inner(std::path::Path::new(&model.model_json_path)).ok()
                })
            })
            .unwrap_or_default();
        guard.scene = scene;
        guard.session = Some(parsed.clone());
    }

    let response = DisplayResponse {
        ok: true,
        channel_count: parsed.channels.len(),
        renderer: "wgpu",
        model_loaded: parsed.live2d_model.is_some(),
        warnings,
        error: None,
    };
    json_response(
        200,
        serde_json::to_value(response).expect("serializable response"),
    )
}

fn handle_picker_create(request_text: &str, state: &Arc<RwLock<DisplayState>>) -> HttpResponse {
    let Some(body) = request_body(request_text) else {
        return json_response(400, serde_json::json!({ "session": null }));
    };
    let request = match serde_json::from_str::<PickerCreateRequest>(body) {
        Ok(request) => request,
        Err(_) => return json_response(400, serde_json::json!({ "session": null })),
    };
    let mut guard = state.write().expect("display state poisoned");
    let active_session = guard.session.as_ref();
    let has_session = active_session.is_some();
    let selected = request
        .selected_ids
        .iter()
        .filter_map(|id| {
            guard
                .available_art_meshes
                .iter()
                .find(|mesh| &mesh.id == id)
                .map(|mesh| apply_art_mesh_alias(active_session, mesh))
        })
        .collect();
    let status = if has_session { "active" } else { "failed" };
    let error = (!has_session).then(|| "session_unavailable".into());
    let session = PickerSession {
        session_id: format!("display-{}", now_millis()),
        status: status.into(),
        purpose: request.purpose,
        selected,
        filtered: Vec::new(),
        hovered: None,
        error,
    };
    guard.picker = Some(session.clone());
    json_response(200, serde_json::json!({ "session": session }))
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn handle_picker_get(path: &str, state: &Arc<RwLock<DisplayState>>) -> HttpResponse {
    let session_id = path.trim_start_matches("/artmesh-picker/sessions/");
    let guard = state.read().expect("display state poisoned");
    let session = guard
        .picker
        .as_ref()
        .filter(|session| session.session_id == session_id)
        .cloned();
    json_response(200, serde_json::json!({ "session": session }))
}

fn describe_picker_purpose(purpose: &str, session: Option<&DisplaySession>) -> Option<String> {
    let mut parts = purpose.split(':');
    if parts.next()? != "channel" {
        return None;
    }
    let channel_index = parts.next()?.parse::<usize>().ok()?;
    let field = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let channel_name = session
        .and_then(|session| session.channels.get(channel_index))
        .map(|channel| channel.name.trim())
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("通道 {}", channel_index + 1));
    let target = if field == "maskArtMeshIds" {
        "遮罩"
    } else {
        "目标"
    };
    Some(format!("{channel_name} {target}"))
}

fn request_line(request_text: &str) -> Option<(String, String)> {
    let mut parts = request_text.lines().next()?.split_whitespace();
    let method = parts.next()?.to_ascii_uppercase();
    let path = parts.next()?.split('?').next()?.to_string();
    Some((method, path))
}

fn request_body(request_text: &str) -> Option<&str> {
    let marker = "\r\n\r\n";
    let index = request_text.find(marker)?;
    Some(&request_text[index + marker.len()..])
}

fn request_complete(buffer: &[u8]) -> bool {
    let text = String::from_utf8_lossy(buffer);
    let Some(header_end) = text.find("\r\n\r\n") else {
        return false;
    };
    let content_length = text[..header_end]
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    buffer.len() >= header_end + 4 + content_length
}

fn json_response(status: u16, body: serde_json::Value) -> HttpResponse {
    HttpResponse { status, body }
}

fn write_json_response(mut stream: impl Write, response: HttpResponse) -> std::io::Result<()> {
    let payload = serde_json::to_vec(&response.body)?;
    let reason = if response.status == 200 {
        "OK"
    } else {
        "Error"
    };
    write!(
        stream,
        "HTTP/1.1 {} {}\r\nContent-Type: application/json; charset=utf-8\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.status,
        reason,
        payload.len()
    )?;
    stream.write_all(&payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{
        collections::BTreeMap,
        path::PathBuf,
        sync::{Arc, RwLock},
    };

    #[cfg(not(feature = "live2d-cubism"))]
    fn write_test_model(moc_bytes: &[u8]) -> (PathBuf, PathBuf) {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("nanavts-display-http-{suffix}"));
        std::fs::create_dir_all(&root).unwrap();
        let moc = root.join("sample.moc3");
        let model = root.join("sample.model3.json");
        std::fs::write(&model, r#"{"FileReferences":{"Moc":"sample.moc3"}}"#).unwrap();
        std::fs::write(moc, moc_bytes).unwrap();
        (root, model)
    }

    fn assert_mesh(value: &serde_json::Value, path: &str, id: &str, label: &str) {
        assert_eq!(value.pointer(&format!("{path}/id")), Some(&json!(id)));
        assert_eq!(value.pointer(&format!("{path}/label")), Some(&json!(label)));
    }

    #[test]
    fn accepts_compatible_session_payload() {
        let state = Arc::new(RwLock::new(DisplayState::new()));
        let body = json!({
            "sessionId": "tauri-test",
            "sentAt": "2026-06-25T00:00:00.000Z",
            "sourceBaseUrl": "http://127.0.0.1:19576",
            "activeChannelIndex": 0,
            "channels": [{ "name": "main", "parts": ["recolor"], "values": { "color": "#00ff00" } }],
            "effectParts": [{ "id": "recolor", "defaultsOn": { "strength": 1.0 } }]
        })
        .to_string();
        let request = format!(
            "POST /session HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );

        let response = route_request(&request, &state);

        assert_eq!(response.status, 200);
        assert_eq!(response.body.get("ok"), Some(&json!(true)));
        assert_eq!(response.body.get("renderer"), Some(&json!("wgpu")));
        assert!(state.read().unwrap().session.is_some());
    }

    #[test]
    fn serves_display_schema() {
        let state = Arc::new(RwLock::new(DisplayState::new()));
        let response = route_request("GET /schema HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n", &state);

        assert_eq!(response.status, 200);
        let parts = response
            .body
            .pointer("/schema/meta/parts")
            .and_then(|value| value.as_array())
            .expect("parts array");
        assert!(parts
            .iter()
            .any(|part| part.get("id") == Some(&json!("recolor"))));
    }

    #[test]
    fn picker_session_reports_clicked_filter() {
        let state = Arc::new(RwLock::new(DisplayState::new()));
        {
            let mut guard = state.write().unwrap();
            guard.session = Some(DisplaySession {
                art_mesh_aliases: BTreeMap::from([
                    ("ArtMeshA".into(), "脸部底色".into()),
                    ("ArtMeshB".into(), "前发高光".into()),
                ]),
                ..DisplaySession::default()
            });
            guard.available_art_meshes = vec![
                ArtMeshItem {
                    id: "ArtMeshA".into(),
                    label: "ArtMeshA".into(),
                    original_name: "ArtMeshA".into(),
                    index: 0,
                    mask_type: "unknown".into(),
                },
                ArtMeshItem {
                    id: "ArtMeshB".into(),
                    label: "ArtMeshB".into(),
                    original_name: "ArtMeshB".into(),
                    index: 1,
                    mask_type: "unknown".into(),
                },
            ];
        }
        let body =
            json!({ "purpose": "channel:0:artMeshIds", "selectedIds": ["ArtMeshA"] }).to_string();
        let request = format!(
            "POST /artmesh-picker/sessions HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );

        let created = route_request(&request, &state);
        let session_id = created
            .body
            .pointer("/session/sessionId")
            .and_then(|value| value.as_str())
            .unwrap();
        state
            .write()
            .unwrap()
            .pick_artmesh_at(10.0, 90.0, 100, 100, false);
        let polled_hover = route_request(
            &format!(
                "GET /artmesh-picker/sessions/{session_id} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n"
            ),
            &state,
        );
        state
            .write()
            .unwrap()
            .pick_artmesh_at(10.0, 90.0, 100, 100, true);
        let polled = route_request(
            &format!(
                "GET /artmesh-picker/sessions/{session_id} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n"
            ),
            &state,
        );

        assert_eq!(created.status, 200);
        assert_mesh(&created.body, "/session/selected/0", "ArtMeshA", "脸部底色");
        assert_mesh(
            &polled_hover.body,
            "/session/hovered",
            "ArtMeshB",
            "前发高光",
        );
        assert_mesh(&polled.body, "/session/filtered/0", "ArtMeshB", "前发高光");
    }

    #[test]
    #[cfg(not(feature = "live2d-cubism"))]
    fn inspects_model_art_meshes() {
        let state = Arc::new(RwLock::new(DisplayState::new()));
        let (root, model) = write_test_model(b"ArtMeshFirst\0ArtMeshSecond\0ArtMeshFirst");
        let body = json!({ "modelJsonPath": model }).to_string();
        let request = format!(
            "POST /model/inspect HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );

        let response = route_request(&request, &state);

        assert_eq!(response.status, 400);
        assert_eq!(response.body.get("ok"), Some(&json!(false)));
        assert_eq!(
            response.body.get("error"),
            Some(&json!("live2d_runtime_unavailable"))
        );
        assert_eq!(response.body.get("availableArtMeshes"), Some(&json!([])));

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_unknown_effect_part() {
        let state = Arc::new(RwLock::new(DisplayState::new()));
        let body = json!({
            "channels": [{ "name": "main", "parts": ["missing"] }],
            "effectParts": [{ "id": "missing", "defaultsOn": {} }]
        })
        .to_string();
        let request = format!(
            "POST /session HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );

        let response = route_request(&request, &state);

        assert_eq!(response.status, 400);
        assert_eq!(
            response.body.get("error"),
            Some(&json!("unknown_effect_part:missing"))
        );
    }

    #[test]
    fn rejects_unknown_art_mesh() {
        let state = Arc::new(RwLock::new(DisplayState::new()));
        #[cfg(not(feature = "live2d-cubism"))]
        let (root, model) = write_test_model(b"ArtMeshKnown");
        #[cfg(feature = "live2d-cubism")]
        let model = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("assets")
            .join("live2d")
            .join("运作档")
            .join("news.model3.json");
        let body = json!({
            "channels": [{
                "name": "main",
                "parts": ["recolor"],
                "artMeshIds": ["DefinitelyMissingArtMesh"]
            }],
            "effectParts": [{ "id": "recolor", "defaultsOn": { "strength": 1.0 } }],
            "live2dModel": { "name": "fixture", "modelJsonPath": model }
        })
        .to_string();
        let request = format!(
            "POST /session HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );

        let response = route_request(&request, &state);

        assert_eq!(response.status, 400);
        #[cfg(not(feature = "live2d-cubism"))]
        assert_eq!(
            response.body.get("error"),
            Some(&json!("live2d_runtime_unavailable"))
        );
        #[cfg(feature = "live2d-cubism")]
        assert_eq!(
            response.body.get("error"),
            Some(&json!("unknown_art_mesh:DefinitelyMissingArtMesh"))
        );

        #[cfg(not(feature = "live2d-cubism"))]
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_missing_model_path() {
        let state = Arc::new(RwLock::new(DisplayState::new()));
        let body = json!({
            "sessionId": "tauri-test",
            "sentAt": "2026-06-25T00:00:00.000Z",
            "sourceBaseUrl": "http://127.0.0.1:19576",
            "channels": [{ "name": "main" }],
            "effectParts": [],
            "live2dModel": {
                "id": "missing",
                "name": "Missing",
                "source": "display",
                "modelJsonPath": "Z:/missing/model.model3.json",
                "rootDir": "Z:/missing",
                "relativePath": "model.model3.json"
            }
        })
        .to_string();
        let request = format!(
            "POST /session HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );

        let response = route_request(&request, &state);

        assert_eq!(response.status, 400);
        assert_eq!(
            response.body.get("error"),
            Some(&json!("live2d_model_not_found"))
        );
        assert!(state.read().unwrap().session.is_none());
    }
}
