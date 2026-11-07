//! Advertising the phone link as `_cmdtrigger._tcp` so the iPhone app finds this computer.
//!
//! On macOS this goes through the system's mDNSResponder (`DNSServiceRegister`), not our own
//! responder. A Mac that sleeps with "Wake for network access" on then hands the record to a
//! Bonjour Sleep Proxy (an Apple TV or HomePod) when there is one, which keeps answering for the
//! Mac and wakes it when the phone connects. Elsewhere it uses mdns-sd, like the HomeKit bridge.

pub const SERVICE_TYPE: &str = "_cmdtrigger._tcp";

pub struct Txt<'a> {
    pub id: &'a str,
    pub ips: &'a str,
    pub port: u16,
}

pub struct Advertiser {
    /// Kept only to stay registered; dropping it deregisters.
    #[cfg(target_os = "macos")]
    _registration: macos::Registration,
    #[cfg(not(target_os = "macos"))]
    inner: (mdns_sd::ServiceDaemon, String),
}

impl Advertiser {
    pub fn publish(name: &str, txt: &Txt) -> Result<Self, String> {
        let mut instance: String = name.chars().filter(|c| *c != '.').collect();
        while instance.len() > 63 {
            instance.pop();
        }
        if instance.trim().is_empty() {
            instance = "CmdTrigger".into();
        }
        let port = txt.port.to_string();
        let pairs = [("id", txt.id), ("v", "1"), ("ip", txt.ips), ("port", port.as_str())];
        #[cfg(target_os = "macos")]
        {
            Ok(Self { _registration: macos::Registration::new(&instance, SERVICE_TYPE, txt.port, &pairs)? })
        }
        #[cfg(not(target_os = "macos"))]
        {
            let daemon = mdns_sd::ServiceDaemon::new().map_err(|e| e.to_string())?;
            let host = format!("cmdtrigger-{}.local.", txt.id);
            let info = mdns_sd::ServiceInfo::new(
                &format!("{SERVICE_TYPE}.local."),
                &instance,
                &host,
                "",
                txt.port,
                &pairs[..],
            )
            .map_err(|e| e.to_string())?
            .enable_addr_auto();
            let fullname = info.get_fullname().to_string();
            daemon.register(info).map_err(|e| e.to_string())?;
            Ok(Self { inner: (daemon, fullname) })
        }
    }
}

#[cfg(not(target_os = "macos"))]
impl Drop for Advertiser {
    fn drop(&mut self) {
        let _ = self.inner.0.unregister(&self.inner.1);
        let _ = self.inner.0.shutdown();
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::{CString, c_char, c_void};

    type DnsServiceRef = *mut c_void;
    type RegisterReply =
        extern "C" fn(DnsServiceRef, u32, i32, *const c_char, *const c_char, *const c_char, *mut c_void);

    // dns_sd.h; the functions live in libSystem.
    unsafe extern "C" {
        fn DNSServiceRegister(
            sd_ref: *mut DnsServiceRef,
            flags: u32,
            interface_index: u32,
            name: *const c_char,
            regtype: *const c_char,
            domain: *const c_char,
            host: *const c_char,
            port_network_order: u16,
            txt_len: u16,
            txt_record: *const c_void,
            callback: Option<RegisterReply>,
            context: *mut c_void,
        ) -> i32;
        fn DNSServiceRefSockFD(sd_ref: DnsServiceRef) -> i32;
        fn DNSServiceProcessResult(sd_ref: DnsServiceRef) -> i32;
        fn DNSServiceRefDeallocate(sd_ref: DnsServiceRef);
    }

    extern "C" fn registered(
        _sd: DnsServiceRef,
        _flags: u32,
        error: i32,
        name: *const c_char,
        regtype: *const c_char,
        _domain: *const c_char,
        _context: *mut c_void,
    ) {
        let text = |p: *const c_char| {
            if p.is_null() {
                String::new()
            } else {
                unsafe { std::ffi::CStr::from_ptr(p) }.to_string_lossy().into_owned()
            }
        };
        if error == 0 {
            tracing::info!("Advertising {} as \"{}\" over Bonjour", text(regtype), text(name));
        } else {
            tracing::warn!("Bonjour registration failed: error {error}");
        }
    }

    /// One registered service; deregistered when dropped.
    pub struct Registration(DnsServiceRef);

    // The ref is only used by `new` (one thread) and `drop`.
    unsafe impl Send for Registration {}
    unsafe impl Sync for Registration {}

    /// A DNS TXT record: each `key=value` prefixed by its length.
    fn txt_record(pairs: &[(&str, &str)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (k, v) in pairs {
            let entry = format!("{k}={v}");
            let bytes = &entry.as_bytes()[..entry.len().min(255)];
            out.push(bytes.len() as u8);
            out.extend_from_slice(bytes);
        }
        out
    }

    impl Registration {
        pub fn new(name: &str, regtype: &str, port: u16, pairs: &[(&str, &str)]) -> Result<Self, String> {
            let name = CString::new(name).map_err(|e| e.to_string())?;
            let regtype = CString::new(regtype).map_err(|e| e.to_string())?;
            let txt = txt_record(pairs);
            let mut sd: DnsServiceRef = std::ptr::null_mut();
            let error = unsafe {
                DNSServiceRegister(
                    &mut sd,
                    0,
                    0,
                    name.as_ptr(),
                    regtype.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    port.to_be(),
                    txt.len() as u16,
                    txt.as_ptr().cast(),
                    Some(registered),
                    std::ptr::null_mut(),
                )
            };
            if error != 0 || sd.is_null() {
                return Err(format!("DNSServiceRegister failed: {error}"));
            }
            // Read the daemon's answer once, to log it.
            let fd = unsafe { DNSServiceRefSockFD(sd) };
            if fd >= 0 {
                let mut poll = PollFd { fd, events: 1, revents: 0 };
                if unsafe { poll_fn(&mut poll, 1, 3000) } > 0 {
                    unsafe { DNSServiceProcessResult(sd) };
                }
            }
            Ok(Self(sd))
        }
    }

    impl Drop for Registration {
        fn drop(&mut self) {
            unsafe { DNSServiceRefDeallocate(self.0) };
        }
    }

    #[repr(C)]
    struct PollFd {
        fd: i32,
        events: i16,
        revents: i16,
    }

    unsafe extern "C" {
        #[link_name = "poll"]
        fn poll_fn(fds: *mut PollFd, nfds: u32, timeout: i32) -> i32;
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn txt_records_are_length_prefixed() {
            assert_eq!(super::txt_record(&[("id", "ab"), ("v", "1")]), b"\x05id=ab\x03v=1");
        }
    }
}
