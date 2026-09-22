//! MCP tool handlers, split by concern; routers combine in server.rs.

pub mod context;
pub mod recall;
pub mod search;
pub mod symbols;
pub mod ticket_context;
pub(super) mod ticket_related;
#[cfg(test)]
mod ticket_related_tests;
