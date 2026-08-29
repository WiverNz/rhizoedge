//! Plant-profile templates (M5-001).
//!
//! Profiles are **reusable across plants** and are stored as an opaque JSON
//! document plus an index name: the numbers a profile carries belong to
//! `rhizo-domain`, and this crate holds transactions, not decisions. Validation
//! happens in the domain before a document reaches here.
//!
//! # Deleting a profile in use is refused
//!
//! Not cascaded, not nullified, not soft-deleted. A profile in use is a template
//! twelve plants were seeded from and might be seeded from again, and the
//! operator asking to delete it has almost certainly forgotten one of them. The
//! refusal names how many.
#![allow(missing_docs)]
use sqlx::Row as _;

use crate::{EdgeDb, StorageError};

/// A profile as stored.
#[derive(Clone, Debug, PartialEq)]
pub struct ProfileRow {
    pub profile_id: String,
    pub name: String,
    /// The domain document, verbatim.
    pub profile_json: String,
    pub updated_at: i64,
}

/// What a delete did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeleteOutcome {
    /// The profile was removed.
    Deleted,
    /// Plants still reference it, and it was left alone.
    InUse {
        /// How many live plants reference it.
        plants: i64,
    },
    /// No such profile.
    NotFound,
}

fn to_row(row: &sqlx::sqlite::SqliteRow) -> ProfileRow {
    ProfileRow {
        profile_id: row.get("profile_id"),
        name: row.get("name"),
        profile_json: row.get("profile_json"),
        updated_at: row.get("updated_at"),
    }
}

/// Inserts or replaces a profile document.
pub async fn upsert(
    db: &EdgeDb,
    profile_id: &str,
    name: &str,
    profile_json: &str,
    now: i64,
) -> Result<ProfileRow, StorageError> {
    sqlx::query(
        "INSERT INTO plant_profiles(profile_id,name,profile_json,updated_at) VALUES(?,?,?,?) \
         ON CONFLICT(profile_id) DO UPDATE SET name=excluded.name,profile_json=excluded.profile_json,updated_at=excluded.updated_at",
    )
    .bind(profile_id)
    .bind(name)
    .bind(profile_json)
    .bind(now)
    .execute(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    get(db, profile_id)
        .await?
        .ok_or_else(|| StorageError::Database("the profile vanished between write and read".into()))
}

/// Inserts a profile only if the id is free.
///
/// Returns `false` when one already exists, so `POST` and `PUT` can differ.
pub async fn insert_new(
    db: &EdgeDb,
    profile_id: &str,
    name: &str,
    profile_json: &str,
    now: i64,
) -> Result<bool, StorageError> {
    let done = sqlx::query(
        "INSERT INTO plant_profiles(profile_id,name,profile_json,updated_at) VALUES(?,?,?,?) \
         ON CONFLICT(profile_id) DO NOTHING",
    )
    .bind(profile_id)
    .bind(name)
    .bind(profile_json)
    .bind(now)
    .execute(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(done.rows_affected() == 1)
}

pub async fn get(db: &EdgeDb, profile_id: &str) -> Result<Option<ProfileRow>, StorageError> {
    Ok(
        sqlx::query("SELECT * FROM plant_profiles WHERE profile_id=?")
            .bind(profile_id)
            .fetch_optional(db.pool())
            .await
            .map_err(StorageError::from_sqlx)?
            .as_ref()
            .map(to_row),
    )
}

/// Lists profiles after `cursor`, in id order.
pub async fn list(
    db: &EdgeDb,
    cursor: Option<&str>,
    limit: i64,
) -> Result<Vec<ProfileRow>, StorageError> {
    let rows = sqlx::query(
        "SELECT * FROM plant_profiles WHERE profile_id > ? ORDER BY profile_id LIMIT ?",
    )
    .bind(cursor.unwrap_or(""))
    .bind(limit.clamp(1, 500))
    .fetch_all(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(rows.iter().map(to_row).collect())
}

/// Deletes a profile, refusing while any live plant still uses it.
pub async fn delete(db: &EdgeDb, profile_id: &str) -> Result<DeleteOutcome, StorageError> {
    if get(db, profile_id).await?.is_none() {
        return Ok(DeleteOutcome::NotFound);
    }
    let plants = super::plant::count_using_profile(db, profile_id).await?;
    if plants > 0 {
        return Ok(DeleteOutcome::InUse { plants });
    }
    sqlx::query("DELETE FROM plant_profiles WHERE profile_id=?")
        .bind(profile_id)
        .execute(db.pool())
        .await
        .map_err(StorageError::from_sqlx)?;
    Ok(DeleteOutcome::Deleted)
}

/// How many profiles exist.
pub async fn count(db: &EdgeDb) -> Result<i64, StorageError> {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM plant_profiles")
        .fetch_one(db.pool())
        .await
        .map_err(StorageError::from_sqlx)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> EdgeDb {
        let db = EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        db
    }

    #[tokio::test]
    async fn profiles_round_trip_and_are_reusable_across_plants() {
        let db = db().await;
        assert_eq!(count(&db).await.unwrap(), 0);
        let created = upsert(
            &db,
            "monstera_default",
            "Monstera",
            r#"{"dose_ml":40}"#,
            1_000,
        )
        .await
        .unwrap();
        assert_eq!(created.name, "Monstera");
        assert_eq!(created.profile_json, r#"{"dose_ml":40}"#);
        assert_eq!(get(&db, "monstera_default").await.unwrap(), Some(created));
        assert_eq!(get(&db, "nope").await.unwrap(), None);

        upsert(&db, "fern_default", "Fern", "{}", 1_000)
            .await
            .unwrap();
        let all = list(&db, None, 50).await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].profile_id, "fern_default");
        let page = list(&db, Some("fern_default"), 50).await.unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].profile_id, "monstera_default");

        let updated = upsert(&db, "monstera_default", "Monstera v2", "{}", 2_000)
            .await
            .unwrap();
        assert_eq!(updated.name, "Monstera v2");
        assert_eq!(updated.updated_at, 2_000);
        assert_eq!(count(&db).await.unwrap(), 2);

        // One profile serves several plants; that is what makes it a template.
        for id in ["monstera-01", "monstera-02"] {
            super::super::plant::create(
                &db,
                &super::super::plant::NewPlant {
                    plant_id: id.to_owned(),
                    name: id.to_owned(),
                    profile_id: Some("monstera_default".to_owned()),
                    ..Default::default()
                },
                1_000,
            )
            .await
            .unwrap();
        }
        assert_eq!(
            super::super::plant::count_using_profile(&db, "monstera_default")
                .await
                .unwrap(),
            2
        );
    }

    #[tokio::test]
    async fn insert_new_refuses_an_id_that_already_exists() {
        let db = db().await;
        assert!(insert_new(&db, "p", "P", "{}", 1).await.unwrap());
        assert!(!insert_new(&db, "p", "Other", "{}", 2).await.unwrap());
        assert_eq!(get(&db, "p").await.unwrap().unwrap().name, "P");
    }

    #[tokio::test]
    async fn deleting_a_profile_in_use_is_refused() {
        let db = db().await;
        upsert(&db, "monstera_default", "Monstera", "{}", 1_000)
            .await
            .unwrap();
        super::super::plant::create(
            &db,
            &super::super::plant::NewPlant {
                plant_id: "monstera-01".to_owned(),
                name: "Monstera".to_owned(),
                profile_id: Some("monstera_default".to_owned()),
                ..Default::default()
            },
            1_000,
        )
        .await
        .unwrap();
        assert_eq!(
            delete(&db, "monstera_default").await.unwrap(),
            DeleteOutcome::InUse { plants: 1 }
        );
        assert!(get(&db, "monstera_default").await.unwrap().is_some());

        // Removing the plant does *not* release the template: the row still
        // exists, still records which template it was seeded from, and the
        // foreign key still holds. Reporting it as free and then failing the
        // delete would be worse than refusing it plainly.
        super::super::plant::delete(&db, "monstera-01", 2_000)
            .await
            .unwrap();
        assert_eq!(
            delete(&db, "monstera_default").await.unwrap(),
            DeleteOutcome::InUse { plants: 1 }
        );

        // An unreferenced profile deletes cleanly.
        upsert(&db, "unused", "Unused", "{}", 1_000).await.unwrap();
        assert_eq!(delete(&db, "unused").await.unwrap(), DeleteOutcome::Deleted);
        assert_eq!(get(&db, "unused").await.unwrap(), None);
        assert_eq!(
            delete(&db, "unused").await.unwrap(),
            DeleteOutcome::NotFound
        );
    }
}
