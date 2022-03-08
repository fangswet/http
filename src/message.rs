use std::fmt::Display;
use std::fmt::Formatter;
use std::borrow::Cow;
use std::error::Error;
use std::collections::HashMap;
use std::io;
use std::str;
use std::io::{ Read, Write, BufRead, BufReader };
use std::net::ToSocketAddrs;
use mio::{ Events, Interest, Poll, Token, net::TcpStream };
use lazy_static::lazy_static;
use regex::Regex;
use std::ops::Index;
use crate::def::*;

const NEWLINE: &'static str = "\r\n";

pub struct Parameter<'a> {
    pub name: Cow<'a, str>,
    pub value: Option<Cow<'a, str>>
}

impl<'a> Parameter<'a> {
    pub fn new<S: Into<Cow<'a, str>>>(name: S, value: Option<S>) -> Self {
        Self { name: name.into(), value: value.map(|v| v.into()) }
    }

    pub fn parse(parameter: &str) -> Option<Self> {
        let parameter = unsafe { &*(parameter as *const str) };
        let pair: Vec<_> = parameter.split('=').collect();
        if pair.len() > 2 { return None }
        let value = if pair.len() == 1 { None } else { Some(Cow::Borrowed(pair[1])) };

        Some(Self { name: Cow::Borrowed(pair[0]), value })
    }

    pub fn parse_many(parameters: &str) -> Vec<Self> {
        parameters.split('&').filter_map(|p| Parameter::parse(p)).collect()
    }

    pub fn construct(&self) -> String {
        match &self.value {
            Some(value) => format!("{}={}", self.name, value),
            None => self.name.to_string()
        }
    }
}

pub struct Target<'a> {
    pub location: Cow<'a, str>,
    pub parameters: Vec<Parameter<'a>>
}

impl<'a> Target<'a> {
    pub fn default() -> Self {
        Self { location: Cow::Borrowed("/"), parameters: Vec::new() }
    }

    // we need a way to minimize the unsafe pointer thing
    // perhaps accept &'static str String
    // target has a lifetime here so we probably should change that
    // also i was thinking of removing unsafe with a struct that holds a &str or &string and a slice/range
    // and as_ref method indexes the str
    pub fn parse(target: &'a str) -> Option<Self> {
        let mut parts = target.split('?');
        let mut target = Target::default();
        if let Some(location) = parts.next() {
            return Some(Self { 
                location: Cow::Borrowed(location), 
                parameters: Parameter::parse_many(parts.as_str()) })
        }

        None
    }
}

impl<'a> Display for Target<'a> {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        let mut target = self.location.to_string();
        if !self.parameters.is_empty() {
            let parameters = self.parameters.iter().map(|p| p.construct()).collect::<Vec<String>>().join("&");
            target.push_str("?");
            target.push_str(&parameters);
        }

        write!(f, "{}", target)
    }
}

// we could make a parse/construct trait because display is too expensive i think
pub struct Headline<'a>(&'a str, &'a str, &'a str);

impl<'a> Headline<'a> {
    pub fn parse(reader: &mut impl BufRead, text: &mut String) -> Result<Self, Box<dyn Error>> {
        let mut line = read_string_line(reader, text)?;
        let mut parts = line.split(' ');
        let first = parts.next().ok_or(ParsingError::Head)?;
        let second = parts.next().ok_or(ParsingError::Head)?;
        let third = parts.as_str();
        if third.len() == 0 { Err(ParsingError::Head)?; }

        Ok(Headline(first, second, third))
    }

    pub fn construct(first: impl Display, second: impl Display, third: impl Display) -> Vec<u8> {
        format!("{} {} {}{}", first, second, third, NEWLINE).into_bytes()
    }
}

// pub for debug
pub struct Request<'a> {
    pub method: Method,
    pub target: Target<'a>,
    pub version: Version,
    pub message: Message<'a>,
    text: String
}

impl<'a> Request<'a> {
    pub fn new() -> Self {
        Self { method: Method::GET,  target: Target::default(), version: Version::V11, message: Message::new(), text: String::new() }
    }

    pub fn parse<R: BufRead>(reader: &mut R) -> Result<Self, Box<dyn Error>> {
        let mut text = String::new();
        let Headline(method, target, version) = Headline::parse(reader, &mut text)?;

        Ok(Self {
            method: Method::parse(method).ok_or(ParsingError::Method)?,
            target: Target::parse(target).ok_or(ParsingError::Head)?,
            version: Version::parse(version).ok_or(ParsingError::Version)?,
            message: Message::parse(reader, &mut text)?,
            text
        })
    }

    pub fn text(&self) -> String {
        format!("{}{}", self.text, String::from_utf8_lossy(self.message.payload.raw()))
    }

    pub fn construct(&mut self) -> Vec<u8> {
        let mut request = Vec::new();
        request.extend(Headline::construct(self.method, &self.target, self.version));
        request.extend(self.message.construct());

        request
    }
}

pub struct Response<'a> {
    pub version: Version,
    pub status: Status,
    pub message: Message<'a>,
    text: String
}

impl<'a> Response<'a> {
    pub fn new() -> Self {
        Self { version: Version::V11, status: Status::Ok, message: Message::new(), text: String::new() }
    }

    pub fn parse<R: BufRead>(reader: &mut R) -> Result<Self, Box<dyn Error>> {
        let mut text = String::new();
        let Headline(version, status, message) = Headline::parse(reader, &mut text)?;
        let status = Status::parse(status).ok_or(ParsingError::Status)?;
        if !status.validate_message(message) { Err(ParsingError::Status)?; }
        
        Ok(Self {
            version: Version::parse(version).ok_or(ParsingError::Version)?,
            message: Message::parse(reader, &mut text)?,
            status, text
        })
    }

    pub fn text(&self) -> String {
        format!("{}{}", self.text, String::from_utf8_lossy(self.message.payload.raw()))
    }

    pub fn construct(&mut self) -> Vec<u8> {
        let mut response = Vec::new();
        response.extend(Headline::construct(self.version, self.status, self.status.message()));
        response.extend(self.message.construct());

        response
    }
}

pub struct Headers<'a>(HashMap<String, Header<'a>>);

impl<'a> Headers<'a> {
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    pub fn add(&mut self, header: Header<'a>) {
        self.0.insert(header.name.to_lowercase(), header);
    }

    pub fn have<T: ToHeader>(&self, to_header: T) -> bool {
        if let Some(header) = self.0.get(T::normalized()) { 
            return header.value.to_lowercase() == to_header.value();
        }

        false
    }
    
    pub fn list(&self) -> std::collections::hash_map::Values<String, Header> {
        self.0.values()
    }

    pub fn get(&self, normalized: &str) -> Option<&str> {
        self.0.get(normalized).map(|h| &h.value as &str)
    }

    pub fn parse<R: BufRead>(reader: &mut R, text: &mut String) -> Result<Self, Box<dyn Error>> {
        let mut headers = Self::new();
        
        loop {
            let line = read_string_line(reader, text)?;
            if line.len() == 0 || text.len() > 10000 { break; }
            if let Some(header) = Header::parse(line) {
                headers.add(header);
            }
        }

        Ok(headers)
    }

    pub fn construct(&self) -> Vec<u8> {
        let mut headers = Vec::new();
        for header in self.list() {
            headers.extend(header.construct().as_bytes());
            headers.extend(NEWLINE.as_bytes());
        }

        headers
    }
}

impl<'a> Display for Headers<'a> {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        write!(f, "{}", self.0.values().fold(String::new(), |acc, h| acc + &h.construct() + "\r\n"))
    }
}

pub struct Message<'a> {
    pub headers: Headers<'a>,
    pub payload: Payload<'a>
}

impl<'a> Message<'a> {
    pub fn new() -> Self {
        Self { headers: Headers::new(), payload: Payload::default() }
    }

    fn parse<R: BufRead>(mut reader: &mut R, text: &mut String) -> Result<Self, Box<dyn Error>> {
        let mut message = Message::new();
        message.headers = Headers::parse(reader, text)?;
        if let Some(content_length) = message.headers.get("content-length") {
            let content_length_usize = content_length.parse::<usize>().or(Err(ParsingError::Payload))?;
            message.payload = Payload::read(reader, content_length_usize)?;
        } else if message.headers.have(TransferEncoding::Chunked) {
            message.payload = Payload::dechunk(reader)?;
        }

        if let Some(encodings) = message.headers.get("content-encoding") {
            message.payload = message.payload.decode(Encodings::parse(encodings))?;
        }

        Ok(message)
    }

    pub fn construct(&mut self) -> Vec<u8> {
        let payload = self.payload.construct();
        if payload.len() > 0 {
            self.headers.add(Header::new("Content-Length", payload.len().to_string())); // contentlength could be toheader
        }

        let mut message = self.headers.construct();
        message.extend(NEWLINE.as_bytes());
        message.extend(payload);
        message
    }
}

fn read_string_line(reader: &mut impl BufRead, text: &mut String) -> io::Result<&'static str> {
    let size = reader.read_line(text)?;
    if size < NEWLINE.len() { Err(io::Error::new(io::ErrorKind::InvalidData, ""))? }
    let line = unsafe { &*(&text[text.len() - size .. text.len() - NEWLINE.len()] as *const str) };
    Ok(line)
}

// this has to be done somewhere else
// fn read_string_line_nonblocking(reader: &mut impl BufRead, text: &mut String) -> io::Result<&'static str> {
//     let mut size;
//     loop {
//         match reader.read_line(text) {
//             Ok(s) => { size = s; break },
//             Err(e) if e.kind() == io::ErrorKind::InvalidData || e.kind() == io::ErrorKind::WouldBlock => continue,
//             Err(e) => Err(e)?
//         }
//     }
//     if size < NEWLINE.len() { Err(io::Error::new(io::ErrorKind::InvalidData, ""))? }
//     let line = unsafe { &*(&text[text.len() - size .. text.len() - NEWLINE.len()] as *const str) };
//     Ok(line)
// }

fn read_exact_string(reader: &mut impl BufRead, text: &mut String, size: usize) -> io::Result<&'static str> {
    let mut buffer = vec![0 as u8; size];
    reader.read_exact(&mut buffer)?;
    let text_length = text.len();
    text.push_str(&String::from_utf8_lossy(buffer.as_ref()));
    Ok(unsafe { &*(&text[text_length..] as *const str) })
}

fn read_line(reader: &mut impl BufRead, buffer: &mut Vec<u8>) -> io::Result<&'static [u8]> {
    let size = reader.read_until('\n' as u8, buffer)?;
    if size < NEWLINE.len() { Err(io::Error::new(io::ErrorKind::InvalidData, ""))? } // return some io error here
    let line = unsafe { &*(&buffer[buffer.len() - size .. buffer.len() - NEWLINE.len()] as *const [u8]) };
    Ok(line)
}

fn read_exact(reader: &mut impl BufRead, buffer: &mut Vec<u8>, size: usize) -> io::Result<&'static [u8]> {
    let mut exact_buffer = vec![0 as u8; size];
    reader.read_exact(&mut exact_buffer)?;
    let buffer_length = buffer.len();
    buffer.append(&mut exact_buffer);
    Ok(unsafe { &*(&buffer[buffer_length..] as *const [u8]) })
}

fn read_str_line(reader: &mut impl BufRead, buffer: &mut Vec<u8>) -> io::Result<&'static str> {
    Ok(str::from_utf8(read_line(reader, buffer)?).or(Err(io::Error::new(io::ErrorKind::InvalidData, "")))?)
}

fn read_exact_str(reader: &mut impl BufRead, buffer: &mut Vec<u8>, size: usize) -> io::Result<&'static str> {
    Ok(str::from_utf8(read_exact(reader, buffer, size)?).or(Err(io::Error::new(io::ErrorKind::InvalidData, "")))?)
}

// we need to optimize this 
pub enum Payload<'a> {
    Identity(Cow<'a, [u8]>),
    Chunked { content:  Vec<u8>, chunks: Vec<&'a [u8]> }
}

impl<'a> Payload<'a> {
    pub fn default() -> Self {
        Self::Identity(Cow::Borrowed(&[]))
    }

    pub fn new(content: &[u8]) -> Self {
        Self::Identity(Cow::Owned(content.to_owned()))
    }

    pub fn read(reader: &mut impl BufRead, length: usize) -> io::Result<Self> {
        let mut content = Vec::new();
        read_exact(reader, &mut content, length)?;
        Ok(Self::Identity(Cow::Owned(content)))
    }

    // optimize
    pub fn chunks(&self) -> Vec<&[u8]> { // this should be a iterator if only iteator could own the values
        match self {
            Self::Chunked { chunks, .. } => chunks.clone(),
            Self::Identity(content) => vec![content.as_ref()]
        }
    }

    pub fn dechunk(reader: &mut impl BufRead) -> Result<Self, Box<dyn Error>> {
        let mut content = Vec::new();
        let mut chunks = Vec::new();
        loop {
            let line = read_str_line(reader, &mut content)?;
            if line.len() < 1 || line == "0" { break }
            let chunk_size = usize::from_str_radix(line, 16).or(Err(""))?;
            let chunk = read_exact(reader, &mut content, chunk_size + NEWLINE.len())?;
            chunks.push(chunk);
        }

        Ok(Self::Chunked { content, chunks })
    }

    pub fn raw(&self) -> &[u8] {
        match self {
            Self::Identity(content) => content,
            Self::Chunked { content, .. } => content
        }
    }

    pub fn text(&self) -> String {
        self.chunks().iter().fold(String::new(), |acc, c| acc + &String::from_utf8_lossy(c))
    }

    pub fn construct(&self) -> &[u8] {
        match self {
            Self::Identity(content) => content,
            // perhaps we could implement that later but idk if its even worth it
            // maybe only if we read from an iterator and constructed chunks on the go 
            Self::Chunked { .. } => panic!("chunked construct")
        }
    }

    // optimize
    pub fn decode(&self, encodings: Encodings) -> io::Result<Self> {
        let decoded = match self {
            Self::Identity(content) => encodings.decode(content)?,
            Self::Chunked { chunks, .. } => encodings.decode(&chunks.concat())?
        };
        
        Ok(Self::Identity(Cow::Owned(decoded)))
    }

    pub fn encode(&self, encodings: Encodings) -> io::Result<Self> {
        Ok(Self::Identity(Cow::Owned(encodings.encode(self.construct())?)))
    }
}