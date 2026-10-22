use crate::{PersistenceError, database_error};
use rusqlite::{Connection, params};

pub(super) fn insert_historical_account(
    connection: &Connection,
    identifier: &str,
    display_name: &str,
    occurred_at: i64,
) -> Result<(), PersistenceError> {
    connection.execute(
        "INSERT INTO accounts(account_id,display_name,account_status,created_at,updated_at) VALUES (?1,?2,'active',?3,?3)",
        params![identifier, display_name, occurred_at],
    ).map_err(database_error("insert historical account fixture"))?;
    Ok(())
}
