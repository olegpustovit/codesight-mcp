//! Bench fixture: a small but realistic Rust module.

use std::collections::HashMap;

pub trait Greeter {
    fn greet(&self, name: &str) -> String;
}

pub struct EnglishGreeter;

impl Greeter for EnglishGreeter {
    fn greet(&self, name: &str) -> String {
        format!("Hello, {name}!")
    }
}

pub struct SpanishGreeter;

impl Greeter for SpanishGreeter {
    fn greet(&self, name: &str) -> String {
        format!("¡Hola, {name}!")
    }
}

pub fn pick_greeter(language: &str) -> Box<dyn Greeter> {
    match language {
        "es" => Box::new(SpanishGreeter),
        _ => Box::new(EnglishGreeter),
    }
}

#[derive(Default)]
pub struct Counter {
    inner: HashMap<String, u64>,
}

impl Counter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bump(&mut self, key: &str) -> u64 {
        let entry = self.inner.entry(key.to_string()).or_insert(0);
        *entry += 1;
        *entry
    }

    pub fn total(&self) -> u64 {
        self.inner.values().sum()
    }
}

pub mod nested {
    pub fn double(x: i64) -> i64 {
        x * 2
    }

    pub struct Wrapper<T> {
        pub inner: T,
    }

    impl<T: Clone> Wrapper<T> {
        pub fn cloned(&self) -> T {
            self.inner.clone()
        }
    }
}
