//! 仅监听回环地址的桌面联调节点：自动确认测试配对和文件接收，不用于生产。
use anyhow::Result;
use file_trans_core::{Engine, EngineConfig};
use file_trans_identity::Identity;
use std::{collections::HashSet, time::Duration};

#[tokio::main]
async fn main() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let mut config = EngineConfig::new(
        directory.path().join("data"),
        directory.path().join("received"),
        "本机验收节点".into(),
    );
    config.listen_address = "127.0.0.1:0".parse()?;
    config.enable_discovery = false;
    let engine = Engine::start(config, Identity::generate()?).await?;
    let sample = directory.path().join("验收样例.bin");
    tokio::fs::write(&sample, [0, 1, 2, 128, 254, 255]).await?;
    println!(
        "仅供本机联调；自动接受回环连接的测试请求。\n地址：{}\n接收目录：{}",
        engine.local_address(),
        engine.snapshot()?.download_dir
    );
    let mut greeted = HashSet::new();
    let mut logged = HashSet::new();
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = tokio::time::sleep(Duration::from_millis(200)) => {},
        }
        let snapshot = engine.snapshot()?;
        for pairing in snapshot
            .pairings
            .iter()
            .filter(|entry| entry.status == "pending")
        {
            println!("配对校验码：{}（请在桌面端核对）", pairing.code);
            engine.decide_pairing(&pairing.id, true)?;
        }
        for activity in &snapshot.activities {
            if activity.direction == "incoming" && activity.status == "awaiting_accept" {
                engine.decide_file(&activity.id, true)?;
            }
            if activity.direction == "incoming"
                && activity.status == "completed"
                && logged.insert(activity.id.clone())
            {
                println!(
                    "已收到 {}：{}，{} 字节",
                    activity.kind, activity.content, activity.size
                );
            }
        }
        for peer in &snapshot.peers {
            if greeted.insert(peer.id.clone()) {
                engine
                    .send_text(&peer.id, "局域网文字传输已连通。\n你好，轻传！🖥️".into())
                    .await?;
                engine.send_file(&peer.id, sample.clone()).await?;
            }
        }
    }
    engine.shutdown().await;
    Ok(())
}
