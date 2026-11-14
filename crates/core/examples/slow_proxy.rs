//! 仅在回环接口运行的限速字节代理，用于桌面断点续传联调，不是公网代理。
use anyhow::{ensure, Result};
use std::{net::SocketAddr, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinSet,
};

#[tokio::main]
async fn main() -> Result<()> {
    let target: SocketAddr = std::env::var("FILE_TRANS_PROXY_TARGET")
        .unwrap_or_else(|_| "127.0.0.1:8787".into())
        .parse()?;
    let address: SocketAddr = std::env::var("FILE_TRANS_PROXY_LISTEN")
        .unwrap_or_else(|_| "127.0.0.1:8788".into())
        .parse()?;
    ensure!(
        target.ip().is_loopback() && address.ip().is_loopback(),
        "联调代理只允许回环地址"
    );
    let listener = TcpListener::bind(address).await?;
    println!("本机限速代理 {} -> {target}", listener.local_addr()?);
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            Some(_) = tasks.join_next(), if !tasks.is_empty() => {},
            accepted = listener.accept() => {
                let (mut client, _) = accepted?;
                tasks.spawn(async move {
                    let mut server = TcpStream::connect(target).await?;
                    let (mut a_read, mut a_write) = client.split();
                    let (mut b_read, mut b_write) = server.split();
                    let throttled = async {
                        let mut bytes = vec![0; 32 * 1024];
                        loop {
                            let length = a_read.read(&mut bytes).await?;
                            if length == 0 { break; }
                            tokio::time::sleep(Duration::from_millis(8)).await;
                            b_write.write_all(&bytes[..length]).await?;
                        }
                        Ok::<(), std::io::Error>(())
                    };
                    tokio::select! { result = throttled => result, result = tokio::io::copy(&mut b_read, &mut a_write) => result.map(|_| ()) }
                });
            }
        }
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    Ok(())
}
