use std::{io, net::TcpStream};

pub fn require_server_peer(stream: &TcpStream) -> io::Result<()> {
    let local = stream.local_addr()?;
    let peer = stream.peer_addr()?;
    if local == peer {
        return Err(io::Error::new(
            io::ErrorKind::ConnectionRefused,
            format!("TCP self-connection at {local} is not a server connection"),
        ));
    }
    Ok(())
}
