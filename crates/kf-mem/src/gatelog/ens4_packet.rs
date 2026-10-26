//! Diagnostic frontiers; packet-specific guest delivery remains external evidence.
#[derive(Debug, Default)]
pub struct Packet;
impl Packet {
    pub fn published(&mut self) {}
    pub fn marker(&mut self, _seq: u64) {}
    pub fn seen(&mut self, _seq: u64) {}
    pub fn class(&self, _unpublished_held: bool) -> &'static str { "unknown" }
}
/// Cheap sequence invariant monitor, no clock or allocation.
#[derive(Debug, Default)]
pub struct Watch;
impl Watch {
    pub fn check(&mut self, _issued: u64, _seen: u64) -> u8 { 0 }
}
