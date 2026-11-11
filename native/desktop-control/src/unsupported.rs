//! macOS (AX) and Linux (AT-SPI) are planned; until then every call reports that clearly.

use anyhow::{Result, bail};
use serde_json::Value;

use crate::{Output, Settings};

const UNSUPPORTED: &str = "Desktop control supports Windows only for now.";

pub struct Desktop;

impl Desktop {
    pub fn new(_settings: Settings) -> Result<Self> {
        bail!(UNSUPPORTED)
    }

    pub fn call(&mut self, _command: &str, _args: &Value) -> Result<Output> {
        bail!(UNSUPPORTED)
    }

    pub fn describe(&self, command: &str, _args: &Value) -> String {
        command.to_string()
    }

    pub fn app(&self, _command: &str, _args: &Value) -> Option<String> {
        None
    }
}

pub fn start_stop_hotkey() {}
