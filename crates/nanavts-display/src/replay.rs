use crate::session::DisplaySession;
use serde_json::Value;
use std::{
    fs,
    io::{Read, Write},
    net::TcpStream,
    path::Path,
    thread,
    time::Duration,
};

const DEFAULT_REPLAY_SESSION: &str = include_str!("../fixtures/replay-session.json");

pub fn load_replay_session(path: Option<&Path>) -> Result<DisplaySession, String> {
    let raw = match path {
        Some(path) => fs::read_to_string(path)
            .map_err(|err| format!("failed to read replay session {}: {err}", path.display()))?,
        None => DEFAULT_REPLAY_SESSION.to_string(),
    };
    parse_replay_session(&raw)
}

pub fn parse_replay_session(raw: &str) -> Result<DisplaySession, String> {
    let value: Value =
        serde_json::from_str(raw).map_err(|err| format!("invalid replay session json: {err}"))?;
    let session_value = value.get("session").cloned().unwrap_or(value);
    let session: DisplaySession = serde_json::from_value(session_value)
        .map_err(|err| format!("invalid replay session payload: {err}"))?;
    if session.channels.is_empty() && session.effect_parts.is_empty() {
        return Err("replay session requires channels or effectParts".into());
    }
    Ok(session)
}

pub fn post_replay_session(addr: &str, session: &DisplaySession) -> Result<String, String> {
    let body = serde_json::to_string(session)
        .map_err(|err| format!("failed to serialize replay session: {err}"))?;
    let request = build_session_request(addr, &body);
    let mut last_error = String::new();
    for _ in 0..25 {
        match post_request(addr, &request) {
            Ok(response) => return parse_response(response),
            Err(err) => {
                last_error = err.to_string();
                thread::sleep(Duration::from_millis(40));
            }
        }
    }
    Err(format!(
        "failed to post replay session to {addr}: {last_error}"
    ))
}

fn post_request(addr: &str, request: &str) -> std::io::Result<String> {
    let mut stream = TcpStream::connect(addr)?;
    stream.write_all(request.as_bytes())?;
    stream.shutdown(std::net::Shutdown::Write)?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    Ok(response)
}

fn parse_response(response: String) -> Result<String, String> {
    let status_line = response.lines().next().unwrap_or("HTTP/1.1 000 Empty");
    if status_line.contains(" 200 ") {
        return Ok(status_line.to_string());
    }
    let body = response.split("\r\n\r\n").nth(1).unwrap_or("").trim();
    Err(format!("display replay failed: {status_line} {body}"))
}

fn build_session_request(host: &str, body: &str) -> String {
    format!(
        "POST /session HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{route_request, DisplayState};
    use serde_json::json;
    use std::sync::{Arc, RwLock};

    #[test]
    fn bundled_fixture_is_accepted_by_session_route() {
        let session = load_replay_session(None).expect("fixture session");
        let body = serde_json::to_string(&session).expect("session json");
        let request = build_session_request("127.0.0.1:19676", &body);
        let state = Arc::new(RwLock::new(DisplayState::new()));

        let response = route_request(&request, &state);

        assert_eq!(response.status, 200);
        assert_eq!(response.body.get("ok"), Some(&json!(true)));
        assert!(state.read().unwrap().session.is_some());
    }

    #[test]
    fn accepts_wrapped_control_app_export() {
        let raw = json!({
            "exportedAt": "2026-06-27T00:00:00.000Z",
            "session": {
                "sessionId": "exported",
                "sentAt": "2026-06-27T00:00:00.000Z",
                "sourceBaseUrl": "http://127.0.0.1:19576",
                "activeChannelIndex": 0,
                "channels": [{ "name": "main", "parts": ["recolor"], "values": { "color": "#33ccff" } }],
                "effectParts": [{ "id": "recolor", "defaultsOn": { "strength": 0.8 } }],
                "artMeshAliases": { "ArtMesh01": "前发高光" }
            }
        })
        .to_string();

        let session = parse_replay_session(&raw).expect("wrapped export");

        assert_eq!(session.channels.len(), 1);
        assert_eq!(session.effect_parts[0].id, "recolor");
        assert_eq!(
            session
                .art_mesh_aliases
                .get("ArtMesh01")
                .map(String::as_str),
            Some("前发高光")
        );
        let serialized = serde_json::to_value(&session).expect("session value");
        assert_eq!(
            serialized.pointer("/artMeshAliases/ArtMesh01"),
            Some(&json!("前发高光"))
        );
    }

    #[test]
    fn rejects_empty_payload_shape() {
        let err = parse_replay_session("{}").expect_err("empty payload should fail");

        assert_eq!(err, "replay session requires channels or effectParts");
    }
}
