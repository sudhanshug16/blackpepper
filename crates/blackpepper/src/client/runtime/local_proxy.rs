use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// A client-owned loopback proxy for a local listener bound only to another
/// interface. Existing loopback/wildcard listeners need no proxy.
pub(super) struct LocalPortProxy {
    stop: Arc<AtomicBool>,
    accept_thread: Option<JoinHandle<()>>,
}

impl LocalPortProxy {
    pub(super) fn start(bind: SocketAddr, target: SocketAddr) -> io::Result<Self> {
        let listener = TcpListener::bind(bind)?;
        Self::from_listener(listener, target)
    }

    fn from_listener(listener: TcpListener, target: SocketAddr) -> io::Result<Self> {
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let accept_thread = thread::spawn(move || {
            while !thread_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((client, _)) => {
                        let connection_stop = Arc::clone(&thread_stop);
                        thread::spawn(move || proxy_connection(client, target, connection_stop));
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(20));
                    }
                    Err(_) => return,
                }
            }
        });
        Ok(Self {
            stop,
            accept_thread: Some(accept_thread),
        })
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.accept_thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for LocalPortProxy {
    fn drop(&mut self) {
        self.stop();
    }
}

fn proxy_connection(mut client: TcpStream, target: SocketAddr, stop: Arc<AtomicBool>) {
    let Ok(mut upstream) = TcpStream::connect_timeout(&target, Duration::from_secs(2)) else {
        let _ = client.shutdown(Shutdown::Both);
        return;
    };
    let Ok(mut client_reader) = client.try_clone() else {
        return;
    };
    let Ok(mut upstream_reader) = upstream.try_clone() else {
        return;
    };
    let left_stop = Arc::clone(&stop);
    let left =
        thread::spawn(move || copy_until_stopped(&mut client_reader, &mut upstream, &left_stop));
    let right = thread::spawn(move || copy_until_stopped(&mut upstream_reader, &mut client, &stop));
    let _ = left.join();
    let _ = right.join();
}

fn copy_until_stopped(reader: &mut TcpStream, writer: &mut TcpStream, stop: &AtomicBool) {
    let _ = reader.set_read_timeout(Some(Duration::from_millis(100)));
    let mut buffer = [0_u8; 16 * 1024];
    while !stop.load(Ordering::Acquire) {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(size) if writer.write_all(&buffer[..size]).is_err() => break,
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break,
        }
    }
    let _ = writer.shutdown(Shutdown::Both);
}

pub(super) type LocalPortProxies = BTreeMap<SocketAddr, LocalPortProxy>;

pub(super) fn target_socket(host: &str, port: u16) -> Result<SocketAddr, String> {
    let address = host.parse::<IpAddr>().map_err(|_| {
        format!("Local forwarding does not support scoped listener address '{host}'.")
    })?;
    Ok(SocketAddr::new(address, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_relays_bytes_and_stops_owning_its_loopback_port() {
        let upstream = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let target = upstream.local_addr().unwrap();
        let bind = SocketAddr::new(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0);
        let listener = TcpListener::bind(bind).unwrap();
        let proxy_address = listener.local_addr().unwrap();
        // Hand the already-bound ephemeral socket to the proxy. Releasing it
        // and rebinding by number lets an unrelated parallel test win the
        // port between those two operations.
        let mut proxy = LocalPortProxy::from_listener(listener, target).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = upstream.accept().unwrap();
            let mut request = [0_u8; 4];
            stream.read_exact(&mut request).unwrap();
            assert_eq!(&request, b"ping");
            stream.write_all(b"pong").unwrap();
        });

        let mut client = TcpStream::connect(proxy_address).unwrap();
        client.write_all(b"ping").unwrap();
        let mut response = [0_u8; 4];
        client.read_exact(&mut response).unwrap();
        assert_eq!(&response, b"pong");
        server.join().unwrap();

        drop(client);
        proxy.stop();
        TcpListener::bind(proxy_address).unwrap();
    }
}
