//! 通过 macOS 的 mDNSResponder 解析已知设备，独立于原始 UDP 组播套接字。
//! FFI 签名对应系统 dns_sd.h；引用及回调上下文由 Resolver 持有，取消 future 时一并释放。
use super::{clean_name, DiscoveredDevice};
use anyhow::{bail, ensure, Context, Result};
use std::{
    ffi::{c_char, c_void, CStr, CString},
    os::fd::{AsFd, AsRawFd, BorrowedFd, RawFd},
    ptr,
    time::Duration,
};
use tokio::{io::unix::AsyncFd, net::lookup_host, time::timeout};

#[link(name = "System")]
extern "C" {
    fn DNSServiceResolve(
        service: *mut *mut c_void,
        flags: u32,
        interface: u32,
        name: *const c_char,
        regtype: *const c_char,
        domain: *const c_char,
        callback: extern "C" fn(
            *mut c_void,
            u32,
            u32,
            i32,
            *const c_char,
            *const c_char,
            u16,
            u16,
            *const u8,
            *mut c_void,
        ),
        context: *mut c_void,
    ) -> i32;
    fn DNSServiceRefSockFD(service: *mut c_void) -> i32;
    fn DNSServiceProcessResult(service: *mut c_void) -> i32;
    fn DNSServiceRefDeallocate(service: *mut c_void);
}

struct ServiceRef(*mut c_void);
// 每个引用只被所属 future 顺序访问；Tokio 可以在线程间移动该 future。
unsafe impl Send for ServiceRef {}
unsafe impl Sync for ServiceRef {}
impl Drop for ServiceRef {
    fn drop(&mut self) {
        unsafe { DNSServiceRefDeallocate(self.0) };
    }
}

struct Resolved {
    host: String,
    port: u16,
    name: String,
}
struct State {
    id: String,
    result: Option<Result<Resolved>>,
}
struct Resolver {
    service: ServiceRef,
    // Box 保证回调上下文的地址不随 future 移动；字段顺序保证先停止回调再销毁上下文。
    state: Box<State>,
    fd: RawFd,
}
impl AsRawFd for Resolver {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}
impl AsFd for Resolver {
    fn as_fd(&self) -> BorrowedFd<'_> {
        unsafe { BorrowedFd::borrow_raw(self.fd) }
    }
}
impl Resolver {
    fn start(id: &str) -> Result<Self> {
        let name = CString::new(&id[..24])?;
        let regtype = CString::new("_filetrans._tcp")?;
        let domain = CString::new("local.")?;
        let mut state = Box::new(State {
            id: id.into(),
            result: None,
        });
        let mut service = ptr::null_mut();
        let error = unsafe {
            DNSServiceResolve(
                &mut service,
                0,
                0,
                name.as_ptr(),
                regtype.as_ptr(),
                domain.as_ptr(),
                resolved,
                (&mut *state as *mut State).cast(),
            )
        };
        check(error)?;
        let service = ServiceRef(service);
        let fd = unsafe { DNSServiceRefSockFD(service.0) };
        ensure!(fd >= 0, "无法取得 Bonjour 解析套接字");
        Ok(Self { service, state, fd })
    }
}

extern "C" fn resolved(
    _service: *mut c_void,
    _flags: u32,
    _interface: u32,
    error: i32,
    _fullname: *const c_char,
    host: *const c_char,
    port: u16,
    txt_len: u16,
    txt: *const u8,
    context: *mut c_void,
) {
    // dns_sd 保证指针在回调期间有效；错误时其余参数未定义，必须先检查错误。
    let state = unsafe { &mut *context.cast::<State>() };
    state.result = Some((|| {
        check(error)?;
        ensure!(
            !host.is_null() && !txt.is_null(),
            "Bonjour 返回了无效的解析结果"
        );
        let host = unsafe { CStr::from_ptr(host) }.to_str()?.to_owned();
        let txt = unsafe { std::slice::from_raw_parts(txt, usize::from(txt_len)) };
        ensure!(
            property(txt, "id") == Some(state.id.as_str()) && property(txt, "version") == Some("1"),
            "Bonjour 设备标识或版本不匹配"
        );
        let port = u16::from_be(port);
        ensure!(
            host.ends_with(".local.") && port != 0,
            "Bonjour 返回了无效的本地地址"
        );
        Ok(Resolved {
            host,
            port,
            name: clean_name(property(txt, "name").unwrap_or("未命名设备")),
        })
    })());
}

fn check(error: i32) -> Result<()> {
    if error == -65570 {
        bail!("本地网络访问被 macOS 拒绝，请在系统设置 → 隐私与安全性 → 本地网络中允许 File Trans");
    }
    ensure!(error == 0, "系统 Bonjour 解析失败（{error}）");
    Ok(())
}

fn property<'a>(mut txt: &'a [u8], key: &str) -> Option<&'a str> {
    while let Some((&length, remaining)) = txt.split_first() {
        let entry = remaining.get(..usize::from(length))?;
        txt = &remaining[usize::from(length)..];
        let Some((name, value)) = std::str::from_utf8(entry).ok()?.split_once('=') else {
            continue;
        };
        if name.eq_ignore_ascii_case(key) {
            return Some(value);
        }
    }
    None
}

pub(super) async fn resolve_peer(id: &str) -> Result<DiscoveredDevice> {
    timeout(Duration::from_millis(2500), async {
        // Resolver 在 AsyncFd 注销前持有并保持该描述符有效。
        let mut resolver = unsafe { AsyncFd::register(Resolver::start(id)?) }
            .map_err(|error| anyhow::anyhow!("注册 Bonjour 解析套接字失败：{error}"))?;
        let result = loop {
            let mut ready = resolver.readable_mut().await?;
            check(unsafe { DNSServiceProcessResult(ready.get_inner_mut().service.0) })?;
            ready.clear_ready();
            if let Some(result) = ready.get_inner_mut().state.result.take() {
                break result?;
            }
        };
        let mut addresses: Vec<_> = lookup_host((result.host.as_str(), result.port))
            .await?
            .filter(|address| address.is_ipv4())
            .map(|address| address.to_string())
            .collect();
        addresses.sort();
        addresses.dedup();
        ensure!(!addresses.is_empty(), "系统 Bonjour 未返回可用的 IPv4 地址");
        Ok(DiscoveredDevice {
            id: id.into(),
            name: result.name,
            addresses,
        })
    })
    .await
    .context("系统 Bonjour 等待设备响应超时")?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_txt_properties_and_rejects_truncated_records() {
        assert_eq!(property(b"\x04id=a\x09version=1", "VERSION"), Some("1"));
        assert_eq!(property(b"\x05id=a", "id"), None);
        assert_eq!(property(b"\x04flag\x04id=b", "id"), Some("b"));
    }
}
