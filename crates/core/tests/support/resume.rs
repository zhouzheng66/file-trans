#![allow(dead_code)]
use anyhow::Result;
use file_trans_core::{
    model::{Activity, NetworkMode, NetworkSettings},
    transfers::{Direction, TransferKey},
    Engine, EngineConfig,
};
use file_trans_identity::Identity;
use std::{
    net::SocketAddr,
    path::Path,
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::{JoinHandle, JoinSet},
    time::{sleep, timeout},
};
use tokio_util::sync::CancellationToken;

pub async fn node(
    root: &Path,
    name: &str,
    identity: Identity,
    relay: &str,
    address: Option<SocketAddr>,
) -> Arc<Engine> {
    let mut config = EngineConfig::new(
        root.join(name),
        root.join(name).join("received"),
        name.into(),
    );
    config.listen_address = address.unwrap_or("127.0.0.1:0".parse().unwrap());
    config.enable_discovery = false;
    config.decision_timeout = Duration::from_secs(10);
    config.network = Some(NetworkSettings {
        mode: NetworkMode::Auto,
        relay_url: relay.into(),
    });
    Engine::start(config, identity).await.unwrap()
}

pub async fn eventually(mut condition: impl FnMut() -> bool) {
    timeout(Duration::from_secs(25), async {
        while !condition() {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("condition timed out");
}

pub async fn pair(a: &Arc<Engine>, b: &Arc<Engine>) -> Result<()> {
    let id = a.request_pair(b.local_address()).await?;
    a.decide_pairing(&id, true)?;
    b.decide_pairing(&id, true)?;
    eventually(|| a.snapshot().unwrap().peers.len() == 1 && b.snapshot().unwrap().peers.len() == 1)
        .await;
    Ok(())
}

pub fn activity(engine: &Engine, id: &str, direction: &str) -> Activity {
    engine
        .snapshot()
        .unwrap()
        .activities
        .into_iter()
        .find(|entry| entry.id == id && entry.direction == direction)
        .expect("missing activity")
}

pub fn key(engine: &Engine, peer: &Engine, id: &str, direction: Direction) -> TransferKey {
    let _ = engine;
    TransferKey {
        id: id.into(),
        peer_id: peer.local().id.clone(),
        direction,
    }
}

pub fn remember(root: &Path, name: &str, peer: &Engine, address: SocketAddr) {
    rusqlite::Connection::open(root.join(name).join("file-trans.db"))
        .unwrap()
        .execute(
            "UPDATE peers SET address=?1 WHERE id=?2",
            rusqlite::params![address.to_string(), peer.local().id],
        )
        .unwrap();
}

pub async fn write_file(path: &Path, size: usize) -> Result<()> {
    let mut file = tokio::fs::File::create(path).await?;
    let mut data = vec![0; 256 * 1024];
    let mut position = 0;
    while position < size {
        let length = (size - position).min(data.len());
        for (index, value) in data[..length].iter_mut().enumerate() {
            *value = ((position + index) % 251) as u8;
        }
        file.write_all(&data[..length]).await?;
        position += length;
    }
    file.flush().await?;
    Ok(())
}

pub async fn offer(a: &Arc<Engine>, b: &Engine, path: &Path) -> Result<String> {
    let id = a.send_file(&b.local().id, path.into()).await?;
    eventually(|| {
        b.snapshot()
            .unwrap()
            .activities
            .iter()
            .any(|entry| entry.id == id && entry.status == "awaiting_accept")
    })
    .await;
    b.decide_file(&id, true)?;
    Ok(id)
}

pub async fn finished(a: &Engine, b: &Engine, id: &str, original: &Path) -> Result<()> {
    eventually(|| {
        ["completed", "failed", "rejected"].contains(&activity(a, id, "outgoing").status.as_str())
    })
    .await;
    let sent = activity(a, id, "outgoing");
    assert_eq!(sent.status, "completed", "{:?}", sent.error);
    let received = activity(b, id, "incoming");
    assert_eq!(received.status, "completed", "{:?}", received.error);
    assert_eq!(
        sha(original).await?,
        sha(Path::new(received.path.as_ref().unwrap())).await?
    );
    Ok(())
}

async fn sha(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = tokio::fs::File::open(path).await?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 256 * 1024];
    loop {
        let length = file.read(&mut buffer).await?;
        if length == 0 {
            break;
        }
        hash.update(&buffer[..length]);
    }
    Ok(hex::encode(hash.finalize()))
}

pub struct Proxy {
    pub address: SocketAddr,
    pub bytes: Arc<AtomicU64>,
    pub active: Arc<AtomicUsize>,
    cancel: CancellationToken,
    task: JoinHandle<()>,
}
impl Proxy {
    pub async fn start(target: SocketAddr) -> Self {
        Self::on("127.0.0.1:0".parse().unwrap(), target).await
    }
    pub async fn on(address: SocketAddr, target: SocketAddr) -> Self {
        let listener = TcpListener::bind(address).await.unwrap();
        let address = listener.local_addr().unwrap();
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let bytes = Arc::new(AtomicU64::new(0));
        let output = bytes.clone();
        let active = Arc::new(AtomicUsize::new(0));
        let counter = active.clone();
        let task = tokio::spawn(async move {
            let mut tasks = JoinSet::new();
            loop {
                tokio::select! {
                    _ = token.cancelled() => break,
                    Some(_) = tasks.join_next(), if !tasks.is_empty() => {},
                    accepted = listener.accept() => {
                        let Ok((mut client, _)) = accepted else { break };
                        let bytes = output.clone(); let token = token.clone();
                        counter.fetch_add(1, Ordering::SeqCst); let guard = Active(counter.clone());
                        tasks.spawn(async move {
                            let _guard = guard;
                            let Ok(mut server) = TcpStream::connect(target).await else { return };
                            let _ = client.set_nodelay(true); let _ = server.set_nodelay(true);
                            let (mut a_read, mut a_write) = client.split(); let (mut b_read, mut b_write) = server.split();
                            let forward = async {
                                let mut buffer = vec![0; 32 * 1024];
                                loop {
                                    let length = a_read.read(&mut buffer).await?;
                                    if length == 0 { break; }
                                    sleep(Duration::from_millis(4)).await;
                                    b_write.write_all(&buffer[..length]).await?;
                                    bytes.fetch_add(length as u64, Ordering::SeqCst);
                                }
                                Ok::<(), std::io::Error>(())
                            };
                            tokio::select! { _ = token.cancelled() => {}, _ = forward => {}, _ = tokio::io::copy(&mut b_read, &mut a_write) => {} }
                        });
                    }
                }
            }
            while tasks.join_next().await.is_some() {}
        });
        Self {
            address,
            bytes,
            active,
            cancel,
            task,
        }
    }
    pub async fn stop(self) {
        self.cancel.cancel();
        self.task.await.unwrap();
    }
}
struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct Service {
    pub address: SocketAddr,
    pub url: String,
    cancel: CancellationToken,
    task: JoinHandle<Result<()>>,
}
impl Service {
    pub async fn start() -> Self {
        Self::on("127.0.0.1:0".parse().unwrap()).await
    }
    pub async fn on(address: SocketAddr) -> Self {
        let listener = TcpListener::bind(address).await.unwrap();
        let address = listener.local_addr().unwrap();
        let cancel = CancellationToken::new();
        let task = tokio::spawn(file_trans_relay_server::serve(
            listener,
            Default::default(),
            cancel.clone(),
        ));
        Self {
            address,
            url: format!("ws://{address}"),
            cancel,
            task,
        }
    }
    pub async fn stop(self) {
        self.cancel.cancel();
        timeout(Duration::from_secs(10), self.task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
}
