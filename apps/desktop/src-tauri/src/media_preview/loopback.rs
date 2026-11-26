use super::validate_media;
use axum::{
    extract::{Path, Request, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use file_trans_core::{
    transfers::{Direction, TransferKey},
    Engine,
};
use ring::{hmac, rand::SystemRandom};
use std::{
    net::{Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::Arc,
};
use tokio::{net::TcpListener, sync::OnceCell};
use tower_http::services::ServeFile;

// WebKitGTK 的视频管线需要 HTTP 媒体地址。首次预览视频时才监听本机随机端口。
static SERVER: OnceCell<Endpoint> = OnceCell::const_new();

struct Endpoint {
    address: SocketAddr,
    signing_key: hmac::Key,
}

pub(super) async fn video_url(engine: Arc<Engine>, key: &TransferKey) -> Result<String, String> {
    let endpoint = SERVER.get_or_try_init(|| start(engine)).await?;
    let signature = hmac::sign(&endpoint.signing_key, key.token().as_bytes());
    Ok(format!(
        "http://{}/{}/{}/{}/{}",
        endpoint.address,
        hex::encode(signature),
        key.direction.as_str(),
        key.peer_id,
        key.id
    ))
}

async fn start(engine: Arc<Engine>) -> Result<Endpoint, String> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|error| format!("无法启动本地视频预览：{error}"))?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    let signing_key = hmac::Key::generate(hmac::HMAC_SHA256, &SystemRandom::new())
        .map_err(|_| "无法创建本地视频预览密钥")?;
    let router = Router::new()
        .route("/{signature}/{direction}/{peer}/{id}", get(video))
        .with_state((engine, signing_key.clone()));
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    Ok(Endpoint {
        address,
        signing_key,
    })
}

async fn video(
    State((engine, signing_key)): State<(Arc<Engine>, hmac::Key)>,
    Path((signature, direction, peer_id, id)): Path<(String, Direction, String, String)>,
    request: Request,
) -> Result<Response, StatusCode> {
    let key = TransferKey {
        direction,
        peer_id,
        id,
    };
    let signature = hex::decode(signature).map_err(|_| StatusCode::NOT_FOUND)?;
    hmac::verify(&signing_key, key.token().as_bytes(), &signature)
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let (path, mime) = tokio::task::spawn_blocking(move || {
        let path = engine.local_attachment_path(&key).ok()?;
        let mime = validate_media(&path).ok()?;
        mime.starts_with("video/").then_some((path, mime))
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .ok_or(StatusCode::NOT_FOUND)?;
    stream_file(path, mime, request).await
}

async fn stream_file(path: PathBuf, mime: &str, request: Request) -> Result<Response, StatusCode> {
    let mime = mime
        .parse()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut response = ServeFile::new_with_mime(path, &mime)
        .try_call(request)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?
        .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}

#[cfg(test)]
#[path = "loopback_tests.rs"]
mod tests;
