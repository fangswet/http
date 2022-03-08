#![allow(unused_variables)]
#![allow(unused_imports)]
#![allow(dead_code)]
use std::net::TcpListener;
use std::sync::Arc;
use std::net::ToSocketAddrs;
use std::io::BufRead;
use std::io::Cursor;
use std::pin::Pin;
use std::marker::PhantomPinned;
use std::ptr::NonNull;
use std::collections::HashMap;
use std::io::BufReader;
use std::borrow::Cow;
use std::io::{ Read, Write };
// use std::net::{ TcpListener, TcpStream, ToSocketAddrs };
use mio::{ Events, Interest, Poll, Token };
use std::time::Duration;
use std::error::Error;
use http::def::*;
use http::message::*;
use http::http::*;
use regex::Regex;

// this is a failed attempt to have non-blocking stream and real-time parsing io in a http server
// the issue is that parsing in real-time without strings is not really a thing
// and also trying to make every stream reading operation non-blocking is not very useful, expensive, and im not sure if possible at all
// what could be done is to have a blocking implementation with timeouts from parent threads (std tcpstream would eliviate the hassle of polling)
// or use tokio and profit from the flexibility of asynchronous code and stream reading implementation
// or take a different more plausible approach to reading the stream with mio where we read and take what we can in a efficient way
// and save and then parse the contents (but that would be messy and tricky to achieve, the parsing pipeline would not be as neatly defined)
// the important thing to take out is this: in our scenario mio would be great to write a single-thread listen scheduler which allows for 
// handling connections while others block (this idea might even get used)

#[derive(Clone, Copy)]
struct Range {
    offset: usize,
    length: usize
}

impl Range {
    pub fn new(offset: usize, length: usize) -> Self {
        Self { offset, length }
    }
}

impl std::ops::Index<Range> for String {
    type Output = str;

    fn index(&self, index: Range) -> &str {
        &self[index.offset..(index.offset + index.length)]
    }
}

struct TestHeader {
    name: Range,
    value: Range
}

struct Test {
    pub text: String,
    references: HashMap<String, TestHeader>
}

impl Test {
    pub fn new(text: String) -> Self {
        Self { text, references: HashMap::new() }
    }

    pub fn test(&mut self) {
        for line in self.text.split('\n') {
            let mut pair = line.split(':');
            let name = pair.next().unwrap();
            let name_range = Range::new(name.as_ptr() as usize - self.text.as_ptr() as usize, name.len());
            let value = pair.next().unwrap();
            let value_range = Range::new(value.as_ptr() as usize - self.text.as_ptr() as usize, value.len());
            self.references.insert(line.to_lowercase(), TestHeader { name: name_range, value: value_range });
        }
    }

    pub fn get(&self) -> Vec<(&str, &str)> {
        self.references.values().map(|r| (&self.text[r.name], &self.text[r.value])).collect()
    }
}

// sum like dis?
// if we also added IntoTestEnum trait ...? could work
// also we could replace the pointer with NonNull or range
// enum TestEnum<'a> {
//     Cow(Cow<'a, str>),
//     Pointer(*const str)
// }

// --- fix unsafes (static str or range+str)
// --- maybe replace parse option return with results (more compact)

fn main() {
    // let mut h = Https11::new(Address::new("google.com", Some(443))).unwrap();
    // h = h.send(&mut Request::new()).unwrap();
    // println!("{}", h.response.text());

    let listener = TcpListener::bind("127.0.0.1:443").unwrap();
    let mut socket = listener.accept().unwrap().0;
    let config = rustls::ServerConfig::new(rustls::NoClientAuth::new());
    let mut session = rustls::ServerSession::new(&Arc::new(config));
    let mut stream = rustls::Stream::new(&mut session, &mut socket);
    let mut buffer = [0; 2000];
    stream.read_exact(&mut buffer).unwrap();
    println!("{}", String::from_utf8_lossy(&buffer));
    // let h = Https11::listen(Address::new("127.0.0.1", Some(443)), &test2);
    // h.unwrap().join().unwrap();
}

fn test1(request: Request) -> Option<Response> {
    println!("{}", request.text());
    None
}

fn test2<'a>(request: &'a Request) -> Option<Response<'a>> {
    println!("{}", request.text());
    None
}