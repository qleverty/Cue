#[cfg(target_os = "windows")]
pub mod imp {
    use std::io;
    use std::net::TcpListener;
    use std::os::windows::io::FromRawSocket;

    type RawSocket = usize;
    const INVALID_SOCKET:      RawSocket = usize::MAX;
    const SOCKET_ERROR:        i32 = -1;
    const AF_INET:             i32 = 2;
    const SOCK_STREAM:         i32 = 1;
    const IPPROTO_TCP:         i32 = 6;
    const SOL_SOCKET:          i32 = 0xffff;
    const SO_EXCLUSIVEADDRUSE: i32 = -5;

    #[repr(C)]
    struct WsaData {
        w_version:        u16,
        w_high_version:   u16,
        sz_description:   [u8; 257],
        sz_system_status: [u8; 129],
        i_max_sockets:    u16,
        i_max_udp_dg:     u16,
        lp_vendor_info:   *mut u8,
    }

    #[repr(C)]
    struct SockAddrIn {
        sin_family: i16,
        sin_port:   u16,
        sin_addr:   u32,
        sin_zero:   [u8; 8],
    }

    #[link(name = "ws2_32")]
    unsafe extern "system" {
        fn WSAStartup(version: u16, data: *mut WsaData) -> i32;
        fn socket(af: i32, ty: i32, proto: i32) -> RawSocket;
        fn setsockopt(s: RawSocket, level: i32, name: i32, val: *const u8, len: i32) -> i32;
        fn bind(s: RawSocket, addr: *const SockAddrIn, len: i32) -> i32;
        fn listen(s: RawSocket, backlog: i32) -> i32;
        fn closesocket(s: RawSocket) -> i32;
        fn htons(x: u16) -> u16;
    }

    pub fn bind_exclusive(port: u16) -> io::Result<TcpListener> {
        unsafe {
            let mut wsa: WsaData = std::mem::zeroed();
            WSAStartup(0x0202, &mut wsa);

            let s = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
            if s == INVALID_SOCKET {
                return Err(io::Error::last_os_error());
            }

            let val: i32 = 1;
            setsockopt(
                s, SOL_SOCKET, SO_EXCLUSIVEADDRUSE,
                &val as *const i32 as *const u8, std::mem::size_of::<i32>() as i32,
            );

            let addr = SockAddrIn {
                sin_family: AF_INET as i16,
                sin_port:   htons(port),
                sin_addr:   0, // INADDR_ANY
                sin_zero:   [0; 8],
            };
            if bind(s, &addr, std::mem::size_of::<SockAddrIn>() as i32) == SOCKET_ERROR {
                let e = io::Error::last_os_error();
                closesocket(s);
                return Err(e);
            }
            if listen(s, 128) == SOCKET_ERROR {
                let e = io::Error::last_os_error();
                closesocket(s);
                return Err(e);
            }

            Ok(TcpListener::from_raw_socket(s as _))
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub mod imp {
    use std::io;
    use std::net::TcpListener;

    pub fn bind_exclusive(port: u16) -> io::Result<TcpListener> {
        TcpListener::bind(("0.0.0.0", port))
    }
}

pub use imp::bind_exclusive;
