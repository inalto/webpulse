//! Domain databases: schema, migrations, dimension names and the store itself.

pub mod dims;
pub mod domain_store;
pub mod migrate;
pub mod schema;

pub use domain_store::DomainStore;
