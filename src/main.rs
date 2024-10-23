#![allow(clippy::blocks_in_conditions)]
use std::{
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket},
    sync::Arc,
    time::Duration,
};

use clap::{Args, Parser, Subcommand};
use ringbuf::traits::{Consumer, Producer, Split};
use ringbuf_blocking::{BlockingHeapRb, BlockingRb};

mod play;
mod record;

type RingBuf = Arc<BlockingHeapRb<u8>>;
type RingProd = ringbuf_blocking::BlockingProd<RingBuf>;
type RingCons = ringbuf_blocking::BlockingCons<RingBuf>;

#[derive(Debug, Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    /// Inactivity timer (reset the connection after not seeing any data in this much seconds)
    ///
    /// If this is 0 for UDP, any source address is accepted
    #[arg(short, long)]
    inactivity_sec: Option<u32>,

    #[command(flatten)]
    net: Endpoint,

    #[command(subcommand)]
    command: Cmd,
}

#[derive(Clone, Debug, Subcommand)]
enum Cmd {
    Play {
        /// Amount of samples to buffer
        #[arg(short = 's', long)]
        buffer_samples: Option<u8>,
        #[arg(short, long)]
        device_name: Option<String>,
    },
    Record {
        #[arg(short, long)]
        node_name: String,
    },
}

trait ProdCons {
    fn work_prod(&mut self, prod: &mut RingProd, inactivity_sec: u32);
    fn work_cons(&mut self, cons: &mut RingCons, inactivity_sec: u32);
}

#[derive(Args, Copy, Clone, Debug)]
struct Endpoint {
    /// Whether to listen for connections instead of connecting to the address
    #[arg(short, long)]
    listen: bool,

    /// Whether to use UDP
    #[arg(short, long)]
    udp: bool,

    /// Connect/bind address
    #[arg(short, long)]
    address: SocketAddr,
}

impl Endpoint {
    fn bind_udp(&self) -> Option<UdpSocket> {
        match std::net::UdpSocket::bind(if self.listen {
            self.address
        } else {
            SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0)
        })
        .and_then(|sock| {
            if !self.listen {
                sock.connect(self.address)?;
            }
            Ok(sock)
        }) {
            Ok(sock) => Some(sock),
            Err(err) => {
                log::error!("udp bind: {err}");
                std::thread::sleep(Duration::from_secs(2));
                None
            }
        }
    }
    fn bind(&self) -> Option<TcpListener> {
        match std::net::TcpListener::bind(self.address) {
            Ok(listener) => Some(listener),
            Err(err) => {
                log::error!("bind: {err}");
                std::thread::sleep(Duration::from_secs(2));
                None
            }
        }
    }
    fn connect(&self) -> Option<TcpStream> {
        match std::net::TcpStream::connect(self.address) {
            Ok(conn) => Some(conn),
            Err(err) => {
                log::error!("connect: {err}");
                std::thread::sleep(Duration::from_secs(2));
                None
            }
        }
    }
}

impl ProdCons for Endpoint {
    fn work_cons(&mut self, cons: &mut RingCons, inactivity_sec: u32) {
        loop {
            if self.udp {
                let Some(mut sock) = self.bind_udp() else {
                    continue;
                };
                sock.work_cons(cons, inactivity_sec);
            } else if self.listen {
                let Some(mut listener) = self.bind() else {
                    continue;
                };
                listener.work_cons(cons, inactivity_sec);
            } else {
                let Some(mut conn) = self.connect() else {
                    continue;
                };
                conn.work_cons(cons, inactivity_sec);
            }
        }
    }
    fn work_prod(&mut self, prod: &mut RingProd, inactivity_sec: u32) {
        loop {
            if self.udp {
                let Some(mut sock) = self.bind_udp() else {
                    continue;
                };
                sock.work_prod(prod, inactivity_sec);
            } else if self.listen {
                let Some(mut listener) = self.bind() else {
                    continue;
                };
                listener.work_prod(prod, inactivity_sec);
            } else {
                let Some(mut conn) = self.connect() else {
                    continue;
                };
                conn.work_prod(prod, inactivity_sec);
            }
        }
    }
}

impl ProdCons for TcpListener {
    fn work_prod(&mut self, prod: &mut RingProd, inactivity_sec: u32) {
        while let Ok((mut conn, _addr)) = self.accept() {
            conn.work_prod(prod, inactivity_sec);
        }
    }
    fn work_cons(&mut self, cons: &mut RingCons, inactivity_sec: u32) {
        while let Ok((mut conn, _addr)) = self.accept() {
            conn.work_cons(cons, inactivity_sec);
        }
    }
}

impl ProdCons for TcpStream {
    fn work_cons(&mut self, cons: &mut RingCons, inactivity_sec: u32) {
        let _ = self.set_read_timeout(Some(Duration::from_secs(inactivity_sec.into())));
        let mut buf = [0u8; 65536];
        loop {
            match cons.wait_occupied(1) {
                Ok(()) => {}
                Err(err) => match err {
                    ringbuf_blocking::WaitError::Closed => break,
                    ringbuf_blocking::WaitError::TimedOut => continue,
                },
            }
            let len = cons.pop_slice(&mut buf);
            if self.write_all(&buf[..len]).is_err() {
                break;
            }
        }
    }
    fn work_prod(&mut self, prod: &mut RingProd, inactivity_sec: u32) {
        let _ = self.set_read_timeout(Some(Duration::from_secs(inactivity_sec.into())));
        let mut buf = [0u8; 65536];
        while let Ok(len) = self.read(&mut buf) {
            loop {
                match prod.wait_vacant(len) {
                    Ok(()) => break,
                    Err(err) => match err {
                        ringbuf_blocking::WaitError::Closed => return,
                        ringbuf_blocking::WaitError::TimedOut => continue,
                    },
                }
            }
            let mut pushed = 0;
            while pushed < len {
                pushed += prod.push_slice(&buf[pushed..len]);
            }
        }
    }
}

impl ProdCons for UdpSocket {
    fn work_cons(&mut self, cons: &mut RingCons, inactivity_sec: u32) {
        let mut buf = [0u8; 65536];
        if self
            .set_write_timeout(Some(Duration::from_secs(inactivity_sec.into())))
            .is_err()
        {
            return;
        }
        loop {
            match cons.wait_occupied(1) {
                Ok(()) => {}
                Err(err) => match err {
                    ringbuf_blocking::WaitError::Closed => break,
                    ringbuf_blocking::WaitError::TimedOut => continue,
                },
            }
            let len = cons.pop_slice(&mut buf);
            match self.send(&buf[..len]) {
                Ok(_) => {}
                Err(err) => {
                    log::error!("udp send: {err}");
                    break;
                }
            }
        }
    }
    fn work_prod(&mut self, prod: &mut RingProd, inactivity_sec: u32) {
        let mut buf = [0u8; 65536];
        let mut connected = false;
        while let Ok(len) = if connected {
            self.recv(&mut buf)
        } else {
            self.recv_from(&mut buf).and_then(|(len, other)| {
                if inactivity_sec != 0 {
                    self.set_read_timeout(Some(Duration::from_secs(inactivity_sec.into())))?;
                    connected = self.connect(other).is_ok();
                }
                Ok(len)
            })
        } {
            loop {
                match prod.wait_vacant(len) {
                    Ok(()) => break,
                    Err(err) => match err {
                        ringbuf_blocking::WaitError::Closed => return,
                        ringbuf_blocking::WaitError::TimedOut => continue,
                    },
                }
            }
            let mut pushed = 0;
            while pushed < len {
                pushed += prod.push_slice(&buf[pushed..len]);
            }
        }
    }
}

fn main() {
    env_logger::init();
    let mut args = Cli::parse();
    loop {
        let buf = BlockingRb::new(0x40000);
        let (mut prod, mut cons) = buf.split();
        let res = match args.command.clone() {
            Cmd::Record { node_name } => {
                std::thread::spawn(move || {
                    args.net
                        .work_cons(&mut cons, args.inactivity_sec.unwrap_or(2))
                });
                record::main(node_name, prod)
            }
            Cmd::Play {
                buffer_samples,
                device_name,
            } => {
                std::thread::spawn(move || {
                    args.net
                        .work_prod(&mut prod, args.inactivity_sec.unwrap_or(2))
                });
                play::main(cons, buffer_samples.unwrap_or(0).into(), device_name)
            }
        };
        match res {
            Ok(()) => log::error!("main loop exited, restarting..."),
            Err(err) => log::error!("main loop exited with error: {err}"),
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}
