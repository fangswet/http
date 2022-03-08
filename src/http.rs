
// we need to store tcpstream for keep-alive connections

use core::pin::Pin;
use core::ptr::NonNull;
use std::sync::Arc;
use std::net::{ TcpStream, TcpListener, SocketAddr, ToSocketAddrs };
use std::error::Error;
use core::convert::TryFrom;
use regex::Regex;
use std::borrow::Cow;
use std::io::{ Read, Write, BufRead, BufReader };
use std::thread;
use lazy_static::lazy_static;
use rustls;
use webpki;
use crate::message::*;
use crate::def::*;

const PORT_HTTP: usize = 80;
const PORT_HTTPS: usize = 443;

lazy_static! {
    static ref URI_REGEX: Regex = Regex::new(r"(?ix)
        ^(?:(?P<protocol>https?)://)?
        (?:www\d*\.)?
        (?P<domain>[^-][a-z0-9-\.]+[^-]\.[a-z0-9]+)
        (?::(?P<port>\d+))?
        (?P<location>/[^\s\?]*)?
        (?:\?(?P<parameters>[^\s\?\\/]*))?$").unwrap();

    static ref RUSTLS_CLIENT_CONFIG: Arc<rustls::ClientConfig> = Arc::new(rustls_client_config());
    static ref RUSTLS_SERVER_CONFIG: Arc<rustls::ServerConfig> = Arc::new(rustls_server_config());
}

fn rustls_client_config() -> rustls::ClientConfig {
    let mut config = rustls::ClientConfig::new();
    config.root_store.add_server_trust_anchors(&webpki_roots::TLS_SERVER_ROOTS);
    config
}

fn rustls_server_config() -> rustls::ServerConfig {
    let mut config = rustls::ServerConfig::new(rustls::NoClientAuth::new());
    config
}

#[derive(Clone)]
pub struct Address<'a> {
    domain: Cow<'a, str>,
    port: usize
}

impl<'a>  Address<'a> {
    pub fn new<S: Into<Cow<'a, str>>>(domain: S, port: Option<usize>) -> Self {
        Self { domain: domain.into(), port: port.unwrap_or(PORT_HTTP) }
    }

    pub fn to_string(&self) -> String {
        format!("{}:{}", self.domain, self.port)
    }

    pub fn host(&self) -> String {
        format!("www.{}", self.domain) 
    }
}

pub struct Uri<'a> {
    address: Address<'a>,
    target: Target<'a>,
    protocol: Option<Protocol>
}

impl<'a> Uri<'a> {
    pub fn parse(uri: &str) -> Option<Self> {
        if let Some(capture) = URI_REGEX.captures_iter(uri).next() {
            let mut address = Address::new(capture["domain"].to_string(), None);
            let mut target = Target::default();
            let protocol = capture.name("protocol").map(|p| Protocol::parse(p.as_str())).flatten();
    
            if let Some(location) = capture.name("location") {
                target.location = Cow::Owned(location.as_str().to_string());
            }
            if let Some(port) = capture.name("port") {
                address.port = port.as_str().parse::<usize>().unwrap();
            }
            else if let Some(Protocol::Https) = protocol {
                address.port = PORT_HTTPS;
            }
            if let Some(parameters) = capture.name("parameters") {
                target.parameters = Parameter::parse_many(parameters.as_str());
            }
    
            return Some(Self { address, target, protocol });
        }
    
        None
    }
}

// would be cool if we could return a new thread 
// pub struct Incoming {
//     listener: TcpListener
// }

// impl Iterator for Incoming {
//     type Item = Request<'static>;

//     fn next(&mut self) -> Option<Self::Item> {
//         Some(Request::new())
//     }
// }

// todo implement keep-alive then h2
pub struct Http11 { }

impl<'a> Http11 {
    pub fn send(address: Address, request: &mut Request) -> Result<Response<'a>, Box<dyn Error>> {
        let mut stream = TcpStream::connect(address.to_string())?;
        request.message.headers.add(Header::new("Host", address.host()));
        request.version = Version::V11;
        stream.write_all(&request.construct())?;
        Response::parse(&mut BufReader::new(stream))
    }

    pub fn listen<H>(address: Address, handler: &'static H) -> Result<thread::JoinHandle<()>, Box<dyn Error>> 
    where H: Fn(Request) -> Option<Response> + Sync {
        let mut listener = TcpListener::bind(address.to_string())?;

        let handle = thread::spawn(move || {
            for mut stream in listener.incoming().filter_map(|s| s.ok()) {
                thread::spawn(move || {
                    if let Ok(request) = Request::parse(&mut BufReader::new(&stream)) {
                        if let Some(mut response) = handler(request) {
                            stream.write_all(&response.construct()).unwrap();
                        }
                    }
                });
            }
        });

        Ok(handle)
    }
}

pub struct TlsStream<'a> {
    pub stream: rustls::StreamOwned<rustls::ClientSession, TcpStream>,
    address: Address<'a>
}

impl<'a> TlsStream<'a> {
    pub fn connect(address: Address<'a>) -> Result<Self, Box<dyn Error>> {
        let dns_name = webpki::DNSNameRef::try_from_ascii_str(&address.domain)?;
        let session = rustls::ClientSession::new(&RUSTLS_CLIENT_CONFIG, dns_name);
        let socket = TcpStream::connect(address.to_string())?;

        Ok(Self { stream: rustls::StreamOwned::new(session, socket), address })
    }
}

// pub struct TlsStream<'a> {
//     address: Address<'a>,
//     stream: Option<RustlsStream<'a>>,
//     socket: TcpStream,
//     session: rustls::ClientSession,
// }

// impl<'a> TlsStream<'a> {
//     pub fn connect(address: Address<'a>) -> Result<Box<Self>, Box<dyn Error>> {
//         let dns_name = webpki::DNSNameRef::try_from_ascii_str(&address.domain)?;
//         let mut session = rustls::ClientSession::new(&RUSTLS_CLIENT_CONFIG, dns_name);
//         let mut socket = TcpStream::connect(address.to_string())?;
//         let mut stream = Box::new(TlsStream { address, socket, session, stream: None });
//         let session_ref = unsafe { &mut *(&mut stream.session as *mut _) };
//         let socket_ref = unsafe { &mut *(&mut stream.socket as *mut _) };
//         stream.stream = Some(rustls::Stream::new(session_ref, socket_ref));

//         Ok(stream)
//     }

//     pub fn accept(stream: TcpStream) -> Result<Box<Self>, Box<dyn Error>> {
        

//         todo!()
//     }
    
//     pub fn get(&mut self) -> &mut RustlsStream<'a> {
//         self.stream.as_mut().unwrap()
//     }
// }

pub struct Https11<'a> {
    pub response: Response<'a>,
    pub stream: TlsStream<'a>
}

// make new struct with handler and listener
// how to keep the streams alive?
// how to determine when to close the stream?
// handler(Request) -> (Response, close: bool)
// but when we already accepted a stream and its saved how do we recieve again on it?
// i guess maybe the spawned stream stays trying to read from it (blocks until something comes)
// but we need to set nonblocking and timeout

impl<'a> Https11<'a> {
    // this is only for sending!
    // method like listen will also be provided for listening
    pub fn new(address: Address<'a>) -> Result<Self, Box<dyn Error>> {
        Ok(Self { stream: TlsStream::connect(address)?, response: Response::new() })
    }

    // we will probably need to add even more because of things like encoding (config)
    pub fn send(mut self, request: &mut Request) -> Result<Self, Box<dyn Error>> {
        request.message.headers.add(Header::new("Host", self.stream.address.host()));
        request.message.headers.add(Header::from(Connection::KeepAlive));
        request.version = Version::V11;
        self.stream.stream.write_all(&request.construct())?;
        self.response = Response::parse(&mut BufReader::new(&mut self.stream.stream))?;
        Ok(self)
    }

    pub fn listen<H>(address: Address, handler: &'static H) -> Result<thread::JoinHandle<()>, Box<dyn Error>>
    where H: for<'b> Fn(&'b Request) -> Option<Response<'b>> + Sync + Send {
        let mut listener = TcpListener::bind(address.to_string())?;

        let handle = thread::spawn(move || {
            for mut socket in listener.incoming().filter_map(|s| s.ok()) {
                thread::spawn(move || {
                    let mut session = rustls::ServerSession::new(&RUSTLS_SERVER_CONFIG);
                    let mut stream = rustls::Stream::new(&mut session, &mut socket);
                    
                    let mut buf = [0; 2000];
                    stream.read_exact(&mut buf).unwrap();
                    println!("{}", String::from_utf8_lossy(&buf));

                    // establish rustls serversession on stream
                    // return from thread if connection close
                    // block in loop to recieve more on keepalive
                    loop {
                        // ideally request::parse would block until new message
                        match Request::parse(&mut BufReader::new(&mut stream)) {              
                            Ok(request) => {
                                if let Some(mut response) = handler(&request) {
                                    stream.write_all(&response.construct()).unwrap();
                                }
    
                                if let Some(connection) = request.message.headers.get(Connection::normalized()) {
                                    if let Some(Connection::KeepAlive) = Connection::parse(&connection) {
                                        println!("keepalive");
                                        continue
                                    }
                                }
                            }              
                            Err(e) => eprintln!("{}", e)
                        }

                        break
                    }
                });
            }
        });

        Ok(handle)
    }
}

pub struct Http { }

impl<'a> Http {
    fn redirect(uri: &str, mut request: Request, limit: usize) -> Result<Response<'a>, Box<dyn Error>> {
        let uri = Uri::parse(uri).ok_or(ParsingError::Head)?;
        request.target = uri.target;
        let response = match uri.protocol {
            None | Some(Protocol::Http) => Http11::send(uri.address, &mut request)?,
            Some(Protocol::Https) => Https11::new(uri.address)?.send(&mut request)?.response
        };

        if let Status::MovedPermanently = response.status {
            if limit < 1 { Err("limit")? }
            if let Some(location) = response.message.headers.get("location") {
                return Http::redirect(location, request, limit - 1)
            }
        }
        
        Ok(response)
    }

    pub fn get(uri: &str) -> Result<Response<'a>, Box<dyn Error>> {
        Http::redirect(uri, Request::new(), 10)
    }
}