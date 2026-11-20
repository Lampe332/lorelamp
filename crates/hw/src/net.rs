//! The IPv4 addresses of this PC's network adapters that are up, with the adapter's name
//! ("WLAN", "Ethernet", "Tailscale"), for the "Handy verbinden" window: a phone in the same Wi-Fi
//! reaches the PC under one of them.

use std::net::Ipv4Addr;

/// One address of one adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetAddress {
    pub ip: Ipv4Addr,
    /// The adapter's name as Windows shows it; empty where the platform gives none.
    pub adapter: String,
}

/// Every IPv4 address of an adapter that is up, loopback left out, in the system's order.
/// Empty where the platform is not supported or the call fails.
pub fn ipv4_addresses() -> Vec<NetAddress> {
    imp::ipv4_addresses()
}

#[cfg(not(windows))]
mod imp {
    pub fn ipv4_addresses() -> Vec<super::NetAddress> {
        Vec::new()
    }
}

#[cfg(windows)]
mod imp {
    use std::net::Ipv4Addr;

    use windows::Win32::NetworkManagement::IpHelper::{
        GetAdaptersAddresses, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, IF_TYPE_SOFTWARE_LOOPBACK,
        IP_ADAPTER_ADDRESSES_LH,
    };
    use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN};

    use super::NetAddress;

    const ERROR_BUFFER_OVERFLOW: u32 = 111;

    pub fn ipv4_addresses() -> Vec<NetAddress> {
        let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
        // Microsoft's advice: start with 15 KB and grow when the call asks for more.
        let mut size: u32 = 15 * 1024;
        for _ in 0..3 {
            // u64 elements keep the buffer aligned for the structures written into it.
            let mut buf = vec![0u64; (size as usize).div_ceil(8)];
            let first = buf.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
            // SAFETY: `buf` holds `size` writable bytes, aligned to 8; the call fills it with a
            // linked list that points only into `buf`, which lives until the walk below ends.
            let rc = unsafe { GetAdaptersAddresses(u32::from(AF_INET.0), flags, None, Some(first), &mut size) };
            if rc == ERROR_BUFFER_OVERFLOW {
                continue;
            }
            if rc != 0 {
                return Vec::new();
            }
            let mut out = Vec::new();
            let mut adapter = first.cast_const();
            while !adapter.is_null() {
                // SAFETY: every node of the list was written by the call into `buf`.
                let a = unsafe { &*adapter };
                if a.OperStatus == IfOperStatusUp && a.IfType != IF_TYPE_SOFTWARE_LOOPBACK {
                    // SAFETY: a PWSTR from the call, null-terminated, or null.
                    let name = if a.FriendlyName.is_null() { String::new() } else { unsafe { a.FriendlyName.to_string() }.unwrap_or_default() };
                    let mut unicast = a.FirstUnicastAddress.cast_const();
                    while !unicast.is_null() {
                        // SAFETY: a node of the same list.
                        let u = unsafe { &*unicast };
                        let sa = u.Address.lpSockaddr;
                        // SAFETY: `lpSockaddr` points to `iSockaddrLength` bytes; an AF_INET one is a SOCKADDR_IN.
                        if !sa.is_null() && unsafe { (*sa).sa_family } == AF_INET && u.Address.iSockaddrLength as usize >= size_of::<SOCKADDR_IN>() {
                            let sin = unsafe { &*sa.cast::<SOCKADDR_IN>() };
                            // SAFETY: every variant of the union is the same four bytes.
                            let raw = unsafe { sin.sin_addr.S_un.S_addr };
                            let ip = Ipv4Addr::from(u32::from_be(raw));
                            if !ip.is_loopback() && !ip.is_unspecified() {
                                out.push(NetAddress { ip, adapter: name.clone() });
                            }
                        }
                        unicast = u.Next.cast_const();
                    }
                }
                adapter = a.Next.cast_const();
            }
            return out;
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn lists_no_loopback() {
        // The test PC's adapters differ; only what must hold everywhere is checked.
        for a in super::ipv4_addresses() {
            assert!(!a.ip.is_loopback() && !a.ip.is_unspecified(), "{a:?}");
        }
    }
}
