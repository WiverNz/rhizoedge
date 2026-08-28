//! Row-based declared-capability lookup for M5 binding validation.
/// Answers from relational rows, without parsing the status JSON.
pub async fn has_capability(
    db: &rhizo_storage::EdgeDb,
    device: &str,
    class: &str,
    id: &str,
) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM device_capabilities WHERE device_id=? AND class=? AND capability_id=?")
        .bind(device).bind(class).bind(id).fetch_one(db.pool()).await? != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn no_rows_means_no_assumed_capability() {
        let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        assert!(
            !has_capability(&db, "unknown", "actuator", "pump-0")
                .await
                .unwrap()
        );
    }
}
