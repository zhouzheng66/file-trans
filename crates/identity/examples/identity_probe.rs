//! 仅输出设备指纹，用于 CI 验证系统凭据库跨进程读取，不输出私钥。
fn main() -> anyhow::Result<()> {
    let directory = std::env::args_os()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("请指定独立验收数据目录"))?;
    let identity = file_trans_identity::Identity::load_or_create(std::path::Path::new(&directory))?;
    println!("{}", identity.id);
    Ok(())
}
