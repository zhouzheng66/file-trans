use bytes::Bytes;
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use std::io;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

/// 有界字节流桥接；应用层关闭或网络结束时终止双向泵，不积压整个文件。
pub fn byte_stream<S, W>(mut source: S, mut sink: W) -> DuplexStream
where
    S: Stream<Item = io::Result<Bytes>> + Send + Unpin + 'static,
    W: Sink<Bytes, Error = io::Error> + Send + Unpin + 'static,
{
    let (application, bridge) = tokio::io::duplex(256 * 1024);
    tokio::spawn(async move {
        let (mut reader, mut writer) = tokio::io::split(bridge);
        let inbound = async {
            while let Some(bytes) = source.next().await {
                writer.write_all(&bytes?).await?;
            }
            writer.shutdown().await
        };
        let outbound = async {
            let mut bytes = vec![0; 32 * 1024];
            loop {
                let length = reader.read(&mut bytes).await?;
                if length == 0 {
                    break;
                }
                tokio::time::timeout(
                    crate::protocol::CONNECT_TIMEOUT,
                    sink.send(Bytes::copy_from_slice(&bytes[..length])),
                )
                .await??;
            }
            tokio::time::timeout(crate::protocol::CONNECT_TIMEOUT, sink.close()).await?
        };
        tokio::select! { _ = inbound => {}, _ = outbound => {} }
    });
    application
}
