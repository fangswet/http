use std::error::Error;
use std::borrow::Cow;
use std::collections::VecDeque;
use flate2::{
    Compression, read::{GzDecoder, GzEncoder, DeflateDecoder, DeflateEncoder}
};
use std::fmt::{self, Debug, Display, Formatter};
use std::io::{self, ErrorKind, Read, Write};
use lazy_static::lazy_static;

#[derive(Clone, Copy, Debug)]
pub enum Method {
    GET,
    POST,
}

impl Method {
    pub fn parse(method: &str) -> Option<Self> {
        match method.to_uppercase().as_str() {
            "GET" => Some(Method::GET),
            "POST" => Some(Method::POST),
            _ => None,
        }
    }
}

impl Display for Method {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Method::GET => write!(f, "GET"),
            Method::POST => write!(f, "POST"),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Version {
    V1,
    V11,
    V2,
}

impl Version {
    pub fn parse(version: &str) -> Option<Self> {
        match version {
            "HTTP/1.0" => Some(Version::V1),
            "HTTP/1.1" => Some(Version::V11),
            "HTTP/2.0" => Some(Version::V2),
            _ => None,
        }
    }
}

impl Display for Version {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Version::V1 => write!(f, "HTTP/1.0"),
            Version::V11 => write!(f, "HTTP/1.1"),
            Version::V2 => write!(f, "HTTP/2.0"),
        }
    }
}   

#[derive(Clone, Copy, Debug)]
pub enum Status {
    Ok,
    NotFound,
    MovedPermanently
}

impl Status {
    pub fn parse(status: &str) -> Option<Self> {
        match status {
            "200" => Some(Status::Ok),
            "404" => Some(Status::NotFound),
            "301" => Some(Status::MovedPermanently),
            _ => None,
        }
    }

    pub fn from_error(error_kind: ErrorKind) -> Option<Self> {
        match error_kind {
            ErrorKind::NotFound => Some(Self::NotFound),
            _ => None,
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Status::Ok => "OK",
            Status::NotFound => "Not Found",
            Status::MovedPermanently => "Moved Permanently"
        }
    }

    pub fn validate_message(&self, message: &str) -> bool {
        message == self.message()
    }
}

impl Display for Status {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Status::Ok => write!(f, "200"),
            Status::NotFound => write!(f, "404"),
            Status::MovedPermanently => write!(f, "301"),
        }
    }
}

const ENCODING_GZIP: &'static str = "gzip";
const ENCODING_DEFLATE: &'static str = "deflate";
const ENCODING_BROTLI: &'static str = "br";

#[derive(Clone, Copy, Debug)]
pub enum Encoding {
    GZip,
    Deflate,
    Brotli
}

impl Encoding {
    fn decode(&self, encoded: &[u8], decoded: &mut Vec<u8>) -> io::Result<usize> {
        match self {
            Self::GZip => GzDecoder::new(encoded).read_to_end(decoded),
            Self::Deflate => DeflateDecoder::new(encoded).read_to_end(decoded),
            Self::Brotli => brotli::Decompressor::new(encoded, 4096).read_to_end(decoded),
        }
    }

    fn encode(&self, payload: &[u8], encoded: &mut Vec<u8>) -> io::Result<usize> {
        match self {
            Self::GZip => GzEncoder::new(payload, Compression::default()).read_to_end(encoded),
            Self::Deflate => DeflateEncoder::new(payload, Compression::default()).read_to_end(encoded),
            Self::Brotli => brotli::CompressorReader::new(payload, 4096, 3, 20).read_to_end(encoded)
        }
    }
}

impl Parsable for Encoding {
    fn parse(value: &str) -> Option<Self> {
        match value.to_lowercase().as_str() {
            ENCODING_GZIP => Some(Self::GZip),
            ENCODING_DEFLATE => Some(Self::Deflate),
            ENCODING_BROTLI => Some(Self::Brotli),
            _ => None,
        }
    }
}

impl ToHeader for Encoding {
    fn normalized() -> &'static str { "content-encoding" }
    fn value(&self) -> &'static str {
        match self {
            Self::GZip => ENCODING_GZIP,
            Self::Deflate => ENCODING_DEFLATE,
            Self::Brotli => ENCODING_BROTLI
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum ContentType {
    HTML,
}

pub struct Encodings(Vec<Encoding>);

impl Encodings {
    pub fn parse(encodings: &str) -> Self {
        Self(Encoding::parse_many(encodings))
    }

    pub fn decode(&self, encoded: &[u8]) -> io::Result<Vec<u8>> {
        let mut buffer = encoded.to_vec();
        let mut decoded;

        for encoding in self.0.iter().rev() {
            decoded = Vec::new();
            encoding.decode(&buffer, &mut decoded)?;
            buffer = decoded;
        }

        Ok(buffer)
    }

    pub fn encode(&self, payload: &[u8]) -> io::Result<Vec<u8>> {
        let mut buffer = payload.to_vec();
        let mut encoded;

        for encoding in &self.0 {
            encoded = Vec::new();
            encoding.encode(&buffer, &mut encoded)?;
            buffer = encoded;
        }

        Ok(buffer)
    }
}

pub struct Header<'a> {
    pub name: Cow<'a, str>,
    pub value: Cow<'a, str>
}

impl<'a> Header<'a> {
    pub fn new<N: Into<Cow<'a, str>>, V: Into<Cow<'a, str>>>(name: N, value: V) -> Self {
        Self { name: name.into(), value: value.into() }
    }

    pub fn from<T: ToHeader>(to_header: T) -> Self {
        Self::new(T::normalized(), to_header.value())
    }

    pub fn from_many<T: ToHeader>(to_headers: &[T]) -> Self {
        Self { 
            name: T::normalized().into(), 
            value: to_headers.into_iter().map(|x| x.value()).collect::<Vec<_>>().join(T::delimiter()).into() 
        }
    }

    pub fn parse(header: &'a str) -> Option<Self> {
        match header.find(':') {
            Some(colon_index) if colon_index < header.len() - 1 
                => Some(Self::new(&header[..colon_index], header[(colon_index + 1)..].trim_start())),
            _ => None
        }
    }

    pub fn construct(&self) -> String {
        format!("{}: {}", self.name, self.value)
    }
}

impl<'a> Display for Header<'a> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "{}", self.construct())
    }
}

const CONTENT_TYPE_HTML: &'static str = "text/html";

// ehh the name
pub trait Parsable where Self: Sized {
    fn parse(value: &str) -> Option<Self>;
}

pub trait ToHeader where Self: Parsable {
    fn normalized() -> &'static str;
    fn value(&self) -> &'static str;
    fn is_multi() -> bool { true }
    fn delimiter() -> &'static str { "," }
    fn parse_many(values: &str) -> Vec<Self> {
        values.split(Self::delimiter()).filter_map(|v| Self::parse(v)).collect()
    }
}

impl ContentType {  
    fn from_extension(extension: &str) -> Option<Self> {
        match extension.to_lowercase().as_str() {
            "html" => Some(ContentType::HTML),
            _ => None,
        }
    }
}

impl Parsable for ContentType {
    fn parse(value: &str) -> Option<Self> {
        match value.to_lowercase().as_str() {
            CONTENT_TYPE_HTML => Some(ContentType::HTML),
            _ => None,
        }
    }
}

impl ToHeader for ContentType {
    fn normalized() -> &'static str { "content-type" }
    fn value(&self) -> &'static str {
        match self {
            ContentType::HTML => CONTENT_TYPE_HTML
        }
    }
}

pub enum TransferEncoding {
    Chunked,
    Indentity
}

const TRANSFER_ENCODING_CHUNKED: &'static str = "chunked";
const TRANSFER_ENCODING_IDENTITY: &'static str = "identity";

impl Parsable for TransferEncoding {
    fn parse(value: &str) -> Option<Self> {
        match value.to_lowercase().as_str() {
            TRANSFER_ENCODING_CHUNKED => Some(TransferEncoding::Chunked),
            TRANSFER_ENCODING_IDENTITY => Some(TransferEncoding::Indentity), // perhaps add "default" method
            _ => None
        }
    }
}

impl ToHeader for TransferEncoding {
    fn normalized() -> &'static str { "transfer-encoding" }
    fn value(&self) -> &'static str {
        match self {
            TransferEncoding::Chunked => TRANSFER_ENCODING_CHUNKED,
            TransferEncoding::Indentity => TRANSFER_ENCODING_IDENTITY,
        }
    }
    fn is_multi() -> bool { false }
}

pub enum Protocol {
    Http, Https
}

const PROTOCOL_HTTP: &'static str = "http";
const PROTOCOL_HTTPS: &'static str = "https";

impl Parsable for Protocol {
    fn parse(protocol: &str) -> Option<Self> {
        match protocol {
            PROTOCOL_HTTP => Some(Self::Http),
            PROTOCOL_HTTPS => Some(Self::Https),
            _ => None
        }
    }
}

impl Protocol {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Http => PROTOCOL_HTTP,
            Self::Https => PROTOCOL_HTTPS
        }
    }
}

const CONNECTION_CLOSE: &'static str = "close";
const CONNECTION_KEEP_ALIVE: &'static str = "keep-alive";

pub enum Connection {
    Close, KeepAlive
}

impl Parsable for Connection {
    fn parse(value: &str) -> Option<Self> {
        match value.to_lowercase().as_str() {
            CONNECTION_CLOSE => Some(Connection::Close),
            CONNECTION_KEEP_ALIVE => Some(Connection::KeepAlive),
            _ => None,
        }
    }
}

impl ToHeader for Connection {
    fn normalized() -> &'static str { "connection" }
    fn value(&self) -> &'static str {
        match self {
            Connection::Close => CONNECTION_CLOSE,
            Connection::KeepAlive => CONNECTION_KEEP_ALIVE
        }
    }
}

#[derive(Clone, Debug)]
pub enum ParsingError {
    Method,
    Version,
    Encoding,
    ContentType,
    Status,
    Head,
    Header,
    Payload,
    Empty,
    Capacity,
    IO,
}

impl Display for ParsingError {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl Error for ParsingError { }