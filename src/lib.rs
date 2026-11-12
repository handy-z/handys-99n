//! Everything but the entry point, so it can be unit-tested.
#![cfg(windows)]

pub mod app {
    pub mod config;
    pub mod dashboard;
    pub mod offset_keys;
    pub mod view;
}
pub mod game {
    pub mod fishing;
}
pub mod r#macro {
    pub mod bot;
    pub mod engine;
}
