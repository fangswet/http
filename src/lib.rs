#![feature(str_split_as_str)]
#![allow(unused_variables)]
#![allow(unused_imports)]
#![allow(dead_code)]

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}

// properly structure (request, response etc outside)

pub mod def;
pub mod message;
pub mod http;

// idea: somehow preserve whole messages to store string in Response, Request as &str
// todo: non-blocking & blocking headers, message, response, request (try to make them drop-in replacements)
// todo: polling loop server, async server, threading server