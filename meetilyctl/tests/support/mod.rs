use meetilyctl::gateway::{Credentials, Gateway};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

pub struct Fixture {
    #[allow(dead_code)]
    pub file: tempfile::NamedTempFile,
    #[allow(dead_code)]
    pub gateway: Gateway,
    pub calls: Arc<Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
pub async fn fixture(
    response: impl Fn(&str, &Value) -> Value + Send + Sync + 'static,
    control: bool,
) -> Fixture {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let credentials = Credentials {
        port,
        read_token: "read-secret".into(),
        control_token: control.then(|| "control-secret".into()),
    };
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        file.path(),
        json!({"port":port,"read_token":"read-secret","control_token":credentials.control_token})
            .to_string(),
    )
    .unwrap();
    let gateway = Gateway::new(credentials).unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let captured = calls.clone();
    let response = Arc::new(response);
    let task = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let captured = captured.clone();
            let response = response.clone();
            tokio::spawn(async move {
                let mut data = Vec::new();
                let mut buf = [0u8; 4096];
                let header_end = loop {
                    let n = socket.read(&mut buf).await.unwrap();
                    if n == 0 {
                        return;
                    }
                    data.extend_from_slice(&buf[..n]);
                    if let Some(at) = data.windows(4).position(|p| p == b"\r\n\r\n") {
                        break at + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&data[..header_end]);
                assert!(headers.starts_with("POST /v1/rpc HTTP/1.1"));
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_lowercase()
                            .strip_prefix("content-length: ")
                            .and_then(|n| n.parse::<usize>().ok())
                    })
                    .unwrap();
                let authorization = headers
                    .lines()
                    .find(|line| line.to_lowercase().starts_with("authorization:"))
                    .unwrap()
                    .to_owned();
                while data.len() < header_end + length {
                    let n = socket.read(&mut buf).await.unwrap();
                    if n == 0 {
                        return;
                    }
                    data.extend_from_slice(&buf[..n]);
                }
                let mut request: Value =
                    serde_json::from_slice(&data[header_end..header_end + length]).unwrap();
                request["authorization"] = json!(authorization);
                captured.lock().unwrap().push(request.clone());
                let body =
                    response(request["method"].as_str().unwrap(), &request["params"]).to_string();
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).as_bytes()).await.unwrap();
            });
        }
    });
    Fixture {
        file,
        gateway,
        calls,
        task,
    }
}
