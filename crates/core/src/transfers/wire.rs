use anyhow::Result;
use file_trans_identity::TlsStream;
use file_trans_protocol::{read_message, write_message, Message};
use std::{future::Future, time::Duration};

pub(crate) async fn read(stream: &mut TlsStream) -> Result<Message> {
    loop {
        let message =
            tokio::time::timeout(crate::engine::IO_TIMEOUT, read_message(stream)).await??;
        if !matches!(message, Message::TransferKeepAlive) {
            return Ok(message);
        }
    }
}

/// 磁盘任务不可因心跳发送失败而取消；让已经开始的写入/同步完成再释放接收锁。
pub(crate) async fn during<T>(
    stream: &mut TlsStream,
    work: impl Future<Output = Result<T>>,
) -> Result<T> {
    tokio::pin!(work);
    let mut heartbeat = tokio::time::interval(Duration::from_secs(8));
    let mut connected = true;
    loop {
        tokio::select! {
            result = &mut work => return result,
            _ = heartbeat.tick(), if connected => {
                connected = matches!(tokio::time::timeout(Duration::from_secs(3), write_message(stream, &Message::TransferKeepAlive)).await, Ok(Ok(())));
            }
        }
    }
}
