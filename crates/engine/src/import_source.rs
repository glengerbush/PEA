//! Provider-neutral preparation and streaming for local email imports.

use crate::ingest::EmailObj;
use serde_json::Value;

trait Adapter {
    fn display_name(&self) -> &str;
    fn total_bytes(&self) -> u64;
    fn stream(
        &self,
        handle: &mut dyn FnMut(EmailObj),
        on_bytes: &mut dyn FnMut(u64),
    ) -> Result<(), String>;
}

impl Adapter for crate::readers::PreparedMbox {
    fn display_name(&self) -> &str {
        crate::readers::PreparedMbox::display_name(self)
    }

    fn total_bytes(&self) -> u64 {
        crate::readers::PreparedMbox::total_bytes(self)
    }

    fn stream(
        &self,
        handle: &mut dyn FnMut(EmailObj),
        on_bytes: &mut dyn FnMut(u64),
    ) -> Result<(), String> {
        crate::readers::PreparedMbox::stream(self, handle, on_bytes)
    }
}

impl Adapter for crate::eml::PreparedEmlZip {
    fn display_name(&self) -> &str {
        crate::eml::PreparedEmlZip::display_name(self)
    }

    fn total_bytes(&self) -> u64 {
        crate::eml::PreparedEmlZip::total_bytes(self)
    }

    fn stream(
        &self,
        handle: &mut dyn FnMut(EmailObj),
        on_bytes: &mut dyn FnMut(u64),
    ) -> Result<(), String> {
        crate::eml::PreparedEmlZip::stream(self, handle, on_bytes)
    }
}

#[derive(Clone, Copy)]
enum AdapterKind {
    Mbox,
    EmlZip,
}

fn select(provider: &str) -> Result<AdapterKind, String> {
    match provider {
        "mbox_import" => Ok(AdapterKind::Mbox),
        "eml_import" => Ok(AdapterKind::EmlZip),
        other => Err(format!("Unsupported provider: {other}")),
    }
}

pub fn supports(provider: &str) -> bool {
    select(provider).is_ok()
}

/// A validated import whose adapter-specific discovery stays private.
pub struct PreparedImport {
    adapter: Box<dyn Adapter>,
}

impl PreparedImport {
    pub fn display_name(&self) -> &str {
        self.adapter.display_name()
    }

    pub fn total_bytes(&self) -> u64 {
        self.adapter.total_bytes()
    }

    pub fn stream(
        &self,
        handle: &mut dyn FnMut(EmailObj),
        on_bytes: &mut dyn FnMut(u64),
    ) -> Result<(), String> {
        self.adapter.stream(handle, on_bytes)
    }
}

/// Selects and prepares the adapter identified by the stored provider.
pub fn prepare(provider: &str, provider_config: &Value) -> Result<PreparedImport, String> {
    let adapter: Box<dyn Adapter> = match select(provider)? {
        AdapterKind::Mbox => Box::new(crate::readers::prepare(provider_config)?),
        AdapterKind::EmlZip => Box::new(crate::eml::prepare(provider_config)?),
    };
    Ok(PreparedImport { adapter })
}
